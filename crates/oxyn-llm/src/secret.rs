//! The API key, and what is done so that it goes out nowhere.
//!
//! A provider key is a secret in the sense of [`I-03`](../../../CLAUDE.md): it
//! has no business in a log, a displayed error, a crash report or a workspace
//! file. The checkable corollary of the invariant is that **no type carrying a
//! secret derives `Debug`** — it is the `tracing::debug!("{provider:?}")`
//! added six months later that leaks.
//!
//! # Why not `secrecy::SecretString`
//!
//! `secrecy` and `zeroize` are not in this crate's dependency contract (see its
//! `Cargo.toml`, which is frozen). [`ApiKey`] is the minimal equivalent:
//! hand-written `Debug`, no `Display`, no `Serialize`, and a `Drop` that
//! overwrites the buffer. This erasure is **best effort**: without `zeroize`,
//! nothing formally prevents the compiler from treating the write as dead. The
//! protection that really matters here is the absence of any display path.

use std::fmt;

/// API key of a model provider.
///
/// Never displayed: `Debug` is masked, `Display` does not exist, and the type
/// is neither `Serialize` nor `Deserialize` — a key is read from the keychain
/// or from the environment, it is not persisted from here.
///
/// Equality is deliberately not implemented: comparing two keys has no
/// legitimate use in this crate, and a naive comparison invites the bad idea
/// of authenticating someone with it.
#[derive(Clone)]
pub struct ApiKey(String);

impl ApiKey {
    /// Adopts a key provided by the caller.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// Reads a key from an environment variable.
    ///
    /// Returns `None` if the variable is missing, empty, or not representable
    /// in UTF-8. It is the path of providers configured outside the interface
    /// (`OPENAI_API_KEY`, `OPENROUTER_API_KEY`…).
    #[must_use]
    pub fn from_env(variable: &str) -> Option<Self> {
        let raw = std::env::var(variable).ok()?;
        let key = Self::new(raw);
        if key.is_blank() { None } else { Some(key) }
    }

    /// Exposes the key, to put it in an HTTP header and nothing else.
    ///
    /// The name is deliberately unpleasant: every call is a place to review.
    /// Never put the result in an error message, a log `format!`, or a URL — a
    /// key as a query parameter ends up in the provider's access logs.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }

    /// Is the key empty or made only of whitespace?
    ///
    /// A blank key is a configuration error, not a missing key: a provider
    /// that receives it will answer `401` rather than be clear.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.0.trim().is_empty()
    }

    /// Length in bytes, the only information we agree to disclose.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Equivalent of [`is_blank`](Self::is_blank) in the strict sense of length.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<String> for ApiKey {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl From<&str> for ApiKey {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl fmt::Debug for ApiKey {
    /// Shows only the length. Never a prefix, never a suffix: four characters
    /// of a key are enough to recognize it in a leak.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ApiKey(<masked, {} bytes>)", self.0.len())
    }
}

impl Drop for ApiKey {
    fn drop(&mut self) {
        // `String::into_bytes` reuses the allocation: so it is indeed the
        // buffer that carried the key that is overwritten, not a copy.
        let mut bytes = std::mem::take(&mut self.0).into_bytes();
        bytes.fill(0);
    }
}

/// What replaces the key in a scrubbed text.
///
/// In English: this mention ends up in a displayed error message, and that is
/// the language of the source code (CLAUDE.md § Language).
pub(crate) const REDACTED: &str = "<redacted API key>";

/// Replaces every literal occurrence of the key with a neutral mention.
///
/// Some providers copy the received key into their error message. This filter
/// is the last barrier before a response body becomes an Oxyn error message,
/// hence a log (I-03).
///
/// An empty or blank key is not searched for: it would appear everywhere.
#[must_use]
pub(crate) fn redact_key(text: &str, key: Option<&ApiKey>) -> String {
    match key {
        Some(key) if !key.is_blank() => text.replace(key.expose(), REDACTED),
        _ => text.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_leaks_nothing() {
        let key = ApiKey::new("sk-proj-0123456789abcdef");
        let rendered = format!("{key:?}");
        assert!(!rendered.contains("sk-proj"), "{rendered}");
        assert!(!rendered.contains("0123"), "{rendered}");
        assert!(rendered.contains("masked"), "{rendered}");
    }

    #[test]
    fn an_option_debug_leaks_nothing_either() {
        // The real case: `#[derive(Debug)]` on a struct that carries
        // `Option<ApiKey>` delegates to the `Debug` of `ApiKey`.
        let wrapped = Some(ApiKey::new("sk-secret"));
        let rendered = format!("{wrapped:?}");
        assert!(!rendered.contains("secret"), "{rendered}");
    }

    #[test]
    fn a_blank_key_is_recognized() {
        assert!(ApiKey::new("").is_blank());
        assert!(ApiKey::new("   \t\n").is_blank());
        assert!(!ApiKey::new("sk-x").is_blank());
    }

    #[test]
    fn redaction_erases_the_key_copied_by_the_provider() {
        let key = ApiKey::new("sk-abcdef");
        let body = r#"{"error":{"message":"Incorrect API key provided: sk-abcdef"}}"#;
        let filter = redact_key(body, Some(&key));
        assert!(!filter.contains("sk-abcdef"), "{filter}");
        assert!(filter.contains(REDACTED), "{filter}");
    }

    #[test]
    fn redaction_without_a_key_leaves_the_text_intact() {
        let body = "model not found";
        assert_eq!(redact_key(body, None), body);
    }

    #[test]
    fn a_blank_key_is_not_used_as_a_redaction_pattern() {
        // Otherwise `replace("", …)` would insert the mention between every character.
        let key = ApiKey::new("   ");
        assert_eq!(redact_key("abc", Some(&key)), "abc");
    }
}
