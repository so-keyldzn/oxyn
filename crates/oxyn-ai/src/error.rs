//! The failures the agent runtime can name.
//!
//! One enumeration per boundary (the repository's Rust rule): this one carries
//! the refusals and misunderstandings of the **model → command bus** boundary.
//! It does not carry the execution errors of a command — those belong to
//! [`OxynError`] and come up as they are through [`AiError::Core`].
//!
//! # A refusal is not a failure
//!
//! [`AiError::ToolNotAllowed`] and [`AiError::RemoteProviderRefused`] describe
//! the product doing its job: an agent asked for something that its
//! declaration or the connection's privacy tier forbids. They map onto
//! [`oxyn_core::OxynError::PolicyDenied`], which carries
//! exactly that meaning.
//!
//! # What never appears in a message
//!
//! No message of this module copies the **content** of a tool argument or of a
//! result: under the `Sampled` tier that content is made of real rows from the
//! user's database, and an error message ends up in a log (I-03). The tool's
//! name and the nature of the defect are enough for diagnosis.

use oxyn_core::OxynError;
use thiserror::Error;

use crate::privacy::PrivacyTier;

/// What can go wrong between a model and the command bus.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum AiError {
    /// The model asked for a tool that does not exist in the registry.
    ///
    /// A nominal case, not an exceptional one: a model invents a tool name from
    /// time to time. The message is fed back into the conversation so that it
    /// corrects itself.
    #[error("unknown tool: `{name}`")]
    UnknownTool {
        /// The requested name, as the provider transmitted it.
        name: String,
    },

    /// The tool exists, but it is not in this agent's allowlist.
    ///
    /// The allowlist is the only authority: a tool known to the registry and
    /// absent from the agent's declaration is refused, not executed "since it
    /// exists".
    #[error("tool `{name}` is not allowed for this agent")]
    ToolNotAllowed {
        /// The requested name.
        name: String,
    },

    /// The arguments do not match the tool's schema.
    ///
    /// `detail` comes from the deserializer: it names the faulty field and the
    /// expected type, never the value received.
    #[error("invalid arguments for tool `{name}`: {detail}")]
    InvalidArguments {
        /// The tool's name.
        name: String,
        /// What the deserializer refused.
        detail: String,
    },

    /// The connection's privacy tier forbids this provider.
    ///
    /// [`PrivacyTier::Local`] is a **guarantee**, not a preference: no path
    /// lets it be overridden, and this is where the refusal happens — when the
    /// runtime is built, before any context has been assembled (ADR-0006).
    #[error("privacy tier `{tier}` forbids a remote model provider")]
    RemoteProviderRefused {
        /// The tier attached to the connection.
        tier: PrivacyTier,
    },

    /// An agent declaration is inconsistent.
    ///
    /// An agent can come from a plugin, hence from a file written by a third
    /// party: its declaration is input to validate, not trusted data
    /// (SECURITY, input surface).
    #[error("invalid agent specification: {0}")]
    InvalidSpec(String),

    /// The provider reported an incident **during** the stream.
    ///
    /// Distinct from [`Core`](Self::Core), which carries failures that happened
    /// before the first byte: once text is displayed, the incident is an event
    /// and not a return value.
    #[error("model provider failed mid-stream: {0}")]
    Provider(String),

    /// The stream stopped **before the end of the turn** without the provider
    /// announcing it ([`StopReason::Interrupted`](oxyn_core::ai::StopReason)).
    ///
    /// Distinct from [`Provider`](Self::Provider), where the provider stated
    /// its failure: here it may have finished the generation — and billed it —
    /// without anything arriving. It is the ambiguous case of I-13, carried as
    /// data by [`class`](Self::class) and not inferred from the message.
    #[error(
        "the answer was interrupted before it ended; the provider may have finished — and billed — \
         this turn: {0}"
    )]
    Interrupted(String),

    /// A reasoning effort the model does not declare.
    ///
    /// Refused before sending, never transmitted "to see": a provider that
    /// silently ignores it bills an answer that is not the one asked for, and
    /// one that refuses it does so after a round trip.
    #[error("{}", effort_not_offered(*effort, *reasoning))]
    ReasoningEffortNotOffered {
        /// The requested effort.
        effort: oxyn_llm::ReasoningEffort,
        /// What the provider says of the model's ability to reason. `Unknown`
        /// is not `No`: the message does not say "does not reason" of a model
        /// we know nothing about.
        reasoning: oxyn_llm::Support,
    },

    /// A domain error, passed on without reinterpretation.
    #[error(transparent)]
    Core(#[from] OxynError),
}

