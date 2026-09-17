//! What an external agent says about its own settings: its modes, and the
//! options it lets a client choose — model, reasoning level, others.
//!
//! # Read, bounded, and never guessed
//!
//! Everything here comes from another process. Names and descriptions are
//! shown as text, lists are cut at a bound, and an option of a kind this build
//! does not know is **left out** rather than shown as something it is not: a
//! control that looks like a choice and does nothing is worse than no control.
//!
//! # What the protocol offers, and what is not relayed
//!
//! `session/update` has more variants than this module reads, on purpose:
//!
//! * `UserMessageChunk` replays the user's own messages when a session is
//!   *loaded*. Oxyn keeps its own transcript and loads no session, so a replay
//!   would only duplicate it — or bring back text asked under another tier;
//! * `SessionInfoUpdate` carries a title the agent composes. Like a tool call's
//!   title, it can quote a path of the machine; Oxyn titles a conversation from
//!   the user's question;
//! * `AvailableCommandsUpdate` lists the agent's slash commands. They act on
//!   the agent's own project — files, terminal — which Oxyn does not grant, so
//!   listing them would offer what cannot run. A user who types one still sends
//!   it as a question, through the same gate;
//! * `PlanUpdate`, `PlanRemoved`, `CompactionUpdate` and
//!   `CompactionSummaryChunk` exist only behind the crate's `unstable_*`
//!   features, which Oxyn does not enable: a draft of the protocol is not
//!   something to build a panel on.

use agent_client_protocol::schema::v1::{
    SessionConfigKind, SessionConfigOption, SessionConfigOptionCategory,
    SessionConfigSelectOptions, SessionModeState,
};

/// Modes kept at most. An agent declares a handful.
const MAX_MODES: usize = 32;
/// Options kept at most.
const MAX_OPTIONS: usize = 32;
/// Choices kept per option, groups flattened.
const MAX_CHOICES: usize = 128;
/// The longest name or description kept, in bytes; longer ones are cut.
const MAX_TEXT_BYTES: usize = 512;
/// The longest identifier kept, in bytes. A longer one is **left out**, never
/// cut: it goes back to the agent, and a prefix is an identifier it never
/// offered — or another one it did.
const MAX_ID_BYTES: usize = 512;

/// The agent's settings as it last declared them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentSettings {
    /// The modes the agent offers, in its order.
    pub modes: Vec<AgentChoice>,
    /// The mode in force, when the agent has modes.
    pub current_mode: Option<String>,
    /// The options the agent lets a client set, in its order.
    pub options: Vec<AgentOption>,
}

impl AgentSettings {
    /// Nothing declared: no mode, no option.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.modes.is_empty() && self.current_mode.is_none() && self.options.is_empty()
    }

    /// Reads what `session/new` declared.
    #[must_use]
    pub(crate) fn declared(
        modes: Option<&SessionModeState>,
        options: Option<&[SessionConfigOption]>,
    ) -> Self {
        let mut settings = Self::default();
        if let Some(modes) = modes {
            settings.set_modes(modes);
        }
        if let Some(options) = options {
            settings.set_options(options);
        }
        settings
    }

    fn set_modes(&mut self, modes: &SessionModeState) {
        self.modes = modes
            .available_modes
            .iter()
            .filter_map(|mode| {
                Some(AgentChoice {
                    id: identifier(&mode.id.to_string(), "mode")?,
                    name: bounded(&mode.name),
                    description: mode.description.as_deref().map(bounded),
                })
            })
            .take(MAX_MODES)
            .collect();
        self.current_mode = identifier(&modes.current_mode_id.to_string(), "current mode");
        self.supersede_modes();
    }

    /// The agent switched mode.
    pub(crate) fn set_current_mode(&mut self, id: &str) {
        self.current_mode = identifier(id, "current mode");
        self.supersede_modes();
    }

    /// The agent confirmed the mode it was asked for, with an empty answer.
    ///
    /// Taken only if the agent still declares that mode: a confirmation of a
    /// mode no longer offered is not something to show. Says whether it was.
    pub(crate) fn confirm_mode(&mut self, id: &str) -> bool {
        let declared = self.modes.iter().any(|mode| mode.id == id);
        if declared {
            self.set_current_mode(id);
        }
        declared
    }

    /// The agent sent its options again. The list is **replaced**: the
    /// protocol sends it whole.
    pub(crate) fn set_options(&mut self, options: &[SessionConfigOption]) {
        self.options = options
            .iter()
            .filter_map(AgentOption::of)
            .take(MAX_OPTIONS)
            .collect();
        self.supersede_modes();
    }

    /// An option of category `mode` replaces the `modes` mechanism: the
    /// agent is not offered two mode selectors, one of which it may stop
    /// honouring.
    ///
    /// The protocol's own words, not the crate's — the schema 1.7.0 says
    /// nothing of it: « Session Config Options supersede the older Session
    /// Modes API » and « Clients that support config options SHOULD use
    /// `configOptions` exclusively and ignore `modes` »
    /// (agentclientprotocol.com, `protocol/session-config-options.md`, lines
    /// 333 and 340, read 2026-09-16). Applied only when such an option exists:
    /// an agent that declares `modes` alone keeps its selector.
    ///
    /// The only place this rule lives: every change to the settings ends here.
    fn supersede_modes(&mut self) {
        if self
            .options
            .iter()
            .any(|option| option.category == OptionCategory::Mode)
        {
            self.modes.clear();
            self.current_mode = None;
        }
    }
}

