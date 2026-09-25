//! Le driver du **protocole** PostgreSQL — donc aussi de Redshift, TimescaleDB,
//! pgvector et Citus.
//!
//! Il n'y a pas de crate `oxyn-driver-redshift`, et il n'y en aura pas. Redshift
//! parle le protocole PostgreSQL ; ce qui l'en distingue est un jeu de
//! capacités, pas un décodeur de plus. Les ~30 systèmes de la vision se ramènent
//! ainsi à ~14 implémentations réelles
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`driver`] | [`PostgresDriver`] : formulaire de connexion, ouverture | DRIVER-CONTRACT |
//! | [`session`] | [`PostgresSession`] : exécuter, annuler, sonder | ARCHITECTURE §4.1 |
//! | [`cursor`] | [`PostgresCursor`] : le flux de lots et sa contre-pression | ADR-0002, I-06 |
//! | [`catalog`] | [`PostgresCatalog`] : introspection par `pg_catalog` | ARCHITECTURE §6 |
//! | [`variant`] | [`PostgresVariant`] : ce que la session sait faire | ADR-0003 |
//! | [`types`] | la correspondance PostgreSQL → Arrow, et ses pertes | DRIVER-CONTRACT §7 |
//! | [`decode`] | le binaire du serveur vers `RecordBatch` | I-09 |
//! | `numeric` | le décodage exact d'un `NUMERIC` (interne) | DRIVER-CONTRACT §7 |
//!
//! # Les cinq choix qui gouvernent cette crate
//!
//! **Les capacités s'évaluent à la connexion.** `version()` et `pg_extension`
//! sont interrogés une fois, et c'est là que `VECTOR_SEARCH` ou `TIME_SERIES`
//! s'activent — ou que Redshift perd `EXPLAIN ANALYZE`. Une capacité absente
//! signifie « je ne sais pas faire », jamais « je ferai semblant » : ce driver
//! ne déclare ni `TRANSACTIONS`, ni `MULTIPLE_STATEMENTS`, ni `BULK_LOAD`, parce
//! qu'il ne les implémente pas ([`variant::base_capabilities`] dit pourquoi).
//!
//! **L'annulation atteint le serveur.** Fermer un onglet détruit le curseur, ce
//! qui annule son jeton, ce qui fait émettre `pg_cancel_backend` depuis une
//! **seconde** connexion — pas depuis le bassin, qui est justement saturé quand
//! on veut annuler. Un futur abandonné ne libère ni la connexion ni le verrou
//! ([DRIVER-CONTRACT §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Rien n'est matérialisé.** Le curseur pousse des lots dimensionnés **en
//! octets** dans un canal d'une place : la tâche ne décode le lot suivant que si
//! le précédent a été pris. Un `SELECT *` sur 500 Go ne fait donc pas gonfler la
//! mémoire ([I-06](../../../CLAUDE.md#i-06)).
//!
//! **Unknown wire types keep their bytes and PostgreSQL type metadata.**
//! The grid formats Arrow binary values explicitly, without guessing text from
//! bytes that happen to be valid UTF-8. Preview composition requests server text
//! for internal types and OID aliases; user SQL is never rewritten or retried.
//!
//! **Le SQL de l'utilisateur part tel quel ; celui d'Oxyn ne concatène rien.**
//! Le texte d'une requête n'est ni analysé ni réécrit : c'est la fonctionnalité
//! d'un outil professionnel. Les requêtes que le driver compose — introspection,
//! annulation — sont des littéraux à paramètres liés
//! ([I-10](../../../CLAUDE.md#i-10)).
//!
//! # Exemple
//!
//! ```no_run
//! use oxyn_core::prelude::*;
//! use oxyn_driver::{Credentials, Cursor as _, Driver as _, Session as _};
//! use oxyn_driver_postgres::PostgresDriver;
//!
//! # async fn exemple() -> Result<()> {
//! let driver = PostgresDriver::new();
//!
//! // Ce qui est persisté ne porte aucun secret.
//! let connexion = ConnectionConfig::new("caisse", DriverId::postgres())
//!     .with_param("host", "interne.example")
//!     .with_param("database", "caisse")
//!     .with_param("user", "lecture")
//!     .with_environment(Environment::Development);
//!
//! // Le mot de passe arrive du trousseau du système, à part.
//! let identifiants = Credentials::new().with_password("résolu-au-dernier-moment");
//!
//! let jeton = CancelToken::new();
//! let session = driver.connect(&connexion, &identifiants, &jeton).await?;
//!
//! // Les capacités sont celles de *cette* session : pgvector installé ici ne
//! // dit rien de la base voisine.
//! if session.capabilities().contains(Capabilities::VECTOR_SEARCH) {
//!     // … la surface de recherche vectorielle a lieu d'exister.
//! }
//!
//! let mut curseur = session
//!     .execute(
//!         ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "SELECT 1")
//!             .with_intent(StatementIntent::Read),
//!         &jeton,
//!     )
//!     .await?;
//!
//! // Le schéma est connu avant la première ligne.
//! assert_eq!(curseur.schema().fields().len(), 1);
//! while let Some(_lot) = curseur.next_batch().await? {}
//! # Ok(())
//! # }
//! ```
//!
//! # Tests d'intégration
//!
//! Tout ce qui demande un serveur est marqué `#[ignore]`. Pour les lancer :
//!
//! ```sh
//! docker run --rm -d -p 5433:5432 -e POSTGRES_PASSWORD=oxyn --name oxyn-pg postgres:17
//! OXYN_PG_TEST_URL='postgres://postgres:oxyn@localhost:5433/postgres' \
//!   cargo test -p oxyn-driver-postgres -- --ignored --test-threads=1
//! ```
//!
//! Les tests d'annulation demandent en plus un serveur qui accepte
//! `pg_cancel_backend` sur ses propres processus, ce qui est le cas par défaut.

