//! Where a request goes: on this machine, or outside.
//!
//! [`AI-PROVIDERS`](../../../docs/AI-PROVIDERS.md) sets the rule and the trap:
//!
//! > An "OpenAI-compatible" endpoint pointed at `localhost` can be a proxy
//! > that re-emits to the cloud. The local/remote classification is made on
//! > the real host **after resolution**, never on the presence of `localhost`
//! > in the URL, and it is re-checked at every configuration change.
//!
//! Hence two functions and not one: [`literal_reach`] settles without a
//! network the decidable cases (a literal IP address), [`resolve_reach`] does
//! the DNS resolution for the others.
//!
//! # What `Local` promises, and what it does not
//!
//! [`Reach::Local`] says that **the TCP connection ends on this machine**. It
//! does not say that the data stays there: a proxy listening on `127.0.0.1`
//! can re-emit anywhere, and no inspection of the endpoint will detect it.
//! What the classification brings is the elimination of the opposite case —
//! an endpoint named `localhost.my-cloud.example` that is local only in
//! name.

use std::fmt;
use std::net::{IpAddr, ToSocketAddrs};

use reqwest::Url;

/// Classification of an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Reach {
    /// The host is a loopback address: the connection does not leave the
    /// machine. See the module's caveat.
    Local,
    /// The host is reachable elsewhere than on the loopback. The data
    /// **leaves the machine**.
    Remote,
    /// The host could not be resolved, or the URL has no host at all.
    ///
    /// To be treated as [`Remote`](Self::Remote) wherever a decision must be
    /// made: when in doubt, protect.
    Unresolved,
}

impl Reach {
    /// Does the data leave the machine, as far as is known?
    ///
    /// [`Unresolved`](Self::Unresolved) answers `true`: an endpoint that could
    /// not be classified does not get the benefit of the doubt.
    #[must_use]
    pub const fn leaves_machine(&self) -> bool {
        !matches!(self, Self::Local)
    }

    /// Stable name, for display and audit.
    ///
    /// **In English**, like everything that crosses the source code boundary
    /// (CLAUDE.md): this value is shown as is by the provider configuration
    /// screen, and the rest of Oxyn's interface is in English. A domain label
    /// in another language than the screen displaying it forces every caller
    /// to translate it again — hence to reinvent a mapping per variant, which
    /// will diverge.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Remote => "remote",
            Self::Unresolved => "unresolved",
        }
    }
}