/// One mode, or one value of a select option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChoice {
    /// What the agent calls it on the wire. Never shown.
    pub id: String,
    /// What the user reads.
    pub name: String,
    /// A line more, when the agent gives one.
    pub description: Option<String>,
}

/// One option the agent lets a client set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentOption {
    /// What the agent calls it on the wire. Never shown.
    pub id: String,
    /// What the user reads.
    pub name: String,
    /// A line more, when the agent gives one.
    pub description: Option<String>,
    /// What the option is about, when the agent says so. The effort selector
    /// reads `ThoughtLevel`, the model selector `Model` — nothing else.
    pub category: OptionCategory,
    /// Its value, and what it may take.
    pub value: OptionValue,
}

impl AgentOption {
    fn of(option: &SessionConfigOption) -> Option<Self> {
        let value = match &option.kind {
            SessionConfigKind::Select(select) => OptionValue::Select {
                // An option whose value in force cannot be named is left out
                // whole: a selector would show a current value that is not it.
                current: identifier(&select.current_value.to_string(), "option value")?,
                choices: match &select.options {
                    SessionConfigSelectOptions::Ungrouped(options) => options.iter().collect(),
                    // Flattened in declared order: groups are presentation,
                    // and a selector that drops a group drops choices.
                    SessionConfigSelectOptions::Grouped(groups) => groups
                        .iter()
                        .flat_map(|group| group.options.iter())
                        .collect::<Vec<_>>(),
                    // A shape this build does not know: no choice is shown
                    // rather than a partial one.
                    _ => Vec::new(),
                }
                .into_iter()
                .filter_map(|choice| {
                    Some(AgentChoice {
                        id: identifier(&choice.value.to_string(), "option choice")?,
                        name: bounded(&choice.name),
                        description: choice.description.as_deref().map(bounded),
                    })
                })
                .take(MAX_CHOICES)
                .collect(),
            },
            SessionConfigKind::Boolean(boolean) => OptionValue::Boolean(boolean.current_value),
            // A kind this build cannot set is left out: shown, it would be a
            // control that does nothing.
            _ => return None,
        };
        Some(Self {
            id: identifier(&option.id.to_string(), "option")?,
            name: bounded(&option.name),
            description: option.description.as_deref().map(bounded),
            category: OptionCategory::of(option.category.as_ref()),
            value,
        })
    }
}

/// What an option is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum OptionCategory {
    /// The agent's mode, when it declares modes as an option.
    Mode,
    /// The model answering.
    Model,
    /// A setting of the model other than the reasoning level.
    ModelConfig,
    /// How much the model reasons: the effort selector's source for an agent.
    ThoughtLevel,
    /// No category, or one this build does not know. Never read as one of the
    /// above.
    Other,
}

