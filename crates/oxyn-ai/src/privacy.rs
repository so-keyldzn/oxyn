//! A connection's privacy tier, applied to an endpoint.
//!
//! Authority: [ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md). The table
//! of tiers lives there and is not copied here.
//!
//! # The type lives in `oxyn-core`, and that is the point
//!
//! [`PrivacyTier`] is defined in
//! [`oxyn_core::connection`] and re-exported here. There is **only one**
//! definition in the repository, and it sits next to the
//! [`ConnectionConfig`](oxyn_core::ConnectionConfig) that carries it: a tier
//! stored in the AI crate would be a tier attached to the AI workspace, hence a
//! global setting under another name — exactly what ADR-0006 refuses.
//!
//! This module therefore only carries what needs `oxyn-llm`: confronting the
//! tier with an endpoint's classification.
//!
//! # `Local` is a guarantee, not a preference
//!
//! [`PrivacyTier::allows_remote_provider`] returns `false` for `Local`, and
//! [`AgentRuntime::run`](crate::runtime::AgentRuntime::run) refuses before
//! assembling anything: there is no path that sends a prompt off the machine
//! under this tier. The check happens on the **session**, because it is the
//! only place where the connection's tier is known.
//!
//! # The `localhost` proxy trap
//!
//! The local/remote classification is made on the real host **after
//! resolution** ([`Reach`]), never on the presence of `localhost` in a URL: an
//! OpenAI-compatible endpoint listening on the loopback can be a proxy that
//! forwards to the cloud. It is re-checked at every configuration change,
//! because the name that resolved to `127.0.0.1` yesterday may resolve
//! elsewhere today.

use oxyn_llm::Reach;

pub use oxyn_core::PrivacyTier;

/// Is this endpoint usable under this tier?
///
/// [`Reach::Unresolved`] is treated as remote: an endpoint we could not
/// classify does not get the benefit of the doubt
/// ([`Reach::leaves_machine`]).
///
/// A free function rather than a method: an endpoint's classification lives in
/// `oxyn-llm`, which `oxyn-core` does not depend on — and must not depend on.
#[must_use]
pub const fn allows_endpoint(tier: PrivacyTier, reach: Reach) -> bool {
    !reach.leaves_machine() || tier.allows_remote_provider()
}

/// The reach of an external agent: **unknowable**, hence [`Reach::Unresolved`].
///
/// It is not the same thing as a badly resolved endpoint. A declared provider
/// has a URL that can be resolved, and whose resolution can be stale
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
/// An external agent is an **opaque process**: it can talk to a local model,
/// to a remote service, or switch between two turns, and nothing in the
/// protocol allows asking it
/// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
///
/// The function takes the declaration so that the caller cannot pick the wrong
/// value, and returns a constant because there is nothing to compute: it is
/// the absence of information that is modeled, not a failed measurement.
#[must_use]
pub const fn agent_reach(_agent: &oxyn_core::ExternalAgentConfig) -> Reach {
    Reach::Unresolved
}

/// Is this external agent usable under this tier?
///
/// A direct consequence of [`agent_reach`]: **not under `Local`**, yes under
/// `Metadata` and `Sampled`. A user whose agent really runs against a local
/// model will find the restriction excessive, and they will be right in
/// substance — but lifting it would require taking their word for it, and
/// `Local` promises "nothing leaves the machine". A promise that comes with a
/// checkbox is no longer a promise.
#[must_use]
pub const fn allows_external_agent(
    tier: PrivacyTier,
    agent: &oxyn_core::ExternalAgentConfig,
) -> bool {
    allows_endpoint(tier, agent_reach(agent))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An external agent never serves a `Local` connection.
    ///
    /// It is the consequence of ADR-0026 that matters, and it relies on no new
    /// mechanism: an agent is `Unresolved`, and "when in doubt, protect" was
    /// already the rule.
    #[test]
    fn an_external_agent_never_serves_a_local_connection() {
        use oxyn_core::{ExternalAgentConfig, ProviderId};

        let agent = ExternalAgentConfig::new(
            ProviderId::new("agent-local").expect("a valid identifier"),
            "Claude Code",
            "claude",
        )
        .with_args(["--acp"]);

        assert_eq!(
            agent_reach(&agent),
            Reach::Unresolved,
            "an opaque process has no knowable reach"
        );
        assert!(
            !allows_external_agent(PrivacyTier::Local, &agent),
            "`Local` promises nothing leaves: an agent whose output we cannot see cannot keep that promise"
        );
        // The two other tiers accept it, otherwise the mode would exist for
        // nobody.
        assert!(allows_external_agent(PrivacyTier::Metadata, &agent));
        assert!(allows_external_agent(PrivacyTier::Sampled, &agent));
    }

    #[test]
    fn an_unresolved_endpoint_is_treated_as_remote() {
        // The AI-PROVIDERS trap: a proxy listening on localhost. The
        // classification comes from `Reach`, never from the URL's shape.
        assert!(allows_endpoint(PrivacyTier::Local, Reach::Local));
        assert!(!allows_endpoint(PrivacyTier::Local, Reach::Remote));
        assert!(
            !allows_endpoint(PrivacyTier::Local, Reach::Unresolved),
            "when in doubt, protect"
        );
        assert!(allows_endpoint(PrivacyTier::Metadata, Reach::Unresolved));
    }

    #[test]
    fn the_tier_returned_here_is_the_domain_one() {
        // A single definition in the repository: if someone reintroduced a
        // local one, this type equality would no longer compile.
        let du_domaine: oxyn_core::PrivacyTier = oxyn_core::PrivacyTier::Metadata;
        let reexporte: PrivacyTier = du_domaine;
        assert_eq!(reexporte, PrivacyTier::default());
    }
}
