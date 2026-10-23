//! The deliberately small console grammar and its conservative classifier.

use oxyn_core::StatementIntent;
use serde_json::Value;

/// One Kibana-Dev-Tools-like HTTP request.
///
/// The console accepts exactly one request: `METHOD /path`, followed by an
/// optional JSON body. Keeping the grammar single-request makes the policy
/// decision and cancellation target unambiguous.
#[derive(Debug, Clone, PartialEq)]
pub struct SearchRequest {
    /// The upper-case HTTP method accepted by Elasticsearch and OpenSearch.
    pub method: String,
    /// The user-provided absolute API path, including its query component.
    pub path: String,
    /// The parsed JSON body, if the request has one.
    pub body: Option<Value>,
}

/// Parses the Search DSL console format without rewriting user input.
///
/// The path must be an absolute path, never a full URL. This prevents a
/// console document from redirecting credentials to another origin.
pub fn parse_request(text: &str) -> Result<SearchRequest, String> {
    let (head, body) = match text.split_once('\n') {
        Some((head, body)) => (head.trim_end_matches('\r'), body.trim()),
        None => (text.trim(), ""),
    };
    let mut words = head.split_ascii_whitespace();
    let Some(method) = words.next() else {
        return Err("a Search DSL request starts with an HTTP method and an absolute path".into());
    };
    let Some(path) = words.next() else {
        return Err("a Search DSL request needs an absolute path".into());
    };
    if words.next().is_some() {
        return Err("the request line has exactly a method and a path".into());
    }
    let method = method.to_ascii_uppercase();
    if !matches!(
        method.as_str(),
        "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD"
    ) {
        return Err("the request method is not supported by this console".into());
    }
    if !path.starts_with('/') || path.starts_with("//") || path.contains('#') {
        return Err("the request path must start with one `/` and stay on this server".into());
    }
    let body = if body.is_empty() {
        None
    } else {
        Some(serde_json::from_str(body).map_err(|_| "the request body must be valid JSON")?)
    };
    Ok(SearchRequest {
        method,
        path: path.to_owned(),
        body,
    })
}

/// Classifies an HTTP request conservatively: unknown routes are mutating.
#[must_use]
pub fn statement_intent(request: &SearchRequest) -> StatementIntent {
    let path = request.path.split('?').next().unwrap_or_default();
    if path.starts_with("/_security/") || path.starts_with("/_snapshot/") {
        return StatementIntent::Grant;
    }
    if path.starts_with("/_cluster/settings") || path.contains("/_settings") {
        return StatementIntent::Ddl;
    }
    if matches!(request.method.as_str(), "GET" | "HEAD") {
        return StatementIntent::Read;
    }
    if matches!(
        path,
        "/_search"
            | "/_count"
            | "/_msearch"
            | "/_mget"
            | "/_explain"
            | "/_validate/query"
            | "/_analyze"
            | "/_field_caps"
    ) || path.ends_with("/_search")
        || path.ends_with("/_count")
        || path.ends_with("/_msearch")
        || path.ends_with("/_mget")
        || path.ends_with("/_explain")
        || path.ends_with("/_validate/query")
        || path.ends_with("/_analyze")
        || path.ends_with("/_field_caps")
        || path.ends_with("/_simulate")
    {
        return StatementIntent::Read;
    }
    if path.starts_with("/_bulk")
        || path.contains("/_update")
        || path.contains("/_delete_by_query")
        || path.starts_with("/_reindex")
    {
        return StatementIntent::Write;
    }
    match request.method.as_str() {
        "PUT" | "PATCH" | "DELETE" => StatementIntent::Ddl,
        "POST" => StatementIntent::Unknown,
        _ => StatementIntent::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_one_request_without_rewriting_its_json() {
        let request = parse_request("POST /logs/_search\n{\"query\":{\"match_all\":{}}}")
            .expect("valid request");
        assert_eq!(request.method, "POST");
        assert_eq!(request.path, "/logs/_search");
        assert!(request.body.is_some());
        assert_eq!(statement_intent(&request), StatementIntent::Read);
    }

    #[test]
    fn unknown_post_is_protected_as_mutating() {
        let request = parse_request("POST /_unknown\n{} ").expect("request");
        assert_eq!(statement_intent(&request), StatementIntent::Unknown);
    }

    #[test]
    fn security_routes_are_grants_even_when_the_verb_is_get() {
        let request = parse_request("GET /_security/user").expect("request");
        assert_eq!(statement_intent(&request), StatementIntent::Grant);
    }

    #[test]
    fn refuses_another_origin_and_multiple_request_lines() {
        assert!(parse_request("GET //elsewhere/_search").is_err());
        assert!(parse_request("GET /_search trailing").is_err());
    }
}
