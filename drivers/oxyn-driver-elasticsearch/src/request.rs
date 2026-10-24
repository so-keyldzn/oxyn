//! The console grammar: one Kibana-Dev-Tools-like request.

use serde_json::Value;

/// One HTTP request, as the user wrote it.
///
/// The console accepts exactly one request: `METHOD path`, followed by an
/// optional JSON body. Keeping the grammar single-request makes the policy
/// decision and the cancellation target unambiguous; the editor cuts a
/// document holding several requests before it reaches the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRequest {
    /// The upper-case HTTP method.
    pub method: String,
    /// The absolute API path, including its query component. A path written
    /// without its leading `/` (`GET _search`, as Kibana accepts) gets one.
    pub path: String,
    /// The parsed JSON body, if the request has one.
    pub body: Option<Value>,
}

/// Parses the Search DSL console format without rewriting the body.
///
/// The path stays on the configured server: a full URL, a `//` authority or a
/// fragment is refused, so a console document cannot redirect credentials to
/// another origin.
///
/// # Errors
///
/// A message naming what is malformed, never quoting the request.
pub fn parse_request(text: &str) -> Result<SearchRequest, String> {
    let text = text.trim_start();
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
    let body = if body.is_empty() {
        None
    } else {
        Some(serde_json::from_str(body).map_err(|error| {
            format!(
                "the request body is not valid JSON (line {}, column {})",
                error.line(),
                error.column()
            )
        })?)
    };
    Ok(SearchRequest { method, path, body })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_request_without_rewriting_its_json() {
        let request = parse_request("\n  POST /logs/_search\n{\"query\":{\"match_all\":{}}}")
            .expect("valid request");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/logs/_search");
        assert_eq!(
            request.body,
            Some(serde_json::json!({"query":{"match_all":{}}}))
        );
    }

    #[test]
    fn a_kibana_path_without_its_slash_is_accepted() {
        let request = parse_request("get _cat/indices?v").expect("valid request");
        assert_eq!(request.method, "GET");
        assert_eq!(request.path, "/_cat/indices?v");
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
        assert!(error.contains("line 1"), "{error}");
        assert!(!error.contains("secret"), "{error}");
    }
}