fn effort_not_offered(effort: oxyn_llm::ReasoningEffort, reasoning: oxyn_llm::Support) -> String {
    match reasoning {
        oxyn_llm::Support::No => format!(
            "the model does not reason, so no reasoning effort can be set (asked: {})",
            effort.as_str()
        ),
        _ => format!(
            "the provider does not declare the reasoning effort `{}` for this model",
            effort.as_str()
        ),
    }
}

impl AiError {
    /// The error's class, when it carries one.
    ///
    /// `None` when nothing fixes it: the caller does not guess it again from
    /// the message.
    #[must_use]
    pub const fn class(&self) -> Option<oxyn_core::ErrorClass> {
        match self {
            Self::Interrupted(_) => Some(oxyn_core::ErrorClass::Ambiguous),
            Self::Core(error) => Some(error.class()),
            _ => None,
        }
    }

    /// Is the error a **policy refusal** rather than a failure?
    ///
    /// Used by the interface: a refusal shows as a decision of the product, not
    /// as an incident to report.
    #[must_use]
    pub const fn is_refusal(&self) -> bool {
        matches!(
            self,
            Self::ToolNotAllowed { .. } | Self::RemoteProviderRefused { .. }
        )
    }

    /// Can the error be sent back to the model so that it corrects itself?
    ///
    /// True for what comes from the model's output — invented tool name,
    /// malformed arguments —, false for what comes from configuration or the
    /// network: feeding back "the provider is unreachable" will not make it
    /// write a better query.
    #[must_use]
    pub const fn is_recoverable_by_model(&self) -> bool {
        matches!(
            self,
            Self::UnknownTool { .. } | Self::ToolNotAllowed { .. } | Self::InvalidArguments { .. }
        )
    }
}

impl From<AiError> for OxynError {
    /// Maps the error onto the domain, keeping its **meaning** and not its
    /// wording.
    ///
    /// The interface decides on the variant: a refusal must remain a refusal
    /// after the conversion, otherwise a `PolicyDenied` would end up displayed
    /// as an internal incident.
    fn from(err: AiError) -> Self {
        // The message is rendered before the `match`: the `Core` variant
        // consumes its content, and computing it afterwards would complicate
        // reading for one allocation.
        let message = err.to_string();
        match err {
            AiError::Core(inner) => inner,
            AiError::UnknownTool { .. }
            | AiError::ToolNotAllowed { .. }
            | AiError::RemoteProviderRefused { .. } => Self::PolicyDenied { reason: message },
            AiError::InvalidArguments { .. } => Self::Serialization(message),
            AiError::InvalidSpec(_) | AiError::ReasoningEffortNotOffered { .. } => {
                Self::Config(message)
            }
            AiError::Provider(_) => Self::Connection(message),
            // Not replayable, like the cut `oxyn-llm` maps the same way: the
            // domain has no ambiguous variant besides `Timeout`, and
            // `Connection` would make a possibly billed turn replayable.
            AiError::Interrupted(_) => Self::Io(std::io::Error::new(
                std::io::ErrorKind::ConnectionAborted,
                message,
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refusal_stays_a_refusal_after_mapping() {
        let refusal = AiError::ToolNotAllowed {
            name: "delete_connection".to_owned(),
        };
        assert!(refusal.is_refusal());
        let projected = OxynError::from(refusal);
        assert!(
            matches!(projected, OxynError::PolicyDenied { .. }),
            "{projected:?}"
        );
    }

    #[test]
    fn a_local_tier_refuses_a_remote_provider() {
        let refusal = AiError::RemoteProviderRefused {
            tier: PrivacyTier::Local,
        };
        let rendered = refusal.to_string();
        assert!(rendered.contains("local"), "{rendered}");
        assert!(refusal.is_refusal());
    }

    #[test]
    fn only_what_comes_from_the_model_is_sent_back_to_it() {
        assert!(
            AiError::UnknownTool {
                name: "x".to_owned()
            }
            .is_recoverable_by_model()
        );
        assert!(
            AiError::InvalidArguments {
                name: "execute_query".to_owned(),
                detail: "missing field `statement`".to_owned(),
            }
            .is_recoverable_by_model()
        );
        assert!(
            !AiError::Provider("connection reset".to_owned()).is_recoverable_by_model(),
            "feeding back a network failure does not make it write a better query"
        );
    }
}
