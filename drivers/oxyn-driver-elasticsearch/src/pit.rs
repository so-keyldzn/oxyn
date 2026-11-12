//! Ownership of a preview's PIT, including abandonment before its first hit.
use crate::cursor::Remote;
use crate::http::{Call, Http, Reply};
use crate::session::Registry;
use oxyn_core::{CancelToken, ExecRequest, OxynError, Result};
use reqwest::Method;
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

pub(crate) struct Lease {
    remote: Option<Remote>,
    http: Arc<Http>,
    registry: Arc<Registry>,
}
impl Lease {
    pub fn update(&mut self, reply: &Reply) {
        if let Some(remote) = &mut self.remote {
            remote.update(reply);
        }
    }
    pub fn take(&mut self) -> Option<Remote> {
        self.remote.take()
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        if let Some(remote) = self.remote.take() {
            self.registry.release(Arc::clone(&self.http), remote);
        }
    }
}

/// The first search has no cursor yet. Its opaque task identity must survive
/// an abandoned HTTP future, independently of its PIT lease.
pub(crate) struct PendingSearch {
    http: Arc<Http>,
    registry: Arc<Registry>,
    handle: oxyn_core::StatementHandle,
    opaque: Option<String>,
}
impl PendingSearch {
    pub fn new(
        armed: bool,
        http: Arc<Http>,
        registry: Arc<Registry>,
        handle: oxyn_core::StatementHandle,
        opaque: String,
    ) -> Self {
        Self {
            http,
            registry,
            handle,
            opaque: armed.then_some(opaque),
        }
    }
    pub fn settle(&mut self) {
        self.opaque = None;
    }
}
impl Drop for PendingSearch {
    fn drop(&mut self) {
        if let Some(opaque) = self.opaque.take() {
            self.registry.remove(self.handle);
            self.registry
                .cleanup_request(Arc::clone(&self.http), None, Some(opaque));
        }
    }
}

pub(crate) async fn open(
    http: Arc<Http>,
    registry: Arc<Registry>,
    opensearch: bool,
    request: &mut ExecRequest,
    shard: Option<u32>,
    cancel: &CancelToken,
) -> Result<Lease> {
    let parsed = crate::request::parse_request(&request.text).map_err(OxynError::Query)?;
    let target = parsed
        .path
        .strip_suffix("/_search")
        .filter(|target| target.len() > 1)
        .ok_or_else(|| OxynError::Config("a native preview must name one search target".into()))?;
    let mut path = if opensearch {
        format!("{target}/_search/point_in_time?keep_alive=5m&allow_partial_pit_creation=false")
    } else {
        format!("{target}/_pit?keep_alive=5m&allow_partial_search_results=false")
    };
    if let Some(shard) = shard {
        path.push_str(&format!("&preference=_shards%3A{shard}"));
    }
    let (send, receive) = tokio::sync::oneshot::channel();
    let worker_http = Arc::clone(&http);
    let worker_registry = Arc::clone(&registry);
    // The response remains owned after the caller disappears: if a PIT was
    // created, its returned identity is closed, never simply dropped.
    registry.spawn_owned(async move {
        let answer = create(&worker_http, &path, opensearch)
            .await
            .map(|(id, remote)| {
                (
                    id,
                    Lease {
                        remote: Some(remote),
                        http: worker_http,
                        registry: worker_registry,
                    },
                )
            });
        // An unreceived value in the channel owns its cleanup too.
        let _ = send.send(answer);
    });
    let (id, lease) = tokio::select! {
        biased;
        () = cancel.cancelled() => return Err(OxynError::Cancelled),
        answer = receive => answer.map_err(|_| OxynError::Internal("PIT opener stopped".into()))??,
    };
    let mut body = parsed
        .body
        .ok_or_else(|| OxynError::Config("preview search has no body".into()))?;
    body.as_object_mut()
        .ok_or_else(|| OxynError::Config("preview search body must be an object".into()))?
        .insert("pit".into(), json!({"id": id, "keep_alive": "5m"}));
    request.text = format!("POST /_search\n{body}");
    Ok(lease)
}

async fn create(http: &Http, path: &str, opensearch: bool) -> Result<(String, Remote)> {
    let reply = http
        .send(Call {
            timeout: Some(Duration::from_secs(10)),
            ..Call::read(Method::POST, path)
        })
        .await?;
    #[derive(Deserialize)]
    struct Created {
        id: Option<String>,
        pit_id: Option<String>,
        _shards: Option<Value>,
    }
    let created: Created = serde_json::from_slice(&reply.bytes)
        .map_err(|_| OxynError::Query("the PIT creation response is invalid".into()))?;
    let id = if opensearch {
        created.pit_id
    } else {
        created.id
    }
    .filter(|id| !id.is_empty())
    .ok_or_else(|| OxynError::Query("the server did not return a PIT identity".into()))?;
    let remote = Remote::Pit {
        id: id.clone(),
        opensearch,
    };
    if created
        ._shards
        .as_ref()
        .and_then(|shards| shards.get("failed"))
        .and_then(Value::as_u64)
        .is_some_and(|failed| failed > 0)
    {
        remote.release(http).await;
        return Err(OxynError::Query(
            "the PIT could not include every shard".into(),
        ));
    }
    Ok((id, remote))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    #[tokio::test]
    async fn an_unreceived_pit_delivery_still_owns_its_remote_cleanup() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("loopback");
        let http = Arc::new(crate::http::tests::http_to(
            listener.local_addr().expect("address").port(),
        ));
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("cleanup connection");
            let mut request = vec![0_u8; 4096];
            let count = socket.read(&mut request).await.expect("cleanup request");
            assert!(String::from_utf8_lossy(&request[..count]).starts_with("DELETE /_pit "));
            let body = r#"{"succeeded":true,"num_freed":1}"#;
            socket.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.expect("answer");
        });
        let registry = Arc::new(Registry::default());
        let (send, receive) = tokio::sync::oneshot::channel();
        let lease = Lease {
            remote: Some(Remote::Pit {
                id: "synthetic-pit".into(),
                opensearch: false,
            }),
            http,
            registry,
        };
        assert!(send.send(lease).is_ok());
        drop(receive); // Cancellation wins after delivery but before receipt.
        tokio::time::timeout(Duration::from_secs(2), server)
            .await
            .expect("cleanup is sent")
            .expect("server assertion");
    }
}
