//! The download against a local server that misbehaves on purpose.
//!
//! What is proven: a file is kept only with its pinned size and checksum, a
//! refused or cancelled file leaves no `.part` behind, the second source is
//! tried when the first fails, and plain HTTP is refused outside the tests.

use std::sync::Arc;
use std::time::Duration;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::JoinHandle;

use super::*;

const CONTENT: &[u8] = b"the pinned bytes of a small test file";

/// What the server answers on one path.
#[derive(Clone)]
struct Route {
    path: &'static str,
    status: u16,
    body: Vec<u8>,
    /// Sends `Content-Length` when true; otherwise the body ends at close.
    announce_length: bool,
    /// Sends only this many bytes of the body, then never closes.
    hang_after: Option<usize>,
}

impl Route {
    fn ok(path: &'static str, body: &[u8]) -> Self {
        Self {
            path,
            status: 200,
            body: body.to_vec(),
            announce_length: true,
            hang_after: None,
        }
    }
}

struct Server {
    base: String,
    task: JoinHandle<()>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(routes: Vec<Route>) -> std::io::Result<Server> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let base = format!("http://{}", listener.local_addr()?);
    let routes = Arc::new(routes);
    let task = tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            // One connection at a time: the client sends one request per
            // source, in order.
            let _ignored = answer(socket, &routes).await;
        }
    });
    Ok(Server { base, task })
}

async fn answer(mut socket: TcpStream, routes: &[Route]) -> std::io::Result<()> {
    let mut request = Vec::new();
    let mut buffer = [0u8; 1024];
    while !request.windows(4).any(|w| w == b"\r\n\r\n") && request.len() < 16 * 1024 {
        let read = socket.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        request.extend_from_slice(buffer.get(..read).unwrap_or(&[]));
    }
    let line = String::from_utf8_lossy(&request);
    let path = line.split_whitespace().nth(1).unwrap_or("/");
    let route = routes.iter().find(|r| r.path == path);
    let Some(route) = route else {
        socket
            .write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await?;
        return Ok(());
    };
    let mut head = format!("HTTP/1.1 {} X\r\nconnection: close\r\n", route.status);
    if route.announce_length {
        head.push_str(&format!("content-length: {}\r\n", route.body.len()));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    match route.hang_after {
        Some(n) => {
            socket.write_all(route.body.get(..n).unwrap_or(&[])).await?;
            socket.flush().await?;
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
        None => socket.write_all(&route.body).await?,
    }
    socket.shutdown().await
}

fn pinned(content: &[u8]) -> PinnedFile {
    PinnedFile {
        name: "file.bin",
        size: u64::try_from(content.len()).unwrap_or(u64::MAX),
        sha256: Box::leak(hex(&Sha256::digest(content)).into_boxed_str()),
    }
}

fn fetcher(scheme: Scheme, urls: Vec<String>) -> Result<Fetcher, EmbedError> {
    Ok(Fetcher {
        client: client(scheme)?,
        sources: Box::new(move |_| urls.clone()),
    })
}

struct Outcome {
    result: Result<(), EmbedError>,
    reports: Vec<u64>,
    root: tempfile::TempDir,
    store: ModelStore,
}

impl Outcome {
    fn final_file(&self) -> Option<Vec<u8>> {
        std::fs::read(self.store.dir().join("file.bin")).ok()
    }

    fn part_exists(&self) -> bool {
        self.store.dir().join("file.bin.part").exists()
    }

    fn detail(&self) -> String {
        match &self.result {
            Err(EmbedError::Download { detail, .. }) => detail.clone(),
            other => format!("not a download error: {other:?}"),
        }
    }
}

async fn fetch(
    scheme: Scheme,
    urls: Vec<String>,
    file: PinnedFile,
    cancel_after_first_report: bool,
) -> Result<Outcome, EmbedError> {
    let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
    let store = ModelStore::new(root.path());
    std::fs::create_dir_all(store.dir()).map_err(|e| EmbedError::io("create", store.dir(), e))?;
    let fetcher = fetcher(scheme, urls)?;
    let cancel = CancelToken::new();
    let mut reports = Vec::new();
    let mut report = |received: u64| {
        reports.push(received);
        if cancel_after_first_report && received > 0 {
            cancel.cancel();
        }
    };
    let result = tokio::time::timeout(
        Duration::from_secs(20),
        fetcher.fetch(&store, &file, &mut report, &cancel),
    )
    .await
    .map_err(|_| EmbedError::Inference("the fetch did not end".to_owned()))?;
    Ok(Outcome {
        result,
        reports,
        root,
        store,
    })
}

fn url(server: &Server, path: &str) -> String {
    format!("{}{path}", server.base)
}

#[tokio::test]
async fn the_pinned_file_is_kept_and_progress_reaches_its_size() -> Result<(), EmbedError> {
    let server = serve(vec![Route::ok("/a", CONTENT)])
        .await
        .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(out.final_file().as_deref(), Some(CONTENT));
    assert!(!out.part_exists());
    assert_eq!(out.reports.last().copied(), Some(pinned(CONTENT).size));
    drop(out.root);
    Ok(())
}

#[tokio::test]
async fn a_wrong_checksum_is_refused_on_every_source_and_leaves_nothing() -> Result<(), EmbedError>
{
    // Same length, different bytes: only the checksum tells them apart.
    let mut forged = CONTENT.to_vec();
    if let Some(last) = forged.last_mut() {
        *last ^= 0x01;
    }
    let server = serve(vec![Route::ok("/a", &forged), Route::ok("/b", &forged)])
        .await
        .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a"), url(&server, "/b")],
        pinned(CONTENT),
        false,
    )
    .await?;
    let detail = out.detail();
    assert_eq!(detail.matches("sha256").count(), 2, "{detail}");
    assert!(out.final_file().is_none());
    assert!(!out.part_exists());
    Ok(())
}

#[tokio::test]
async fn the_second_source_is_tried_when_the_first_fails() -> Result<(), EmbedError> {
    let server = serve(vec![Route::ok("/mirror", CONTENT)])
        .await
        .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/missing"), url(&server, "/mirror")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(out.result.is_ok(), "{:?}", out.result);
    assert_eq!(out.final_file().as_deref(), Some(CONTENT));
    Ok(())
}

#[tokio::test]
async fn an_announced_length_other_than_the_pinned_size_is_refused_before_reading()
-> Result<(), EmbedError> {
    let mut longer = CONTENT.to_vec();
    longer.push(b'!');
    let server = serve(vec![Route::ok("/a", &longer)])
        .await
        .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(out.detail().contains("announces"), "{}", out.detail());
    assert!(!out.part_exists());
    Ok(())
}

#[tokio::test]
async fn a_body_larger_than_pinned_is_cut_at_the_size() -> Result<(), EmbedError> {
    let endless = vec![b'x'; 64 * 1024];
    let server = serve(vec![Route {
        announce_length: false,
        ..Route::ok("/a", &endless)
    }])
    .await
    .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(out.detail().contains("more than"), "{}", out.detail());
    assert!(out.final_file().is_none());
    assert!(!out.part_exists());
    Ok(())
}

#[tokio::test]
async fn a_truncated_body_is_refused() -> Result<(), EmbedError> {
    let short = CONTENT.get(..10).unwrap_or(&[]);
    let server = serve(vec![Route {
        announce_length: false,
        ..Route::ok("/a", short)
    }])
    .await
    .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(out.detail().contains("sent 10 bytes"), "{}", out.detail());
    assert!(!out.part_exists());
    Ok(())
}

#[tokio::test]
async fn cancelling_mid_body_stops_at_once_and_removes_the_part() -> Result<(), EmbedError> {
    let server = serve(vec![Route {
        hang_after: Some(8),
        ..Route::ok("/a", CONTENT)
    }])
    .await
    .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::PlainAllowed,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        true,
    )
    .await?;
    assert!(
        matches!(out.result, Err(EmbedError::Cancelled)),
        "{:?}",
        out.result
    );
    assert!(out.final_file().is_none());
    assert!(!out.part_exists());
    Ok(())
}

