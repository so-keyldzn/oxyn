//! Building the connection parameters without ever making them displayable.
//!
//! # The trap this module exists to close
//!
//! `sqlx` 0.9's [`PgConnectOptions`] derives `Debug` **and carries the password
//! in clear**. A `tracing::debug!("{options:?}")` added six months later
//! therefore writes a production password to a log file — that is exactly the
//! leak mode [I-03](../../../CLAUDE.md#i-03) describes, and it is invisible in
//! review.
//!
//! [`ConnectSpec`] wraps these options and **has no derived `Debug`**: its own
//! shows only the host, the port, the database, the user and the TLS mode. The
//! type does not serialize, does not display, and its only way out is
//! `ConnectSpec::options`, which returns a reference to pass to `sqlx`.
//!
//! Corollary: **no structure of this crate stores a bare
//! [`PgConnectOptions`].** A test checks it.
//!
//! # The second trap: the environment
//!
//! A driver receives its configuration, it does not go looking for it
//! ([DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md)). Yet **all** the
//! constructors of [`PgConnectOptions`] read `PGHOST`, `PGPORT`, `PGUSER`,
//! `PGDATABASE`, `PGPASSWORD`, `PGSSLMODE`, `PGAPPNAME` and `PGOPTIONS`; there
//! is none that does not.
//!
//! This module therefore systematically overwrites everything it can overwrite
//! — including the password, replaced by an empty string when the connection
//! declares none, which neutralizes `PGPASSWORD`. A `trust` or `peer`
//! authentication does not suffer from it: the server then asks for no
//! password.
//!
//! What **remains** out of reach is recorded in [`LEAKY_ENV`]: `sqlx` offers no
//! accessor to clear a certificate or server options inherited from the
//! environment.
//
// TODO(phase 1): ask upstream for a `PgConnectOptions` constructor that does
// not read the environment, or move the cleanup into `oxyn-desktop` before the
// runtime starts. Unblocks: removing `LEAKY_ENV`.

use std::fmt;
use std::time::Duration;

use oxyn_core::{ConnectionConfig, OxynError, Result};
use oxyn_driver::{Credentials, DriverMetadata};
use secrecy::ExposeSecret as _;
use sqlx::postgres::{PgConnectOptions, PgSslMode};

/// The environment variables `sqlx` reads and that this module cannot
/// neutralize, for lack of an accessor to clear them.
///
/// They carry no secret — they are certificate paths and server options —, but
/// they can change a connection's behavior without its configuration knowing.
/// Documented here rather than kept quiet.
pub const LEAKY_ENV: [&str; 4] = ["PGSSLROOTCERT", "PGSSLCERT", "PGSSLKEY", "PGOPTIONS"];

/// Application name announced to the server when the connection sets none.
///
/// Shows up in `pg_stat_activity`: that is what lets a DBA know a query comes
/// from Oxyn rather than from a batch job.
pub const DEFAULT_APPLICATION_NAME: &str = "oxyn";

/// Default port of the protocol.
pub const DEFAULT_PORT: u16 = 5432;

/// The parameters of a PostgreSQL connection, secret included, not displayable.
#[derive(Clone)]
pub struct ConnectSpec {
    options: PgConnectOptions,
    /// What can be shown, frozen at construction so that `Debug` has nothing to
    /// extract from the options — hence nothing to forget to mask.
    apercu: Apercu,
}

/// The showable part of a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Apercu {
    host: String,
    port: u16,
    database: String,
    user: String,
    sslmode: &'static str,
}

