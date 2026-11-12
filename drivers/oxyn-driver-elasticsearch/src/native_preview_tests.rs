use super::*;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

#[tokio::test]
async fn an_empty_leading_partition_does_not_erase_the_next_cancellation_handle() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback");
    let port = listener.local_addr().expect("address").port();
    let server = tokio::spawn(async move {
        let mut created = 0;
        loop {
            let (mut socket, _) = listener.accept().await.expect("request");
            let mut wire = Vec::new();
            let request = loop {
                let mut bytes = [0_u8; 4096];
                let count = socket.read(&mut bytes).await.expect("request bytes");
                assert!(count > 0);
                wire.extend_from_slice(&bytes[..count]);
                let text = String::from_utf8_lossy(&wire);
                let Some((head, body)) = text.split_once("\r\n\r\n") else {
                    continue;
                };
                let length = head
                    .lines()
                    .find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse::<usize>().expect("length"))
                    })
                    .unwrap_or(0);
                if body.len() >= length {
                    break text.into_owned();
                }
            };
            let body = if request.starts_with("GET /alias/_search_shards ") {
                r#"{"shards":[[{"index":"a","shard":0}],[{"index":"b","shard":0}]]}"#.to_owned()
            } else if request.starts_with("POST /alias/_search/point_in_time?") {
                created += 1;
                format!(r#"{{"pit_id":"synthetic-{created}"}}"#)
            } else if request.starts_with("DELETE ") {
                r#"{"pits":[{"successful":true}]}"#.to_owned()
            } else {
                let (_, body) = request.split_once("\r\n\r\n").expect("body");
                let query: serde_json::Value = serde_json::from_str(body).expect("query");
                let hits = if query.pointer("/query/bool/filter/0/term/_index")
                    == Some(&serde_json::json!("b"))
                {
                    serde_json::json!([{"_index":"b","_id":"one","_source":{"n":1},"sort":[0]}])
                } else {
                    serde_json::json!([])
                };
                serde_json::json!({"timed_out":false,"hits":{"hits":hits}}).to_string()
            };
            socket.write_all(&answer(&body)).await.expect("response");
            socket.shutdown().await.expect("close response");
        }
    });
    let mut session = session_to(port);
    session.catalog = ElasticsearchCatalog::new(
        Arc::clone(&session.http),
        Product {
            name: "OpenSearch",
            version: "2.19.7".into(),
        },
        session.capabilities,
    );
    let cursor = session
        .execute_preview(
            request(
                "POST /alias/_search\n{\"size\":50,\"sort\":[\"_doc\"],\"track_total_hits\":false}",
            ),
            &CancelToken::new(),
        )
        .await
        .expect("skip the empty first partition");
    let token = session
        .registry
        .get(cursor.handle())
        .expect("the live second partition keeps its cancellation registration");
    assert!(!token.is_cancelled());
    session.cancel(cursor.handle()).await.expect("cancel");
    assert!(token.is_cancelled());
    drop(cursor);
    Box::new(session).close().await.expect("cleanup");
    server.abort();
    let _ = server.await;
}
