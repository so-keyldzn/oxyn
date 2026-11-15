//! Direct passwordless local targets; unresolved transports fail closed.
use super::ToolVersion;
use crate::{Result, TransferError, TransferSource};
use oxyn_core::{NativeTransferFormat, NativeTransferOperation, NativeTransferRequest};
use std::net::IpAddr;
use std::path::Path;

/// A destination-bound native target with no password or ambient authentication.
#[derive(Clone)]
pub struct Target {
    /// Literal loopback address; no DNS or multi-host selection exists.
    pub host: IpAddr,
    /// Explicit configured port.
    pub port: u16,
    /// Exact database name, never a URI.
    pub database: String,
    /// Exact configured account; no OS-user fallback.
    pub user: String,
    /// Actual server major, not a caller-selected tool version.
    pub server_major: u16,
}
fn text(source: &TransferSource, key: &str) -> Result<String> {
    let value = source.params.get(key).ok_or(TransferError::Unavailable(
        "Complete direct host, port, database and user settings are required",
    ))?;
    if value.is_empty()
        || value.len() > 1024
        || value != value.trim()
        || value.chars().any(char::is_control)
    {
        return Err(TransferError::Unavailable(
            "The native destination contains unsupported or ambiguous settings",
        ));
    }
    Ok(value.clone())
}
impl Target {
    /// Validate actual product, explicit transport and exact native destination.
    ///
    /// # Errors
    /// Unsupported product/platform/authentication mode, ambiguous settings or version.
    pub fn from_source(source: &TransferSource) -> Result<Self> {
        if !cfg!(any(target_os = "macos", target_os = "linux")) {
            return Err(TransferError::Unavailable(
                "Native process-tree cleanup is not implemented on this platform",
            ));
        }
        if source.driver.as_str() != "postgres" || source.product != "PostgreSQL" {
            return Err(TransferError::Unavailable(
                "A native transfer adapter is not available for this database product",
            ));
        }
        if source.params.keys().any(|key| {
            !matches!(
                key.as_str(),
                "host" | "port" | "database" | "user" | "sslmode" | "ssl_mode"
            )
        }) {
            return Err(TransferError::Unavailable(
                "Only direct PostgreSQL destination settings are supported; SSH and additional transport modes remain unavailable",
            ));
        }
        let modes: Vec<_> = [source.params.get("sslmode"), source.params.get("ssl_mode")]
            .into_iter()
            .flatten()
            .collect();
        if modes.is_empty() || modes.iter().any(|mode| mode.as_str() != "disable") {
            return Err(TransferError::Unavailable(
                "Native transfer currently requires explicitly disabled TLS on a direct local target; no transport downgrade is performed",
            ));
        }
        let host: IpAddr = text(source,"host")?.parse().map_err(|_| TransferError::Unavailable("Native passwordless transfer requires a literal loopback IP; hostnames and remote destinations remain unavailable"))?;
        if !host.is_loopback() {
            return Err(TransferError::Unavailable(
                "Native passwordless transfer is restricted to loopback targets",
            ));
        }
        let port = text(source, "port")?
            .parse()
            .ok()
            .filter(|port| *port != 0)
            .ok_or(TransferError::Unavailable(
                "An explicit valid destination port is required",
            ))?;
        if source.version.len() > 1024 {
            return Err(TransferError::Unavailable(
                "The original session reported an unreadable server version",
            ));
        }
        let server_major = source
            .version
            .split(|c: char| !c.is_ascii_digit())
            .find(|part| !part.is_empty())
            .and_then(|value| value.parse().ok())
            .ok_or(TransferError::Unavailable(
                "The original session has not reported a usable PostgreSQL version",
            ))?;
        Ok(Self {
            host,
            port,
            database: text(source, "database")?,
            user: text(source, "user")?,
            server_major,
        })
    }
    /// A tool older than the target major cannot export it safely.
    ///
    /// # Errors
    /// Tool/server major mismatch. Archive semantics remain separately unverified.
    pub fn check_tool(&self, tool: ToolVersion) -> Result<()> {
        if self.server_major > tool.major {
            return Err(TransferError::Unavailable(
                "Select a corrected PostgreSQL tool at least as new as the destination server major",
            ));
        }
        Ok(())
    }
    /// Fixed conninfo excludes passwords, service redirection and ambient transports.
    pub(crate) fn conninfo(&self, passfile: &Path, application: &str) -> Result<String> {
        let passfile = passfile.to_str().ok_or(TransferError::File)?;
        Ok([
            ("host", self.host.to_string()),
            ("hostaddr", self.host.to_string()),
            ("port", self.port.to_string()),
            ("dbname", self.database.clone()),
            ("user", self.user.clone()),
            ("sslmode", "disable".into()),
            ("gssencmode", "disable".into()),
            ("require_auth", "none".into()),
            ("passfile", passfile.into()),
            ("connect_timeout", "10".into()),
            ("application_name", application.into()),
            ("options", "-csearch_path=".into()),
        ]
        .into_iter()
        .map(|(key, value)| format!("{key}={}", quote(&value)))
        .collect::<Vec<_>>()
        .join(" "))
    }
    /// Adapter-owned argv: there is no shell, --create, connection URI or free flag.
    pub(crate) fn arguments(
        &self,
        request: &NativeTransferRequest,
        passfile: &Path,
        application: &str,
    ) -> Result<Vec<String>> {
        request
            .validate()
            .map_err(|_| TransferError::Unavailable("SQL script restoration is unavailable"))?;
        let mut args = vec![
            "--no-password".into(),
            format!("--dbname={}", self.conninfo(passfile, application)?),
        ];
        match request.operation {
            NativeTransferOperation::Export => {
                args.push(
                    match request.format {
                        NativeTransferFormat::CustomArchive => "--format=custom",
                        NativeTransferFormat::Sql => "--format=plain",
                        _ => {
                            return Err(TransferError::Unavailable(
                                "Unsupported native export format",
                            ));
                        }
                    }
                    .into(),
                );
                args.push("--quote-all-identifiers".into());
            }
            NativeTransferOperation::Restore => {
                args.push("--exit-on-error".into());
                args.push("--single-transaction".into());
            }
            _ => return Err(TransferError::Unavailable("Unsupported native operation")),
        }
        if request.no_owner {
            args.push("--no-owner".into());
        }
        if request.no_privileges {
            args.push("--no-privileges".into());
        }
        Ok(args)
    }
}
fn quote(value: &str) -> String {
    let mut quoted = String::with_capacity(value.len() + 2);
    quoted.push('\'');
    for c in value.chars() {
        if matches!(c, '\'' | '\\') {
            quoted.push('\\');
        }
        quoted.push(c);
    }
    quoted.push('\'');
    quoted
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Target")
            .field("server_major", &self.server_major)
            .finish_non_exhaustive()
    }
}