impl OptionCategory {
    fn of(category: Option<&SessionConfigOptionCategory>) -> Self {
        match category {
            Some(SessionConfigOptionCategory::Mode) => Self::Mode,
            Some(SessionConfigOptionCategory::Model) => Self::Model,
            Some(SessionConfigOptionCategory::ModelConfig) => Self::ModelConfig,
            Some(SessionConfigOptionCategory::ThoughtLevel) => Self::ThoughtLevel,
            _ => Self::Other,
        }
    }
}

/// An option's value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionValue {
    /// One of a list, in the agent's order.
    Select {
        /// The identifier in force.
        current: String,
        /// What it may take.
        choices: Vec<AgentChoice>,
    },
    /// On or off.
    Boolean(bool),
}

/// A change the user asks of the agent's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingChange {
    /// Switch to one of the agent's modes.
    Mode {
        /// A mode identifier, as the agent declared it.
        id: String,
    },
    /// Set one of the agent's options.
    Option {
        /// An option identifier, as the agent declared it.
        id: String,
        /// The new value.
        value: SettingValue,
    },
}

/// The value asked for an option.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingValue {
    /// One of a select option's choices, by identifier.
    Choice(String),
    /// On or off.
    Boolean(bool),
}

/// Why a change was not sent — or not taken.
///
/// None carries the agent's words: an agent's error message may quote a
/// question, a path or rows, and a refused setting needs none of it.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SettingRefused {
    /// A question is being answered. The protocol allows a mode switch then,
    /// but a mode changes the agent's tools and permissions in the middle of
    /// an answer the user asked under the previous one.
    #[error("the agent is answering; change its settings once it has finished")]
    QuestionInProgress,
    /// The agent declared no such mode.
    #[error("the agent does not offer this mode")]
    UnknownMode,
    /// The agent declared no such option.
    #[error("the agent does not offer this option")]
    UnknownOption,
    /// The option exists but offers no such value, or takes another kind.
    #[error("the agent does not offer this value for the option")]
    UnknownValue,
    /// The agent answered with an error. Its code only.
    #[error("the agent refused the change (error {code})")]
    ByAgent {
        /// The JSON-RPC error code.
        code: i32,
    },
}

impl AgentSettings {
    /// Whether `change` names only what the agent declared. Nothing is sent
    /// otherwise: the agent would at best refuse it, and at worst read an
    /// identifier it never offered as something else.
    ///
    /// # Errors
    /// The first thing that does not match.
    pub fn check(&self, change: &SettingChange) -> Result<(), SettingRefused> {
        match change {
            SettingChange::Mode { id } => self
                .modes
                .iter()
                .any(|mode| &mode.id == id)
                .then_some(())
                .ok_or(SettingRefused::UnknownMode),
            SettingChange::Option { id, value } => {
                let option = self
                    .options
                    .iter()
                    .find(|option| &option.id == id)
                    .ok_or(SettingRefused::UnknownOption)?;
                let offered = match (&option.value, value) {
                    (OptionValue::Select { choices, .. }, SettingValue::Choice(choice)) => {
                        choices.iter().any(|offered| &offered.id == choice)
                    }
                    (OptionValue::Boolean(_), SettingValue::Boolean(_)) => true,
                    _ => false,
                };
                offered.then_some(()).ok_or(SettingRefused::UnknownValue)
            }
        }
    }
}

/// The identifier as declared, or `None` past [`MAX_ID_BYTES`] — with a
/// warning that names what was left out, never its content.
fn identifier(id: &str, what: &'static str) -> Option<String> {
    if id.len() > MAX_ID_BYTES {
        tracing::warn!(
            what,
            bytes = id.len(),
            "an external agent declared an identifier past the bound; left out"
        );
        return None;
    }
    Some(id.to_owned())
}

/// At most [`MAX_TEXT_BYTES`], cut on a character boundary.
fn bounded(text: &str) -> String {
    let mut end = text.len().min(MAX_TEXT_BYTES);
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.get(..end).unwrap_or_default().to_owned()
}

#[cfg(test)]
mod tests;