mod cancel;
pub mod catalog;
pub mod cursor;
pub mod decode;
pub mod driver;
mod lease;
mod preview;
pub mod session;
mod transaction_text;
pub mod types;
pub mod variant;

pub(crate) mod error;
pub(crate) mod numeric;
pub(crate) mod options;

/// Les tests de `Session::cancel` qui demandent un serveur. Tous `#[ignore]`.
#[cfg(test)]
mod cancel_tests;
/// L'état de session qu'une connexion emporte au bassin. Tous `#[ignore]`.
#[cfg(test)]
mod context_tests;
#[cfg(test)]
mod ddl_tests;
#[cfg(test)]
mod definition_tests;
/// Les tests qui demandent un serveur. Tous `#[ignore]`.
#[cfg(test)]
mod integration;

pub use catalog::{PostgresCatalog, logical_type};
pub use cursor::PostgresCursor;
pub use decode::{BatchAssembler, DecodeError};
pub use driver::{PostgresDriver, postgres_metadata};
pub use error::PostgresError;
pub use options::{ConnectSpec, DEFAULT_APPLICATION_NAME, DEFAULT_PORT, LEAKY_ENV};
pub use session::PostgresSession;
pub use types::{META_FALLBACK, META_PG_TYPE, PgDecoding, decoding_for, schema_for};
pub use variant::{PostgresFlavor, PostgresVariant, base_capabilities, driver_capabilities};

#[cfg(test)]
mod tests {
    use oxyn_core::{Capabilities, DriverId, QueryLanguage, SqlDialect};
    use oxyn_driver::DriverRegistry;
    use std::sync::Arc;

    use crate::PostgresDriver;

    /// Le driver s'enregistre, se déclare, et sa déclaration tient debout.
    ///
    /// C'est le seul trajet de la crate qui ne demande pas de serveur, et c'est
    /// celui qu'`oxyn-desktop` empruntera au démarrage.
    #[test]
    fn le_driver_s_enregistre_et_annonce_ce_qu_il_sait_faire() {
        let mut registre = DriverRegistry::new();
        registre
            .register(Arc::new(PostgresDriver::new()))
            .expect("le driver est cohérent");

        let driver = registre
            .require(&DriverId::postgres())
            .expect("il vient d'être enregistré");

        assert_eq!(driver.metadata().display_name, "PostgreSQL");
        assert!(
            driver
                .capabilities()
                .supports_language(QueryLanguage::Sql(SqlDialect::Postgres))
        );
        assert!(
            driver
                .capabilities()
                .contains(Capabilities::SERVER_SIDE_CANCEL),
            "sans ce drapeau, le bouton « Annuler » ne peut rien promettre"
        );
    }

    /// Redshift n'a pas de crate à lui : c'est le même driver, un dialecte
    /// différent (ADR-0003).
    #[test]
    fn redshift_passe_par_ce_driver_et_pas_par_un_autre() {
        use crate::PostgresVariant;

        let redshift = PostgresVariant::detect(
            "PostgreSQL 8.0.2 on i686-pc-linux-gnu, Redshift 1.0.75008",
            "8.0.2",
            Vec::new(),
        );
        assert_eq!(redshift.dialect(), SqlDialect::Redshift);
        assert!(redshift.capabilities().contains(Capabilities::SQL));
        assert!(
            !redshift
                .capabilities()
                .contains(Capabilities::EXPLAIN_ANALYZE),
            "un panneau « Plan d'exécution » ne doit pas exister ici"
        );
    }
}