impl ConnectSpec {
    /// Builds the parameters from a configuration and resolved credentials.
    ///
    /// The configuration carries **no** secret: [`DriverMetadata::validate`]
    /// has already refused any parameter that looks like one. The password
    /// arrives through `credentials`, resolved from the system keychain by the
    /// caller.
    ///
    /// The recognized keys are `host`, `port`, `database`, `user`, `sslmode` and
    /// `application_name`, with their usual aliases. **Any other key becomes a
    /// server session option** (`-c key=value`): that is the meaning it has in a
    /// `libpq` URL, and the user wrote it explicitly.
    ///
    /// # Errors
    /// [`OxynError::Config`] if the configuration does not designate this
    /// driver, if a required parameter is missing, if the port is not a 16-bit
    /// integer, or if `sslmode` does not name a known mode.
    pub fn from_config(
        metadata: &DriverMetadata,
        config: &ConnectionConfig,
        credentials: &Credentials,
    ) -> Result<Self> {
        metadata.validate(config)?;

        let mut host = "localhost".to_owned();
        let mut port = metadata.default_port.unwrap_or(DEFAULT_PORT);
        let mut database = String::new();
        let mut user = String::new();
        let mut sslmode = PgSslMode::Prefer;
        let mut sslmode_nom = "prefer";
        let mut application_name = DEFAULT_APPLICATION_NAME.to_owned();
        let mut extras: Vec<(String, String)> = Vec::new();

        for (cle, valeur) in &config.params {
            match cle.trim().to_ascii_lowercase().as_str() {
                "host" | "hostname" | "server" => host = valeur.trim().to_owned(),
                "port" => {
                    port = valeur.trim().parse::<u16>().map_err(|_| {
                        OxynError::Config(
                            "parameter `port` expects an integer between 1 and 65535".to_owned(),
                        )
                    })?;
                }
                "database" | "dbname" | "db" => database = valeur.trim().to_owned(),
                "user" | "username" => user = valeur.trim().to_owned(),
                "sslmode" | "ssl_mode" => {
                    let (mode, nom) = parse_ssl_mode(valeur.trim())?;
                    sslmode = mode;
                    sslmode_nom = nom;
                }
                "application_name" => {
                    let demande = valeur.trim();
                    if !demande.is_empty() {
                        application_name = demande.to_owned();
                    }
                }
                _ => extras.push((cle.clone(), valeur.clone())),
            }
        }

        if host.is_empty() {
            return Err(OxynError::Config("parameter `host` is required".to_owned()));
        }
        if database.is_empty() {
            return Err(OxynError::Config(
                "parameter `database` is required".to_owned(),
            ));
        }
        if user.is_empty() {
            return Err(OxynError::Config("parameter `user` is required".to_owned()));
        }

        // `new_without_pgpass` rather than `new`: the latter also reads the
        // `~/.pgpass` file, which a driver has no business doing. Whatever this
        // one may have read from the environment is overwritten right after.
        let mut options = PgConnectOptions::new_without_pgpass()
            .host(&host)
            .port(port)
            .database(&database)
            .username(&user)
            .ssl_mode(sslmode)
            .application_name(&application_name);

        // Always set the password, even empty: that is what prevents a
        // `PGPASSWORD` lying around in the environment from applying to a
        // connection that declares none.
        options = match credentials.password() {
            Some(secret) => options.password(secret.expose_secret()),
            None => options.password(""),
        };

        if !extras.is_empty() {
            options = options.options(extras.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        }
        // A session setting the driver **imposes**, and declares here.
        //
        // `oxyn-query`'s splitter and classifier read `'a\'` as a complete
        // string, which is only true with `standard_conforming_strings = on`. A
        // server set to `off` — `ALTER DATABASE … SET`, legacy of an old
        // application — would read what follows as code: a `DELETE` classified
        // as a read would go out without the confirmation that names the
        // connection ([I-02](../../../CLAUDE.md#i-02)).
        //
        // Set as a startup parameter, **after** the user's options: a later
        // `-c` wins, and a connection parameter takes precedence over
        // `ALTER DATABASE` and `ALTER ROLE`. It changes the meaning of no query
        // written for an up-to-date server — it is PostgreSQL's default value. A
        // server that refused the parameter refuses the connection, which shows;
        // the cursor sets it again after any execution that could have changed
        // it (`SQL_RESET_AFTER_WRITE`).
        options = options.options([("standard_conforming_strings", "on")]);

        Ok(Self {
            options,
            apercu: Apercu {
                host,
                port,
                database,
                user,
                sslmode: sslmode_nom,
            },
        })
    }

    /// The parameters to hand to `sqlx`.
    ///
    /// The name is neutral, but the use is not: what comes out of here carries
    /// the password. The only legitimate call is opening a connection.
    #[must_use]
    pub(crate) fn options(&self) -> &PgConnectOptions {
        &self.options
    }

    /// The host, as the configuration gave it.
    #[must_use]
    pub fn host(&self) -> &str {
        &self.apercu.host
    }

    /// The port.
    #[must_use]
    pub const fn port(&self) -> u16 {
        self.apercu.port
    }

    /// The target database.
    #[must_use]
    pub fn database(&self) -> &str {
        &self.apercu.database
    }

    /// The user.
    #[must_use]
    pub fn user(&self) -> &str {
        &self.apercu.user
    }

    /// The requested TLS mode, under its `libpq` name.
    #[must_use]
    pub const fn ssl_mode(&self) -> &'static str {
        self.apercu.sslmode
    }
}

