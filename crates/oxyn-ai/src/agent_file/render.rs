//! Composing a system prompt, and the closed variables.

use super::files::{dialect_fragment, recipient_fragment};
use super::{AgentFileError, PromptTarget, VARIABLES};
use crate::spec::AgentSpec;

/// The system prompt of `spec` for `target`: the role, then the dialect's
/// fragment, then the recipient's, with the variables filled.
///
/// The **only** way a system prompt is produced. The order is fixed, and
/// nothing is joined in besides these three texts: the database's structure
/// and samples reach the model through `ContextBuilder`, which applies the
/// connection's tier (I-04).
///
/// A dialect without a fragment of its own gets `ansi.md`.
///
/// # Errors
/// [`AgentFileError::UnknownVariable`] when one of the three texts names a
/// variable outside [`VARIABLES`] — never filled with an empty string — and
/// [`AgentFileError::NoRecipientFragment`] for a recipient no fragment is
/// written for.
pub fn render_system_prompt(
    spec: &AgentSpec,
    target: &PromptTarget,
) -> Result<String, AgentFileError> {
    let (dialect_file, dialect) = dialect_fragment(target.dialect);
    let (recipient_file, recipient) =
        recipient_fragment(target.recipient).ok_or(AgentFileError::NoRecipientFragment {
            recipient: target.recipient.as_str(),
        })?;
    let parts = [
        (spec.name.as_str(), spec.system_prompt.trim()),
        (dialect_file, dialect.trim()),
        (recipient_file, recipient.trim()),
    ];
    let mut prompt = String::new();
    for (file, text) in parts {
        if !prompt.is_empty() {
            prompt.push_str("\n\n");
        }
        prompt.push_str(&fill(file, text, 1, |name| value(target, name))?);
    }
    Ok(prompt)
}

/// Checks that every `{{` of `text` opens one of [`VARIABLES`].
///
/// `first_line` is the line of the file `text` starts on, so that the error
/// points into the file and not into the body.
pub(super) fn check_variables(
    file: &str,
    text: &str,
    first_line: u64,
) -> Result<(), AgentFileError> {
    fill(file, text, first_line, |name| {
        VARIABLES.contains(&name).then_some("")
    })
    .map(drop)
}

/// The value of a variable for this target, `None` outside [`VARIABLES`].
fn value<'a>(target: &'a PromptTarget, name: &str) -> Option<&'a str> {
    match name {
        "dialect" => Some(target.dialect.as_str()),
        "driver" => Some(target.driver.as_str()),
        "environment" => Some(target.environment.as_str()),
        "recipient" => Some(target.recipient.as_str()),
        _ => None,
    }
}

/// Replaces every `{{name}}` of `text` by `lookup(name)`.
///
/// A `{{` with no closing `}}`, or whose name `lookup` does not know, refuses
/// the whole text: a placeholder left as is would reach the model as a
/// literal, and one replaced by nothing would silently drop a sentence's
/// subject.
fn fill<'a>(
    file: &str,
    text: &str,
    first_line: u64,
    lookup: impl Fn(&str) -> Option<&'a str>,
) -> Result<String, AgentFileError> {
    let mut filled = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((before, after)) = rest.split_once("{{") {
        filled.push_str(before);
        let resolved = after
            .split_once("}}")
            .and_then(|(name, tail)| lookup(name).map(|value| (value, tail)));
        let Some((value, tail)) = resolved else {
            let offset = text.len() - after.len();
            let line = text
                .get(..offset)
                .map_or(0, |head| head.matches('\n').count());
            return Err(AgentFileError::UnknownVariable {
                file: file.to_owned(),
                line: first_line.saturating_add(u64::try_from(line).unwrap_or(u64::MAX)),
            });
        };
        filled.push_str(value);
        rest = tail;
    }
    filled.push_str(rest);
    Ok(filled)
}