#[tokio::test]
async fn plain_http_is_refused_by_the_real_client() -> Result<(), EmbedError> {
    let server = serve(vec![Route::ok("/a", CONTENT)])
        .await
        .map_err(|e| EmbedError::io("bind", "", e))?;
    let out = fetch(
        Scheme::HttpsOnly,
        vec![url(&server, "/a")],
        pinned(CONTENT),
        false,
    )
    .await?;
    assert!(
        matches!(out.result, Err(EmbedError::Download { .. })),
        "{:?}",
        out.result
    );
    assert!(out.final_file().is_none());
    Ok(())
}

/// A second process — modeled by a second open of the lock file, which
/// `flock` treats the same way — makes a download refuse at once, before it
/// touches anything; once released, the download goes ahead.
#[tokio::test]
async fn a_download_refuses_while_another_process_holds_the_directory() -> Result<(), EmbedError> {
    let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
    let store = ModelStore::new(root.path());
    // Another process: a store of its own — its own mutex — on the same root.
    let other = ModelStore::new(root.path()).lock_exclusive()?;

    let nowhere = fetcher(Scheme::PlainAllowed, Vec::new())?;
    let refused = store
        .download_with(&nowhere, |_| {}, &CancelToken::new(), convert_here)
        .await;
    assert!(
        matches!(refused, Err(EmbedError::DownloadInProgress)),
        "{refused:?}"
    );

    drop(other);
    let went_ahead = store
        .download_with(&nowhere, |_| {}, &CancelToken::new(), convert_here)
        .await;
    // No source to fetch from: it fails further on, not on the lock.
    assert!(
        matches!(went_ahead, Err(EmbedError::Download { .. })),
        "{went_ahead:?}"
    );
    Ok(())
}

