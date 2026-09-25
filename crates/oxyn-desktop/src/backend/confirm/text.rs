//! What a native confirmation says, composed from what the backend holds.
//!
//! Pure functions: no store, no dialog. Everything in a [`Confirmation`] comes
//! from the saved configuration or the command the executor holds, never from
//! an argument the webview sent — a script may choose what to have approved,
//! not what the dialog says about it ([ADR-0037 § 2](../../../../../docs/adr/0037-dialogue-natif-pour-les-confirmations-critiques.md)).
//!
//! Every string that is not one of this module's own literals goes through
//! [`visible`]: a connection name comes from a workspace file, a statement
//! from a script, a reason quotes the connection name (SECURITY, surface
//! d'entrée).

use std::collections::BTreeSet;
use std::fmt::Write as _;

use oxyn_core::{Command, ConnectionConfig, Environment};
use oxyn_driver::DriverMetadata;
use oxyn_exec::PendingCommand;

/// The label of the button that refuses. The confirming label must differ:
/// the plugin reports `true` for a custom button whose label **equals** the
/// confirming one, so two equal labels would make Cancel confirm
/// (RESEARCH-NOTES, `tauri-plugin-dialog` 2.7.3).
pub(crate) const CANCEL: &str = "Cancel";
/// Confirms a write, a DDL or a connection change on production.
pub(crate) const WRITE_TO_PRODUCTION: &str = "Write to production";
/// Confirms a change of environment or privacy tier.
pub(crate) const CHANGE_MARKING: &str = "Change marking";

/// A statement shown whole up to this many characters.
const STATEMENT_WHOLE: usize = 1_000;
/// Beyond, this many characters of its start, then as many of its end: a
/// harmless preamble must not hide on its own what follows.
const STATEMENT_EDGE: usize = 500;
/// Any other untrusted string — a name, a value, a reason.
const FIELD_MAX: usize = 200;

/// How the dialog looks. The host maps it to its own icon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Severity {
    Warning,
    Danger,
}

/// One native confirmation: its text and its confirming label.
///
/// The label is a constant of this module, never a parameter: see [`CANCEL`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Confirmation {
    pub(crate) title: String,
    pub(crate) body: String,
    pub(crate) confirm: &'static str,
    pub(crate) severity: Severity,
}

/// What an edit does to the stored secrets, by field name: the values never
/// reach this module.
#[derive(Clone, Default)]
pub(crate) struct SecretsShown {
    /// The secret fields the user retyped.
    pub(crate) retyped: BTreeSet<String>,
    /// The others are forgotten rather than kept: the edit moves the
    /// connection, and a secret does not follow it elsewhere.
    pub(crate) others_forgotten: bool,
}

/// The approval of a held command, on a connection retained as production.
///
/// `saved` is the configuration as stored, when there is one — absent for a
/// connection being created, whose configuration is the command itself.
pub(crate) fn held_command(
    held: &PendingCommand,
    environment: Environment,
    saved: Option<&ConnectionConfig>,
    metadata: Option<&DriverMetadata>,
    secrets: &SecretsShown,
) -> Confirmation {
    let mut body = String::new();
    let title = match (&held.command, saved) {
        (Command::CreateConnection { config }, _) => {
            push_connection(&mut body, config, environment);
            body.push_str("\nA new connection is saved with this marking.");
            "Save a production connection?"
        }
        (Command::UpdateConnection { config }, Some(saved)) => {
            push_connection(&mut body, saved, environment);
            body.push_str("\nEvery change of this edit:\n");
            push_edit(&mut body, saved, config, metadata, secrets);
            "Change a production connection?"
        }
        (Command::DeleteConnection { .. }, Some(saved)) => {
            push_connection(&mut body, saved, environment);
            body.push_str("\nThe saved connection and its keyring secrets are removed.");
            "Delete a production connection?"
        }
        (command, saved) => {
            if let Some(saved) = saved {
                push_connection(&mut body, saved, environment);
            } else {
                let _ = writeln!(body, "Environment: {}", shout(environment));
            }
            let _ = writeln!(
                body,
                "\n{} operation — {}",
                command.intent(),
                bounded(&held.reason)
            );
            if let Some(statement) = command.statement_text() {
                let _ = write!(
                    body,
                    "\nStatement ({} characters):\n{}",
                    statement.chars().count(),
                    cut_statement(statement)
                );
            }
            "Write to production?"
        }
    };
    Confirmation {
        title: title.to_owned(),
        body: body.trim_end().to_owned(),
        confirm: WRITE_TO_PRODUCTION,
        severity: Severity::Danger,
    }
}

/// A change of environment or privacy tier, confirmed before the edit is sent.
///
/// The edit is confirmed whole, so it is shown whole: every changed field,
/// and whether each secret is kept, retyped or forgotten.
pub(crate) fn marking_change(
    saved: &ConnectionConfig,
    edited: &ConnectionConfig,
    metadata: Option<&DriverMetadata>,
    secrets: &SecretsShown,
) -> Confirmation {
    let mut body = String::new();
    push_connection(&mut body, saved, saved.environment);
    body.push_str("\nEvery change of this edit:\n");
    push_edit(&mut body, saved, edited, metadata, secrets);
    Confirmation {
        title: "Change how this connection is marked?".to_owned(),
        body: body.trim_end().to_owned(),
        confirm: CHANGE_MARKING,
        severity: if saved.environment.is_production() || edited.environment.is_production() {
            Severity::Danger
        } else {
            Severity::Warning
        },
    }
}

