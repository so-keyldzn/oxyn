//! The console grammar: one Kibana-Dev-Tools-like request.

use serde_json::Value;

/// One HTTP request, as the user wrote it.
///
/// The console accepts exactly one request: `METHOD path`, followed by an
/// optional body. Keeping the grammar single-request makes the policy
/// decision and the cancellation target unambiguous; the editor cuts a
/// document holding several requests before it reaches the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRequest {
    /// The upper-case HTTP method.
    pub method: String,
    /// The absolute API path, including its query component. A path written
    /// without its leading `/` (`GET _search`, as Kibana accepts) gets one.
    pub path: String,
    /// The body, parsed, for classification. `None` for an NDJSON body.
    pub body: Option<Value>,
    /// The body exactly as written: what is sent. Re-serializing the parsed
    /// value would round numbers and reorder nothing useful.
    pub text: Option<String>,
    /// Whether the body is newline-delimited JSON (`_bulk`, `_msearch`).
    pub ndjson: bool,
}

/// Parses the Search DSL console format without rewriting the body.
///
/// The path stays on the configured server: a full URL, a `//` authority or a
/// fragment is refused, so a console document cannot redirect credentials to
/// another origin.
///
/// # Errors
/// A message naming what is malformed, never quoting the request.
pub fn parse_request(text: &str) -> Result<SearchRequest, String> {
    // Whole-line comments, as Kibana's console takes them, are the user's
    // notes: they are not part of what is sent.
    let uncommented = text
        .lines()
        .filter(|line| {
            let line = line.trim_start();
            !line.starts_with('#') && !line.starts_with("//")
        })
        .collect::<Vec<_>>()
        .join("\n");
    let text = uncommented.trim_start();
    let (head, body) = match text.split_once('\n') {
        Some((head, body)) => (head.trim_end_matches('\r'), body.trim()),
        None => (text.trim_end(), ""),
    };
    let mut words = head.split_ascii_whitespace();
    let Some(method) = words.next() else {
        return Err("a Search DSL request starts with an HTTP method and an API path".into());
    };
    let Some(path) = words.next() else {
        return Err("a Search DSL request needs an API path after its method".into());
    };
    if words.next().is_some() {
        return Err("the request line has exactly a method and a path".into());
    }
    let method = method.to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err(
            "the request method is not one of GET, POST, PUT, PATCH, DELETE or HEAD".into(),
        );
    }
    if path.contains("://") || path.starts_with("//") || path.contains('#') {
        return Err("the request path must be an API path on this server, not a URL".into());
    }
    let path = if path.starts_with('/') {
        path.to_owned()
    } else {
        format!("/{path}")
    };
    let ndjson = takes_ndjson(&path);
    if body.is_empty() {
        return Ok(SearchRequest {
            method,
            path,
            body: None,
            text: None,
            ndjson,
        });
    }
    if ndjson {
        for (number, line) in body.lines().enumerate() {
            if !line.trim().is_empty() && serde_json::from_str::<Value>(line).is_err() {
                return Err(format!(
                    "line {} of the body is not one JSON document: `_bulk` and `_msearch` \
                     take one document per line",
                    number + 2
                ));
            }
        }
        // The server refuses a bulk body whose last line has no newline.
        return Ok(SearchRequest {
            method,
            path,
            body: None,
            text: Some(format!("{body}\n")),
            ndjson,
        });
    }
    let parsed = serde_json::from_str(body).map_err(|error| {
        format!(
            "the request body is not valid JSON (line {}, column {})",
            error.line() + 1,
            error.column()
        )
    })?;
    Ok(SearchRequest {
        method,
        path,
        body: Some(parsed),
        text: Some(body.to_owned()),
        ndjson,
    })
}

/// Whether an endpoint takes newline-delimited JSON.
fn takes_ndjson(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or_default();
    path.ends_with("/_bulk")
        || path.ends_with("/_msearch")
        || path.ends_with("/_msearch/template")
        || path == "/_bulk"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_request_without_rewriting_its_body() {
        let body = "{\"query\":{\"term\":{\"n\":12345678901234567890.5}}}";
        let request =
            parse_request(&format!("\n  POST /logs/_search\n{body}")).expect("valid request");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/logs/_search");
        assert_eq!(request.text.as_deref(), Some(body), "sent byte for byte");
        assert!(request.body.is_some());
        assert!(!request.ndjson);
    }

    #[test]
    fn whole_line_comments_are_not_sent() {
        let request = parse_request(
            "# the errors of today\nPOST /logs/_search\n{\n  // only 500s\n  \"query\": {\"term\": {\"status\": 500}}\n}",
        )
        .expect("valid request");
        assert_eq!(request.path, "/logs/_search");
        assert!(
            request
                .text
                .as_deref()
                .is_some_and(|text| !text.contains("only 500s"))
        );
        assert!(request.body.is_some());
    }

    #[test]
    fn a_kibana_path_without_its_slash_is_accepted() {
        let request = parse_request("get _cat/indices?v").expect("valid request");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/_cat/indices?v");
    }

    #[test]
    fn a_bulk_body_is_one_document_per_line_ending_with_a_newline() {
        let request = parse_request(
            "POST /_bulk\n{\"index\":{\"_index\":\"logs\"}}\n{\"a\":1}\n{\"delete\":{\"_index\":\"logs\",\"_id\":\"2\"}}",
        )
        .expect("valid bulk");
        assert!(request.ndjson);
        assert!(
            request
                .text
                .as_deref()
                .is_some_and(|text| text.ends_with("}\n"))
        );
        let refused =
            parse_request("POST /logs/_bulk\n{\"index\":{}}\n{\"a\":").expect_err("bad line");
        assert!(refused.contains("line 3"), "{refused}");
    }

    #[test]
    fn refuses_another_origin_and_extra_words() {
        assert!(parse_request("GET //elsewhere/_search").is_err());
        assert!(parse_request("GET http://elsewhere/_search").is_err());
        assert!(parse_request("GET /_search#fragment").is_err());
        assert!(parse_request("GET /_search trailing").is_err());
        assert!(parse_request("TRACE /").is_err());
        assert!(parse_request("   ").is_err());
    }

    #[test]
    fn a_malformed_body_is_located_without_being_quoted() {
        let error = parse_request("POST /_search\n{\"query\": secret").expect_err("invalid JSON");
        assert!(error.contains("line 2"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }
}