impl fmt::Display for Reach {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Extracts the literal IP address of a URL, when its host is one.
///
/// `host_str` returns an IPv6 between brackets (`[::1]`): they are removed
/// before parsing.
fn literal_ip(url: &Url) -> Option<IpAddr> {
    let host = url.host_str()?;
    let bare = host
        .strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .unwrap_or(host);
    bare.parse::<IpAddr>().ok()
}

/// Classifies an endpoint **without a network**, when possible.
///
/// Returns `Some` only for a literal IP address — the only case where the
/// question is settled without resolution. A domain name, `localhost`
/// included, returns `None`: that is exactly the trap the rule targets.
#[must_use]
pub fn literal_reach(url: &Url) -> Option<Reach> {
    literal_ip(url).map(|ip| {
        if ip.is_loopback() {
            Reach::Local
        } else {
            Reach::Remote
        }
    })
}

/// Classifies an endpoint, resolving its name if needed.
///
/// A host is [`Reach::Local`] only if **all** its resolved addresses are
/// loopback: a name that resolves both to `127.0.0.1` and to a public address
/// is remote, because the second one is the one that may be used.
///
/// # Blocking
///
/// The standard library's DNS resolution is **blocking**. This function must
/// never be called from the UI thread (I-05), nor from an async task without
/// going through a blocking pool. It is meant to be called when a provider is
/// registered and at each change of its configuration, not at each request.
/// Pass this measurement to [`crate::build_provider`]: classification alone
/// does not constrain a later DNS answer or the socket.
#[must_use]
pub fn resolve_reach(url: &Url) -> Reach {
    if let Some(immediate) = literal_reach(url) {
        return immediate;
    }
    let Some(host) = url.host_str() else {
        return Reach::Unresolved;
    };
    // The port does not matter to resolution; `0` avoids imposing an arbitrary
    // default when the URL carries none and the scheme imposes none.
    let port = url.port_or_known_default().unwrap_or(0);
    let Ok(addresses) = (host, port).to_socket_addrs() else {
        return Reach::Unresolved;
    };
    let mut seen = false;
    for address in addresses {
        seen = true;
        if !address.ip().is_loopback() {
            return Reach::Remote;
        }
    }
    if seen {
        Reach::Local
    } else {
        Reach::Unresolved
    }
}

/// Classifies an endpoint given as a string.
///
/// It is the entry point for a caller that holds an
/// [`AiProviderConfig::base_url`](oxyn_core::AiProviderConfig) — a `String` —
/// and has no reason to depend on the HTTP client to parse a URL.
///
/// An **unreadable URL returns [`Reach::Unresolved`]**, never an error: an
/// endpoint that cannot be classified counts as remote wherever a decision is
/// made ([`Reach::leaves_machine`] already answers `true` on it). Returning a
/// `Result` would force every caller to choose a default, and the wrong
/// default — "local" — is silent.
///
/// # Blocking
///
/// The standard library's DNS resolution is **blocking**. Like
/// [`resolve_reach`], this function must never be called from the UI thread
/// (I-05), nor from an async task without going through a blocking pool. It is
/// meant to be called when a provider is registered and at each runtime
/// opening, not at each request.
#[must_use]
pub fn endpoint_reach(base_url: &str) -> Reach {
    match Url::parse(base_url) {
        Ok(url) => resolve_reach(&url),
        Err(_) => Reach::Unresolved,
    }
}

/// Returns a URL that can be shown, stripped of its credentials.
///
/// A URL can carry a `user:password` pair in its authority part. Copying it
/// into an error message or a `Debug` is a leak (I-03) — and that is exactly
/// what the natural display of a [`Url`] does. Query parameters and fragments
/// can also carry tokens, so neither is ever shown.
#[must_use]
pub fn redacted(url: &Url) -> String {
    let mut clean = url.clone();
    // `set_username` and `set_password` fail on URLs without an authority
    // (`data:`, `mailto:`): there is then no credential to remove.
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean.set_query(None);
    clean.set_fragment(None);
    clean.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(raw: &str) -> Url {
        Url::parse(raw).expect("valid test URL")
    }

    #[test]
    fn a_literal_loopback_address_is_settled_without_dns() {
        assert_eq!(
            literal_reach(&url("http://127.0.0.1:11434/v1")),
            Some(Reach::Local)
        );
        assert_eq!(
            literal_reach(&url("http://127.0.0.53:8080")),
            Some(Reach::Local)
        );
        assert_eq!(
            literal_reach(&url("http://[::1]:1234/v1")),
            Some(Reach::Local)
        );
    }

    #[test]
    fn a_literal_public_address_is_remote() {
        assert_eq!(
            literal_reach(&url("https://93.184.216.34/v1")),
            Some(Reach::Remote)
        );
        assert_eq!(
            literal_reach(&url("http://192.168.1.10:11434/v1")),
            Some(Reach::Remote),
            "the local network is not the local machine: the data leaves"
        );
    }

    #[test]
    fn a_domain_name_is_not_settled_on_its_shape() {
        // The AI-PROVIDERS trap, from both sides.
        assert_eq!(literal_reach(&url("http://localhost:11434/v1")), None);
        assert_eq!(
            literal_reach(&url("https://localhost.mon-nuage.example/v1")),
            None,
            "a name containing `localhost` proves nothing"
        );
    }

    #[test]
    fn an_unresolved_endpoint_gets_no_benefit_of_the_doubt() {
        assert!(Reach::Unresolved.leaves_machine());
        assert!(Reach::Remote.leaves_machine());
        assert!(!Reach::Local.leaves_machine());
    }

    #[test]
    fn a_literal_address_resolves_without_network() {
        // `resolve_reach` short-circuits: this test depends on no resolver.
        assert_eq!(
            resolve_reach(&url("http://127.0.0.1:11434/v1")),
            Reach::Local
        );
        assert_eq!(resolve_reach(&url("https://8.8.8.8/")), Reach::Remote);
    }

    #[test]
    fn an_unreadable_string_gets_no_benefit_of_the_doubt() {
        // The wrong default would be "local", and it would be silent.
        for raw in ["", "not a url", "://", "mailto:someone@example.com"] {
            assert_eq!(endpoint_reach(raw), Reach::Unresolved, "{raw}");
            assert!(endpoint_reach(raw).leaves_machine(), "{raw}");
        }
    }

    #[test]
    fn a_literal_string_is_classified_without_network() {
        assert_eq!(endpoint_reach("http://127.0.0.1:11434"), Reach::Local);
        assert_eq!(endpoint_reach("http://[::1]:1234/v1"), Reach::Local);
        assert_eq!(endpoint_reach("https://93.184.216.34/v1"), Reach::Remote);
    }

    #[test]
    fn the_credentials_of_a_url_are_not_displayed() {
        let with_credentials = url("https://alice:hunter2@api.example.com/v1/");
        let rendered = redacted(&with_credentials);
        assert!(!rendered.contains("hunter2"), "{rendered}");
        assert!(!rendered.contains("alice"), "{rendered}");
        assert!(rendered.contains("api.example.com"), "{rendered}");
    }

    #[test]
    fn a_url_without_credentials_is_returned_as_is() {
        let simple = url("http://localhost:11434/v1/");
        assert_eq!(redacted(&simple), "http://localhost:11434/v1/");
    }
}
