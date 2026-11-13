use super::*;

fn response(rows: &str, tail: &str) -> String {
    format!(
        "{{\"baton\":\"cursor-private-identity\",\"base_url\":null}}\n{{\"type\":\"step_begin\",\"step\":0,\"cols\":[{{\"name\":\"label\",\"decltype\":null}}]}}\n{rows}{tail}"
    )
}

async fn reply_server(body: String, chunked: bool) -> Server {
    Server::start(move |r| match r.path.as_str() {
        "/v3" => Reply::Raw(200, vec![]),
        "/v3/cursor" if chunked => {
            Reply::Chunks(body.as_bytes().chunks(1).map(<[u8]>::to_vec).collect())
        }
        "/v3/cursor" => Reply::Raw(200, body.as_bytes().to_vec()),
        "/v3/pipeline" => closed(),
        _ => panic!("unexpected path"),
    })
    .await
}

#[tokio::test]
async fn split_utf8_and_libsql_replication_metadata_preserve_rows() {
    let server = reply_server(response(
        "{\"type\":\"row\",\"row\":[{\"type\":\"text\",\"value\":\"東京 é\"}]}\n",
        "{\"type\":\"step_end\"}\n{\"type\":\"replication_index\",\"replication_index\":9226}\n",
    ), true).await;
    let session = session(&server);
    let mut cursor = session
        .execute_preview(request(), &CancelToken::new())
        .await
        .expect("schema");
    let batch = cursor.next_batch().await.expect("batch").expect("row");
    assert_eq!(batch.column(0).as_string::<i32>().value(0), "東京 é");
    assert!(cursor.next_batch().await.expect("EOF").is_none());
    drop(cursor);
    session.close().await.expect("closed");
}

#[tokio::test]
async fn a_trailing_row_or_error_after_replication_metadata_is_not_eof() {
    for tail in [
        "{\"type\":\"row\",\"row\":[]}\n",
        "{\"type\":\"error\",\"error\":{\"message\":\"failed\",\"code\":\"TEST\"}}\n",
    ] {
        let server = reply_server(response("", &format!("{{\"type\":\"step_end\"}}\n{{\"type\":\"replication_index\",\"replication_index\":null}}\n{tail}")), false).await;
        let session = session(&server);
        assert!(
            session
                .execute_preview(request(), &CancelToken::new())
                .await
                .is_err()
        );
        session.close().await.expect("closed");
    }
}

#[tokio::test]
async fn a_later_storage_class_change_is_explicit_and_releases_the_stream() {
    let mut rows =
        "{\"type\":\"row\",\"row\":[{\"type\":\"integer\",\"value\":\"1\"}]}\n".repeat(200);
    rows.push_str(
        "{\"type\":\"row\",\"row\":[{\"type\":\"text\",\"value\":\"not an integer\"}]}\n",
    );
    let server = reply_server(response(&rows, "{\"type\":\"step_end\"}\n"), false).await;
    let session = session(&server);
    let mut cursor = session
        .execute_preview(request(), &CancelToken::new())
        .await
        .expect("sample");
    assert_eq!(
        cursor
            .next_batch()
            .await
            .expect("batch")
            .expect("rows")
            .num_rows(),
        200
    );
    assert!(cursor.next_batch().await.is_err());
    drop(cursor);
    session.close().await.expect("closed");
    assert!(server.requests().iter().any(|r| r.path == "/v3/pipeline"));
}

#[tokio::test]
async fn oversized_headers_are_refused_without_exposing_their_content() {
    let body = format!("{{\"padding\":\"{}\"}}\n", "private".repeat(160_000));
    let server = reply_server(body, false).await;
    let session = session(&server);
    let result = session
        .execute_preview(request(), &CancelToken::new())
        .await;
    let Err(error) = result else {
        panic!("oversized frame accepted");
    };
    assert!(!error.to_string().contains("private"));
    session.close().await.expect("closed");
}

#[tokio::test]
async fn protocol_support_is_negotiated_once_without_running_sql() {
    for status in [200, 404, 405] {
        let server = Server::start(move |r| {
            assert_eq!(r.path, "/v3");
            Reply::Raw(status, vec![])
        })
        .await;
        let http = Http::new(ConnectSpec::plaintext(&server.base(), TOKEN)).expect("client");
        for _ in 0..2 {
            assert_eq!(
                http.supports_cursor(&DriverId::turso(), &CancelToken::new())
                    .await
                    .expect("negotiated"),
                status == 200
            );
        }
        assert_eq!(server.requests().len(), 1);
    }
}

#[test]
fn routing_is_confined_to_the_configured_origin_or_verified_cloud_nodes() {
    let http = Http::new(ConnectSpec::plaintext(
        "https://database-org.turso.io/",
        TOKEN,
    ))
    .expect("client");
    assert!(http.cursor_base(Some("https://node.turso.io/")).is_ok());
    for url in [
        "http://node.turso.io/",
        "https://turso.io.attacker.invalid/",
        "https://turso.io/",
        "https://node.turso.io:444/",
        "https://name:password@node.turso.io/",
        "https://node.turso.io/?token=x",
    ] {
        assert!(http.cursor_base(Some(url)).is_err());
    }
    let custom =
        Http::new(ConnectSpec::plaintext("https://db.example.test/", TOKEN)).expect("client");
    assert!(custom.cursor_base(Some("https://db.example.test/")).is_ok());
    assert!(
        custom
            .cursor_base(Some("https://other.example.test/"))
            .is_err()
    );
    assert!(custom.cursor_base(Some("https://node.turso.io/")).is_err());
}
