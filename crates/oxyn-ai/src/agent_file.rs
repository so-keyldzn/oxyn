//! Agents declared as Markdown files, and the one way a system prompt is made.
//!
//! Authority: [ADR-0049](../../../docs/adr/0049-agents-declared-as-markdown-files.md).
//!
//! # Three files make one prompt
//!
//! A system prompt is written along three axes, each in its own file, all
//! shipped with `include_str!`:
//!
//! | File | Axis | Front matter |
//! |---|---|---|
//! | `agents/<role>.md` | what the agent does | the [`AgentSpec`] fields |
//! | `prompts/dialects/<dialect>.md` | the SQL the connection speaks | none |
//! | `prompts/recipients/<recipient>.md` | who reads the prompt, with which tools | none |
//!
//! [`render_system_prompt`] joins them in that order and fills the closed
//! variables. Nothing else is concatenated in: the database's structure and
//! samples reach the model through `ContextBuilder`, under the connection's
//! tier, and never as a variable ([I-04](../../../CLAUDE.md#i-04)). A dialect
//! without a fragment of its own gets `ansi.md`; a recipient without one is an
//! error, since a prompt that does not say how its reader calls tools leaves
//! the reader to guess.
//!
//! # A file is input
//!
//! Shipped files are checked by tests; a user's file, later, is not. The
//! parser is therefore strict ([`parse_agent_file`]), refuses rather than
//! guesses, and never panics on what it reads
//! ([I-09](../../../CLAUDE.md#i-09)). An error names the file and the line, and
//! repeats nothing the file says: the agent picker shows it, and a log may
//! record it.

mod files;
mod parse;
mod render;
mod target;
#[cfg(test)]
mod tests;

use std::fmt;

pub use files::shipped_agents;
pub(crate) use files::{SCHEMA_AGENT, SQL_AGENT, shipped_agent};
pub use parse::parse_agent_file;
pub use render::render_system_prompt;
pub(crate) use target::dialect_names;
pub use target::{DIALECTS, ExternalAgentKind, PromptTarget, Recipient};

#[cfg(doc)]
use crate::spec::AgentSpec;

/// Largest file Oxyn reads as an agent or a fragment, in bytes.
///
/// Checked before parsing: `serde-saphyr`'s own size cap applies to readers,
/// not to a string (RESEARCH-NOTES). A prompt is a page or two; 64 KiB leaves
/// room for that and none for a file that is something else.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// The variables a prompt may name, as `{{name}}`.
///
/// Closed, and extended only by an ADR (ADR-0049 § 3): each is a value Oxyn
/// holds, never database content, a server response or a secret.
pub const VARIABLES: [&str; 4] = ["dialect", "driver", "environment", "recipient"];

/// Why a file was refused.
///
/// Every variant names the file and, where there is one, the line. None
/// carries the file's text: the messages are Oxyn's, from a closed list, so a
/// user's file cannot write into the picker or a log through its errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentFileError {
    /// The file is larger than [`MAX_FILE_BYTES`].
    #[error("{file}: {bytes} bytes, over the {MAX_FILE_BYTES}-byte limit of an agent file")]
    TooLarge {
        /// The file.
        file: String,
        /// Its size.
        bytes: usize,
    },

    /// The file does not start with the `---` line that opens a front matter.
    #[error("{file}, line 1: an agent file starts with a `---` line opening its front matter")]
    MissingFrontMatter {
        /// The file.
        file: String,
    },

    /// The front matter is never closed by a `---` line.
    #[error("{file}, line {line}: the front matter opened on line 1 is never closed by `---`")]
    UnclosedFrontMatter {
        /// The file.
        file: String,
        /// The last line of the file.
        line: u64,
    },

    /// The front matter is not what an agent declaration accepts.
    #[error("{file}, line {line}: {problem}")]
    FrontMatter {
        /// The file.
        file: String,
        /// The line, counted from the top of the file.
        line: u64,
        /// What is wrong, from a closed list.
        problem: YamlProblem,
    },

    /// `applies_to` names a dialect Oxyn does not know.
    #[error("{file}, line {line}: unknown dialect in `applies_to`")]
    UnknownDialect {
        /// The file.
        file: String,
        /// The line of the name.
        line: u64,
    },

    /// `recipients` names a recipient Oxyn does not know.
    #[error("{file}, line {line}: unknown recipient in `recipients`")]
    UnknownRecipient {
        /// The file.
        file: String,
        /// The line of the name.
        line: u64,
    },

    /// A `{{` does not open one of the [`VARIABLES`].
    #[error(
        "{file}, line {line}: unknown variable; a prompt may only name {{{{dialect}}}}, \
         {{{{driver}}}}, {{{{environment}}}} and {{{{recipient}}}}"
    )]
    UnknownVariable {
        /// The file, or the agent's name for a declaration built in code.
        file: String,
        /// The line of the `{{`.
        line: u64,
    },

    /// The declaration breaks a rule of [`AgentSpec::validate`].
    #[error("{file}: {reason}")]
    Declaration {
        /// The file.
        file: String,
        /// `validate`'s sentence. It may quote a tool name the file lists,
        /// and nothing else of it.
        reason: String,
    },

    /// No fragment is written for this recipient.
    #[error("no prompt fragment is written for the recipient `{recipient}`")]
    NoRecipientFragment {
        /// The recipient's name, from Oxyn's closed list.
        recipient: &'static str,
    },
}

/// What is wrong in a front matter, without repeating it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum YamlProblem {
    /// A key the declaration does not have — often a misspelling.
    UnknownKey,
    /// A key given twice.
    DuplicateKey,
    /// A required key is missing.
    MissingKey,
    /// An anchor or an alias: an agent file has no use for references.
    AnchorOrAlias,
    /// A `<<` merge key.
    MergeKey,
    /// A boolean written other than `true` or `false`.
    NotStrictBoolean,
    /// A tag Oxyn does not accept.
    UnsupportedTag,
    /// A value of the wrong type or out of its range.
    WrongValue,
    /// Too deep, or too many nodes, for a declaration.
    TooComplex,
    /// Not well-formed YAML, or not the shape of a declaration.
    Malformed,
}

impl fmt::Display for YamlProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnknownKey => "a key an agent declaration does not have",
            Self::DuplicateKey => "a key given twice",
            Self::MissingKey => "a required key (`id` or `name`) is missing",
            Self::AnchorOrAlias => "an anchor or an alias, which an agent file may not use",
            Self::MergeKey => "a `<<` merge key, which an agent file may not use",
            Self::NotStrictBoolean => "a boolean other than `true` or `false`",
            Self::UnsupportedTag => "a YAML tag Oxyn does not accept",
            Self::WrongValue => "a value of the wrong type, or out of its range",
            Self::TooComplex => "nested deeper, or larger, than a declaration can be",
            Self::Malformed => "not a well-formed front matter",
        })
    }
}
