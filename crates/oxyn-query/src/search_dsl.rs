//! Where an Elasticsearch request starts and ends in a console document.
//!
//! A document holds requests the way Kibana's console does: a line that
//! starts with an HTTP method opens a request, and the lines up to the next
//! such line are its body. A JSON string cannot hold a raw line break, so a
//! body line never starts with a bare method word: the cut is exact, not a
//! guess. Lines starting with `#` or `//` are comments.
//!
//! The request under the cursor is the one whose lines hold it; the blank and
//! comment lines after a request belong to it, as in Kibana, and the cursor
//! before the first request names none.

/// The methods that open a request, as the driver accepts them.
const METHODS: [&str; 6] = ["GET", "POST", "PUT", "PATCH", "DELETE", "HEAD"];

/// The request under `cursor_byte`, without the blank and comment lines that
/// trail it; `None` before the first request or past the text.
#[must_use]
pub(crate) fn current_request(text: &str, cursor_byte: usize) -> Option<&str> {
    if cursor_byte > text.len() {
        return None;
    }
    // The last request line at or before the cursor's line.
    let mut start = None;
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        if offset > cursor_byte {
            break;
        }
        if opens_request(line) {
            start = Some(offset);
        }
        offset += line.len();
    }
    let start = start?;
    let rest = text.get(start..)?;
    // The request runs to the next request line, or the end of the text.
    let mut length = 0;
    for (index, line) in rest.split_inclusive('\n').enumerate() {
        if index > 0 && opens_request(line) {
            break;
        }
        length += line.len();
    }
    let request = rest.get(..length)?;
    Some(trim_trailing(request))
}

/// Whether a line opens a request: a method, then a path.
fn opens_request(line: &str) -> bool {
    let mut words = line.split_whitespace();
    let (Some(method), Some(_path)) = (words.next(), words.next()) else {
        return false;
    };
    METHODS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(method))
}

/// The request without its trailing blank and comment lines.
fn trim_trailing(request: &str) -> &str {
    let mut end = request.len();
    for line in request.split_inclusive('\n').rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("//") {
            end -= line.len();
        } else {
            break;
        }
    }
    request.get(..end).unwrap_or(request).trim_end()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DOCUMENT: &str = "# the cluster\nGET /_cluster/health\n\nPOST /logs/_search\n{\n  \"query\": {\"match\": {\"msg\": \"GET /not-a-request\"}}\n}\n\n// then\nget _cat/indices?v\n";

    fn at(needle: &str) -> usize {
        DOCUMENT.find(needle).expect("in the document")
    }

    #[test]
    fn the_request_under_the_cursor_is_cut_at_the_next_request_line() {
        assert_eq!(
            current_request(DOCUMENT, at("health")),
            Some("GET /_cluster/health")
        );
        let search =
            "POST /logs/_search\n{\n  \"query\": {\"match\": {\"msg\": \"GET /not-a-request\"}}\n}";
        assert_eq!(current_request(DOCUMENT, at("POST")), Some(search));
        assert_eq!(current_request(DOCUMENT, at("\"query\"")), Some(search));
        assert_eq!(current_request(DOCUMENT, at("not-a-request")), Some(search));
        assert_eq!(
            current_request(DOCUMENT, at("_cat")),
            Some("get _cat/indices?v")
        );
    }

    #[test]
    fn trailing_blank_and_comment_lines_belong_to_the_request_above() {
        let search = current_request(DOCUMENT, at("// then")).expect("the search");
        assert!(search.starts_with("POST /logs/_search"));
        assert!(!search.contains("// then"));
    }

    #[test]
    fn before_the_first_request_or_past_the_text_there_is_none() {
        assert_eq!(current_request(DOCUMENT, at("the cluster")), None);
        assert_eq!(current_request(DOCUMENT, DOCUMENT.len() + 1), None);
        assert_eq!(current_request("", 0), None);
        assert_eq!(current_request("{\"no\": \"request\"}", 2), None);
    }

    #[test]
    fn a_lone_method_word_is_not_a_request_line() {
        assert_eq!(current_request("GET\n", 1), None);
    }
}