/// Name, environment in full letters, and the non-secret address the
/// connection screen already shows: two connections of the same name differ
/// there.
fn push_connection(body: &mut String, config: &ConnectionConfig, environment: Environment) {
    let _ = writeln!(body, "Connection: “{}”", bounded(&config.name));
    let _ = writeln!(body, "Environment: {}", shout(environment));
    if let Some(address) = address(config) {
        let _ = writeln!(body, "Address: {}", bounded(&address));
    }
}

fn shout(environment: Environment) -> String {
    environment.as_str().to_uppercase()
}

/// A file path, or `host:port/database`, from the parameters — which hold no
/// secret: the store refuses one there.
fn address(config: &ConnectionConfig) -> Option<String> {
    let param = |key: &str| config.params.get(key).filter(|value| !value.is_empty());
    if let Some(path) = param("path") {
        return Some(path.clone());
    }
    let host = param("host")?;
    let mut address = host.clone();
    if let Some(port) = param("port") {
        address.push(':');
        address.push_str(port);
    }
    if let Some(database) = param("database") {
        address.push('/');
        address.push_str(database);
    }
    Some(address)
}

fn push_edit(
    body: &mut String,
    saved: &ConnectionConfig,
    edited: &ConnectionConfig,
    metadata: Option<&DriverMetadata>,
    secrets: &SecretsShown,
) {
    let before = body.len();
    let mut change = |label: &str, old: &str, new: &str| {
        if old != new {
            let _ = writeln!(
                body,
                "• {}: {} → {}",
                bounded(label),
                shown(old),
                shown(new)
            );
        }
    };
    change("Name", &saved.name, &edited.name);
    change(
        "Environment",
        &shout(saved.environment),
        &shout(edited.environment),
    );
    change(
        "Privacy tier",
        saved.privacy_tier.as_str(),
        edited.privacy_tier.as_str(),
    );
    change(
        "Read-only",
        yes_no(saved.read_only),
        yes_no(edited.read_only),
    );
    // Without the driver's metadata no key is known not to be secret: none is
    // shown. Closed by default, like the rest of this module.
    let secret = |key: &str| {
        metadata.is_none_or(|metadata| {
            metadata
                .field(key)
                .is_some_and(oxyn_driver::ConnectionField::is_secret)
        })
    };
    let label = |key: &str| {
        metadata
            .and_then(|metadata| metadata.field(key))
            .map_or_else(|| key.to_owned(), |field| field.label.clone())
    };
    let keys: Vec<&String> = saved
        .params
        .keys()
        .chain(
            edited
                .params
                .keys()
                .filter(|key| !saved.params.contains_key(*key)),
        )
        .collect();
    for key in keys {
        // Never a value under a secret field's key, even one a hostile file
        // put there: the dialog is one of I-03's channels.
        if secret(key) {
            continue;
        }
        let value = |config: &ConnectionConfig| config.params.get(key).cloned().unwrap_or_default();
        change(&label(key), &value(saved), &value(edited));
    }
    if let Some(metadata) = metadata {
        for field in metadata.secret_fields() {
            let state = if secrets.retyped.contains(&field.key) {
                "retyped"
            } else if secrets.others_forgotten {
                "forgotten"
            } else {
                "kept"
            };
            let _ = writeln!(body, "• {}: {state}", bounded(&field.label));
        }
    }
    if body.len() == before {
        body.push_str("• no field changes\n");
    }
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn shown(value: &str) -> String {
    if value.is_empty() {
        "(empty)".to_owned()
    } else {
        format!("“{}”", bounded(value))
    }
}

/// The statement whole up to [`STATEMENT_WHOLE`] characters; beyond, its
/// start and its end, and how much is left out between them. The cut is said,
/// not guessed.
fn cut_statement(statement: &str) -> String {
    let count = statement.chars().count();
    if count <= STATEMENT_WHOLE {
        return visible(statement);
    }
    let head: String = statement.chars().take(STATEMENT_EDGE).collect();
    let tail: String = statement.chars().skip(count - STATEMENT_EDGE).collect();
    format!(
        "{}\n… {} more characters …\n{}",
        visible(&head),
        count - 2 * STATEMENT_EDGE,
        visible(&tail)
    )
}

/// [`visible`], bounded to [`FIELD_MAX`] characters, the cut said.
fn bounded(text: &str) -> String {
    let count = text.chars().count();
    if count <= FIELD_MAX {
        return visible(text);
    }
    let head: String = text.chars().take(FIELD_MAX).collect();
    format!("{}… {} more characters", visible(&head), count - FIELD_MAX)
}

/// Control characters and invisible or direction-changing marks, written out
/// as `\u{…}`: a name holding a newline would draw a line of its own in the
/// dialog, and a right-to-left override would show `production` reversed.
///
/// Newline and tab keep a short visible form, statements being full of them.
pub(crate) fn visible(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '\n' => out.push('↵'),
            '\t' => out.push('⇥'),
            c if c.is_control() || is_invisible(c) => {
                let _ = write!(out, "\\u{{{:04X}}}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out
}

/// Format characters that draw nothing, or reorder what follows.
fn is_invisible(character: char) -> bool {
    matches!(
        character,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{17B4}'..='\u{17B5}'
            | '\u{180B}'..='\u{180F}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FE00}'..='\u{FE0F}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            | '\u{1BCA0}'..='\u{1BCA3}'
            | '\u{1D173}'..='\u{1D17A}'
            | '\u{E0000}'..='\u{E0FFF}'
    )
}

#[cfg(test)]
mod tests;
