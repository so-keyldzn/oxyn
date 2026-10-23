//! Captures against a real server: what they capture, and that ending one
//! ends it on the server. `#[ignore]`, as the rest of the server suite.

use std::time::Duration;

use oxyn_core::{CancelToken, ExecLimits, ExecRequest, OxynError, QueryLanguage};
use oxyn_driver::Session;

use crate::integration::{clean, column, open_session, run};

fn capture(text: &str, timeout: Duration) -> ExecRequest {
    ExecRequest::new(QueryLanguage::RedisCommand, text)
        .with_limits(ExecLimits::default().writable().with_timeout(timeout))
}

async fn drained(
    session: &dyn Session,
    request: ExecRequest,
) -> Result<Vec<arrow::record_batch::RecordBatch>, OxynError> {
    let mut cursor = session.execute(request, &CancelToken::new()).await?;
    let mut batches = Vec::new();
    while let Some(batch) = cursor.next_batch().await? {
        batches.push(batch);
    }
    Ok(batches)
}

/// `PUBSUB NUMSUB channel`'s count, from another session.
async fn subscribers(observer: &dyn Session, channel: &str) -> Option<String> {
    let reply = run(observer, &format!("PUBSUB NUMSUB {channel}"))
        .await
        .expect("PUBSUB NUMSUB");
    column(&reply, "value").into_iter().nth(1).flatten()
}

/// The clients `CLIENT LIST` shows as monitors (`flags=O`).
async fn monitors(observer: &dyn Session) -> usize {
    let listing = run(observer, "CLIENT LIST").await.expect("CLIENT LIST");
    column(&listing, "value")
        .into_iter()
        .flatten()
        .collect::<String>()
        .lines()
        .filter(|line| {
            line.split(' ').any(|field| {
                field
                    .strip_prefix("flags=")
                    .is_some_and(|f| f.contains('O'))
            })
        })
        .count()
}

#[tokio::test]
#[ignore = "needs OXYN_REDIS_TEST_URL"]
async fn a_subscription_captures_what_is_published_and_ends_on_the_server() {
    let Some(session) = clean().await else { return };
    let Some(publisher) = open_session().await else {
        return;
    };
    let capturing = drained(
        &session,
        capture("SUBSCRIBE oxyn-it:news", Duration::from_secs(4)),
    );
    let publishing = async {
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            subscribers(&publisher, "oxyn-it:news").await.as_deref(),
            Some("1"),
            "the capture is subscribed while it runs"
        );
        for message in ["first", "second"] {
            run(&publisher, &format!("PUBLISH oxyn-it:news {message}"))
                .await
                .expect("PUBLISH");
        }
    };
    let (captured, ()) = futures::future::join(capturing, publishing).await;
    let captured = captured.expect("a capture ends as a result");
    assert_eq!(
        column(&captured, "kind"),
        [
            Some("subscribe".to_owned()),
            Some("message".to_owned()),
            Some("message".to_owned())
        ]
    );
    assert_eq!(
        column(&captured, "value"),
        [
            Some("1".to_owned()),
            Some("first".to_owned()),
            Some("second".to_owned())
        ]
    );
    let mut ended = false;
    for _ in 0..50 {
        if subscribers(&publisher, "oxyn-it:news").await.as_deref() == Some("0") {
            ended = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        ended,
        "the server no longer counts the capture's subscription"
    );
    // The console's own connection was never subscribed.
    assert!(run(&session, "PING").await.is_ok());
}

#[tokio::test]
#[ignore = "needs OXYN_REDIS_TEST_URL"]
async fn monitor_captures_other_commands_and_ends_on_the_server() {
    let Some(session) = clean().await else { return };
    let Some(observer) = open_session().await else {
        return;
    };
    let before = monitors(&observer).await;
    let capturing = drained(&session, capture("MONITOR", Duration::from_secs(3)));
    let acting = async {
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert_eq!(
            monitors(&observer).await,
            before + 1,
            "the capture monitors"
        );
        run(&observer, "ECHO oxyn-it-monitored")
            .await
            .expect("ECHO");
    };
    let (captured, ()) = futures::future::join(capturing, acting).await;
    let captured = captured.expect("a capture ends as a result");
    let commands = column(&captured, "command");
    assert!(
        commands
            .iter()
            .flatten()
            .any(|command| command == "\"ECHO\" \"oxyn-it-monitored\""),
        "{commands:?}"
    );
    assert!(
        column(&captured, "time").iter().all(Option::is_some),
        "every line carries the server's time"
    );
    let mut ended = false;
    for _ in 0..50 {
        if monitors(&observer).await == before {
            ended = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ended, "the server no longer lists the capture as a monitor");
}