impl fmt::Debug for ConnectSpec {
    /// Written by hand: this type carries a secret, and a derived `Debug` is the
    /// most frequent leak mode because it is invisible in review
    /// ([I-03](../../../CLAUDE.md#i-03)).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectSpec")
            .field("host", &self.apercu.host)
            .field("port", &self.apercu.port)
            .field("database", &self.apercu.database)
            .field("user", &self.apercu.user)
            .field("sslmode", &self.apercu.sslmode)
            .field("password", &"<redacted>")
            .finish()
    }
}

/// Translates a `libpq` `sslmode` into its `sqlx` equivalent.
///
/// The six names are `libpq`'s, because that is what the user has in their
/// existing connection files.
fn parse_ssl_mode(valeur: &str) -> Result<(PgSslMode, &'static str)> {
    let couple = match valeur.to_ascii_lowercase().as_str() {
        "disable" => (PgSslMode::Disable, "disable"),
        "allow" => (PgSslMode::Allow, "allow"),
        "prefer" | "" => (PgSslMode::Prefer, "prefer"),
        "require" => (PgSslMode::Require, "require"),
        "verify-ca" | "verify_ca" => (PgSslMode::VerifyCa, "verify-ca"),
        "verify-full" | "verify_full" => (PgSslMode::VerifyFull, "verify-full"),
        _ => {
            return Err(OxynError::Config(
                "parameter `sslmode` expects: disable, allow, prefer, require, \
                 verify-ca or verify-full"
                    .to_owned(),
            ));
        }
    };
    Ok(couple)
}

/// Timeout for acquiring a connection from the pool.
///
/// Short on purpose: beyond it, the user prefers a clear error to an interface
/// that seems frozen ([PERFORMANCE](../../../docs/PERFORMANCE.md)).
pub const ACQUIRE_TIMEOUT: Duration = Duration::from_secs(15);