/// The scenario that a lock file inside the directory lost: a removal while
/// another process downloads. It is refused and the download's files stay;
/// once the download ends, the removal goes ahead and the lock file — beside
/// the directory — survives it, so the next process locks the same file.
#[tokio::test]
async fn a_removal_waits_for_the_download_of_another_process() -> Result<(), EmbedError> {
    let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
    let downloading = ModelStore::new(root.path());
    let removing = ModelStore::new(root.path());
    std::fs::create_dir_all(downloading.dir())
        .map_err(|e| EmbedError::io("create", downloading.dir(), e))?;
    let held = downloading.lock_exclusive()?;
    let part = downloading.part_path(&SAFETENSORS);
    std::fs::write(&part, b"in flight").map_err(|e| EmbedError::io("write", &part, e))?;

    let refused = removing.remove();
    assert!(
        matches!(refused, Err(EmbedError::DownloadInProgress)),
        "{refused:?}"
    );
    assert!(
        part.exists(),
        "the download's file must survive a refused removal"
    );
    // Nor can a third process start a second download beside the first.
    assert!(matches!(
        ModelStore::new(root.path()).lock_exclusive(),
        Err(EmbedError::DownloadInProgress)
    ));

    drop(held);
    removing.remove()?;
    assert!(!downloading.dir().exists());
    assert!(removing.lock_path().exists());
    assert!(!removing.lock_path().starts_with(removing.dir()));
    Ok(())
}

/// A safetensors left by an interrupted download is checked again right
/// before the conversion parses it; a damaged one is removed, not parsed.
#[tokio::test]
async fn a_damaged_safetensors_is_never_converted() -> Result<(), EmbedError> {
    let root = tempfile::tempdir().map_err(|e| EmbedError::io("create", "tmp", e))?;
    let store = ModelStore::new(root.path());
    std::fs::create_dir_all(store.dir()).map_err(|e| EmbedError::io("create", store.dir(), e))?;
    let path = store.safetensors_path();
    let file = std::fs::File::create(&path).map_err(|e| EmbedError::io("create", &path, e))?;
    // Sparse: the pinned size, all zeros.
    file.set_len(SAFETENSORS.size)
        .map_err(|e| EmbedError::io("extend", &path, e))?;

    let converted = store.convert_in_place();
    assert!(
        matches!(converted, Err(EmbedError::Conversion(_))),
        "{converted:?}"
    );
    assert!(!path.exists());
    assert!(!store.part_path(&CONVERTED).exists());
    assert!(!store.path(&CONVERTED).exists());
    Ok(())
}

#[test]
fn the_pinned_sources_are_hugging_face_then_the_release() {
    let [first, second] = pinned::sources(&TOKENIZER);
    assert_eq!(
        first,
        "https://huggingface.co/ibm-granite/granite-embedding-97m-multilingual-r2/resolve/\
         835ad14087e140460703cf0fae09f97d469d65c2/tokenizer.json"
    );
    assert!(second.starts_with("https://github.com/so-keyldzn/oxyn/releases/download/"));
    assert!(second.ends_with("/tokenizer.json"));
}

/// The real thing, end to end: TLS, Hugging Face's redirection to its
/// storage, the conversion and its pinned checksum, then a second call that
/// fetches nothing.
///
/// `OXYN_EMBED_DOWNLOAD_ROOT=<empty dir> cargo nextest run -p oxyn-embed
/// --run-ignored only the_pinned_model`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "downloads 220 MB from Hugging Face: set OXYN_EMBED_DOWNLOAD_ROOT"]
async fn the_pinned_model_downloads_converts_and_is_kept() -> Result<(), EmbedError> {
    let Some(root) = std::env::var_os("OXYN_EMBED_DOWNLOAD_ROOT") else {
        panic!("OXYN_EMBED_DOWNLOAD_ROOT is not set");
    };
    let store = ModelStore::new(root);
    let mut steps = Vec::new();
    // Through a supplied step, as `oxyn-desktop` runs it in a child process:
    // the download still holds the directory while the step runs, so a
    // second process could neither convert nor remove beside it.
    let other = ModelStore::new(store.root());
    store
        .download_with_converter(
            |step| steps.push(step),
            &CancelToken::new(),
            |held| async move {
                assert!(matches!(
                    other.lock_exclusive(),
                    Err(EmbedError::DownloadInProgress)
                ));
                convert_here(held).await
            },
        )
        .await?;
    assert_eq!(store.status()?, crate::ModelStatus::Ready);
    assert!(!store.safetensors_path().exists());
    assert_eq!(
        steps.last().map(|s| s.phase),
        Some(DownloadPhase::Converting)
    );

    let mut again = Vec::new();
    store
        .download(|step| again.push(step), &CancelToken::new())
        .await?;
    assert!(
        again.iter().all(|s| s.phase == DownloadPhase::Verifying),
        "{again:?}"
    );
    Ok(())
}
