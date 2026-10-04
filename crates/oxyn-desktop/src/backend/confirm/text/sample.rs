//! A sample's identity comes from the stored configuration used by its run.

use oxyn_catalog::CatalogPath;
use oxyn_core::{AiProviderConfig, ConnectionConfig, ExternalAgentConfig};
use oxyn_llm::Reach;

use super::{Confirmation, Severity, bounded, push_connection, visible};

/// The declaration actually serving the exchange, frozen before it starts.
/// Keeping it with the pending request prevents a later edit from disguising
/// the recipient of an already running provider or agent.
#[derive(Clone)]
pub(crate) enum SampleDestination {
    Provider(AiProviderConfig),
    Agent(ExternalAgentConfig),
}

/// Catalog names and the bounded read held by the backend, never display
/// labels supplied in a sample approval.
#[derive(Clone)]
pub(crate) struct SampleDescription {
    pub(crate) source: CatalogPath,
    pub(crate) destination: SampleDestination,
    pub(crate) reach: Reach,
    pub(crate) rows: u32,
}

/// Fails closed if the destination cannot be named. URL credentials, query
/// parameters, key references and agent environment values are never shown.
pub(crate) fn sample(
    connection: &ConnectionConfig,
    description: &SampleDescription,
    columns: &[String],
) -> Option<Confirmation> {
    let mut body = String::new();
    push_connection(&mut body, connection, connection.environment);
    match &description.destination {
        SampleDestination::Provider(provider) => {
            let url = tauri::Url::parse(&provider.base_url).ok()?;
            let host = url.host_str()?;
            // Show the entire host: cutting its suffix could conceal which
            // domain owns it. Its size is bounded by the stored declaration.
            body.push_str(&format!("\nProvider endpoint host: {}", visible(host)));
            if let Some(port) = url.port() {
                body.push_str(&format!(":{port}"));
            }
        }
        SampleDestination::Agent(agent) => {
            // Quote each word separately: a single argument containing spaces
            // must not look like several arguments to a different program.
            let command = std::iter::once(&agent.command)
                .chain(&agent.args)
                .map(|word| bounded(&crate::ipc::ai::shell_quote(word)))
                .collect::<Vec<_>>()
                .join(" ");
            body.push_str(&format!("\nAgent command: {command}"));
        }
    }
    body.push_str(match description.reach {
        Reach::Local => "\nReach: local — rows stay on this machine.",
        Reach::Remote => "\nReach: remote — rows leave this machine.",
        Reach::Unresolved => "\nReach: unresolved — rows may leave this machine.",
    });
    body.push_str(&format!(
        "\nRelation: {}\nColumns:",
        bounded(&description.source.to_string())
    ));
    for column in columns {
        body.push_str(&format!("\n• {}", bounded(column)));
    }
    body.push_str(&format!("\nMaximum rows: {}", description.rows));
    Some(Confirmation {
        title: "Send these rows to AI?".to_owned(),
        body,
        confirm: "Send rows",
        severity: Severity::Warning,
    })
}