/// Number of simultaneous connections of a session.
///
/// An Oxyn session is not an application server: a few tabs, a catalog tree,
/// and cancellation — which does **not** borrow from the pool (see
/// [`crate::session`]). Four is enough, and leaves the database alone.
pub const MAX_CONNECTIONS: u32 = 4;

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use super::*;
    use crate::driver::postgres_metadata;

    const MOT_DE_PASSE: &str = "hunter2";

    fn config() -> ConnectionConfig {
        ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "db.interne")
            .with_param("port", "6543")
            .with_param("database", "caisse")
            .with_param("user", "lecture")
    }

    #[test]
    fn debug_never_shows_the_password() {
        // I-03, checkable corollary. This test is the module's reason to exist.
        let spec = ConnectSpec::from_config(
            &postgres_metadata(),
            &config(),
            &Credentials::new().with_password(MOT_DE_PASSE),
        )
        .expect("complete configuration");

        let rendu = format!("{spec:?}");
        assert!(!rendu.contains(MOT_DE_PASSE), "leak: {rendu}");
        assert!(rendu.contains("<redacted>"), "{rendu}");

        // What remains must stay useful for diagnostics.
        assert!(rendu.contains("db.interne"), "{rendu}");
        assert!(rendu.contains("6543"), "{rendu}");
        assert!(rendu.contains("lecture"), "{rendu}");
    }

    #[test]
    fn recognized_parameters_reach_sqlx() {
        let spec = ConnectSpec::from_config(&postgres_metadata(), &config(), &Credentials::new())
            .expect("complete configuration");
        assert_eq!(spec.host(), "db.interne");
        assert_eq!(spec.port(), 6543);
        assert_eq!(spec.database(), "caisse");
        assert_eq!(spec.user(), "lecture");
        assert_eq!(spec.ssl_mode(), "prefer");
    }

    #[test]
    fn the_default_port_is_the_protocol_s() {
        let sans_port = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "localhost")
            .with_param("database", "caisse")
            .with_param("user", "lecture");
        let spec = ConnectSpec::from_config(&postgres_metadata(), &sans_port, &Credentials::new())
            .expect("complete configuration");
        assert_eq!(spec.port(), DEFAULT_PORT);
    }

    #[test]
    fn an_unreadable_port_is_refused_with_a_message_that_says_what_to_do() {
        let mauvais = config().with_param("port", "cinq-mille");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &mauvais, &Credentials::new())
            .expect_err("refusal expected");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(erreur.to_string().contains("65535"), "{erreur}");
    }

    #[test]
    fn an_unknown_sslmode_is_refused_rather_than_silently_downgraded() {
        // Downgrading it to `prefer` would open a connection in clear where the
        // user asked for encryption.
        let mauvais = config().with_param("sslmode", "peut-etre");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &mauvais, &Credentials::new())
            .expect_err("refusal expected");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(erreur.to_string().contains("verify-full"), "{erreur}");
    }

    #[test]
    fn the_six_libpq_tls_modes_are_accepted() {
        for nom in [
            "disable",
            "allow",
            "prefer",
            "require",
            "verify-ca",
            "verify-full",
        ] {
            let avec = config().with_param("sslmode", nom);
            let spec = ConnectSpec::from_config(&postgres_metadata(), &avec, &Credentials::new())
                .expect("known mode");
            assert_eq!(spec.ssl_mode(), nom);
        }
    }

    #[test]
    fn a_secret_stored_in_the_parameters_is_refused() {
        // The configuration is persisted: a secret has no place in it (I-03).
        // `DriverMetadata::validate` is what refuses, and this module calls it.
        let fuite = config().with_param("password", MOT_DE_PASSE);
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &fuite, &Credentials::new())
            .expect_err("refusal expected");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
        assert!(!erreur.to_string().contains(MOT_DE_PASSE), "{erreur}");
    }

    #[test]
    fn a_missing_required_field_is_named() {
        let sans_base = ConnectionConfig::new("caisse", DriverId::postgres())
            .with_param("host", "localhost")
            .with_param("user", "lecture");
        let erreur =
            ConnectSpec::from_config(&postgres_metadata(), &sans_base, &Credentials::new())
                .expect_err("refusal expected");
        assert!(erreur.to_string().contains("database"), "{erreur}");
    }

    #[test]
    fn a_configuration_targeting_another_driver_is_refused() {
        let ailleurs = ConnectionConfig::new("fichier", DriverId::sqlite())
            .with_param("host", "localhost")
            .with_param("database", "caisse")
            .with_param("user", "lecture");
        let erreur = ConnectSpec::from_config(&postgres_metadata(), &ailleurs, &Credentials::new())
            .expect_err("refusal expected");
        assert!(matches!(erreur, OxynError::Config(_)), "{erreur:?}");
    }
}
