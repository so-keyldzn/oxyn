//! Reading an agent file: a YAML front matter, then the prompt.

use oxyn_core::AgentId;
use serde::Deserialize;
use serde_saphyr::budget::BudgetBreach;
use serde_saphyr::{Budget, DuplicateKeyPolicy, MergeKeyPolicy, Options, Spanned};

use super::render::check_variables;
use super::target::{Recipient, dialect_named};
use super::{AgentFileError, MAX_FILE_BYTES, YamlProblem};
use crate::context::ContextPolicy;
use crate::error::AiError;
use crate::spec::{AgentSpec, DEFAULT_MAX_TURNS};
use crate::tools::ToolRegistry;

/// The line that opens and closes a front matter.
const FENCE: &str = "---";

/// The front matter: every field of [`AgentSpec`] but the prompt, under the
/// same names, `tools` for `allowed_tools`.
///
/// `deny_unknown_fields`: a misspelled key is an error, not a setting silently
/// ignored (ADR-0049 § 1). Dialects and recipients are read as spanned strings
/// so that an unknown one is reported at its line.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FrontMatter {
    id: AgentId,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    applies_to: Vec<Spanned<String>>,
    #[serde(default)]
    recipients: Vec<Spanned<String>>,
    #[serde(default)]
    tools: Vec<String>,
    #[serde(default)]
    context: ContextPolicy,
    #[serde(default)]
    output_schema: Option<serde_json::Value>,
    #[serde(default = "default_max_turns")]
    max_turns: usize,
}

const fn default_max_turns() -> usize {
    DEFAULT_MAX_TURNS
}

/// Reads an agent file named `name` (for its errors only).
///
/// Refused, with the file name and the line: a file over
/// [`MAX_FILE_BYTES`](super::MAX_FILE_BYTES) (checked before parsing); a front
/// matter missing, or not the first thing in the file; YAML with an anchor,
/// an alias, a merge key, a duplicate key, a boolean other than `true` /
/// `false`, an unknown tag or an unknown key; an unknown dialect or
/// recipient; a prompt naming a variable outside
/// [`VARIABLES`](super::VARIABLES); a declaration
/// [`AgentSpec::validate`] refuses against the shipped tool registry.
///
/// Never panics, whatever the text: these files can come from a user.
///
/// # Errors
/// [`AgentFileError`], one variant per reason above.
pub fn parse_agent_file(name: &str, text: &str) -> Result<AgentSpec, AgentFileError> {
    let file = || name.to_owned();
    if text.len() > MAX_FILE_BYTES {
        return Err(AgentFileError::TooLarge {
            file: file(),
            bytes: text.len(),
        });
    }
    let (yaml, body, body_line) = split(name, text)?;

    let front: FrontMatter =
        serde_saphyr::from_str_with_options(yaml, strict()).map_err(|err| {
            AgentFileError::FrontMatter {
                file: file(),
                // The YAML starts on line 2, after the opening fence.
                line: err.location().map_or(1, |at| at.line()).saturating_add(1),
                problem: problem(&err),
            }
        })?;

    let applies_to = front
        .applies_to
        .iter()
        .map(|entry| {
            dialect_named(&entry.value).ok_or_else(|| AgentFileError::UnknownDialect {
                file: file(),
                line: entry.referenced.line().saturating_add(1),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let recipients = front
        .recipients
        .iter()
        .map(|entry| {
            Recipient::named(&entry.value).ok_or_else(|| AgentFileError::UnknownRecipient {
                file: file(),
                line: entry.referenced.line().saturating_add(1),
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    check_variables(name, body, body_line)?;

    let mut spec = AgentSpec::new(front.id, front.name, body.trim())
        .with_description(front.description)
        .with_tools(front.tools)
        .with_context(front.context)
        .with_max_turns(front.max_turns);
    spec.output_schema = front.output_schema;
    spec.applies_to = applies_to;
    spec.recipients = recipients;

    spec.validate(&ToolRegistry::builtin())
        .map_err(|err| AgentFileError::Declaration {
            file: file(),
            reason: match err {
                AiError::UnknownTool { .. } => "`tools` names a tool Oxyn does not have".to_owned(),
                other => other.to_string(),
            },
        })?;
    Ok(spec)
}

/// Splits the file into its YAML, its body, and the line the body starts on.
///
/// The front matter must open on the very first line: a file with text, a
/// blank line or a byte order mark before it is one where the author's
/// settings would otherwise be read as prompt.
fn split<'a>(name: &str, text: &'a str) -> Result<(&'a str, &'a str, u64), AgentFileError> {
    let mut lines = text.split_inclusive('\n');
    let opens = lines
        .next()
        .is_some_and(|first| first.trim_end_matches(['\r', '\n']) == FENCE);
    if !opens {
        return Err(AgentFileError::MissingFrontMatter {
            file: name.to_owned(),
        });
    }
    let yaml_start = text.len() - lines.clone().map(str::len).sum::<usize>();
    let mut offset = yaml_start;
    let mut line: u64 = 1;
    for current in lines {
        line = line.saturating_add(1);
        if current.trim_end_matches(['\r', '\n']) == FENCE {
            let yaml = text.get(yaml_start..offset).unwrap_or_default();
            let body = text.get(offset + current.len()..).unwrap_or_default();
            return Ok((yaml, body, line.saturating_add(1)));
        }
        offset += current.len();
    }
    Err(AgentFileError::UnclosedFrontMatter {
        file: name.to_owned(),
        line,
    })
}

/// The parser settings ADR-0049 § 1 decides, on top of the defaults.
///
/// An agent file has no use for references between nodes: each refused
/// feature is either a resource-exhaustion lever or a way for a key to mean
/// something other than what it reads.
fn strict() -> Options {
    let mut budget = Budget::default();
    budget.max_anchors = 0;
    budget.max_aliases = 0;
    let mut options = Options::default();
    options.budget = Some(budget);
    options.merge_keys = MergeKeyPolicy::Error;
    options.duplicate_keys = DuplicateKeyPolicy::Error;
    options.strict_booleans = true;
    options.reject_unsupported_tags = true;
    // The snippet would quote the file's lines into the error.
    options.with_snippet = false;
    options
}

/// What a parser error means, without its text: the library's message may
/// quote the file.
fn problem(err: &serde_saphyr::Error) -> YamlProblem {
    use serde_saphyr::Error;
    match err {
        Error::SerdeUnknownField { .. } => YamlProblem::UnknownKey,
        Error::DuplicateMappingKey { .. } => YamlProblem::DuplicateKey,
        Error::SerdeMissingField { .. } => YamlProblem::MissingKey,
        Error::MergeKeyNotAllowed { .. } => YamlProblem::MergeKey,
        Error::InvalidBooleanStrict { .. } => YamlProblem::NotStrictBoolean,
        Error::UnsupportedTag { .. } => YamlProblem::UnsupportedTag,
        Error::UnknownAnchor { .. } | Error::AliasError { .. } => YamlProblem::AnchorOrAlias,
        Error::Budget { breach, .. } => match breach {
            BudgetBreach::Anchors { .. } | BudgetBreach::Aliases { .. } => {
                YamlProblem::AnchorOrAlias
            }
            _ => YamlProblem::TooComplex,
        },
        Error::SerdeInvalidType { .. }
        | Error::SerdeInvalidValue { .. }
        | Error::SerdeUnknownVariant { .. }
        | Error::InvalidScalar { .. } => YamlProblem::WrongValue,
        _ => YamlProblem::Malformed,
    }
}
