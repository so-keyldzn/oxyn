//! Le contrat que les quatorze drivers d'Oxyn respectent.
//!
//! Cette crate ne parle à aucune base de données. Elle définit **ce qu'un driver
//! doit être** — trois traits, ses métadonnées, son registre, sa chaîne de
//! connexion — et rien d'autre. Chaque erreur commise ici se paie autant de fois
//! qu'il y a de drivers : les ~30 systèmes de la vision se ramènent à ~14
//! implémentations réelles, un driver par **protocole** et non par produit
//! ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
//!
//! Le contrat de fond fait autorité dans
//! [DRIVER-CONTRACT](../../../docs/DRIVER-CONTRACT.md) ; il n'est pas recopié
//! ici.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`traits`] | [`Driver`], [`Session`], [`Cursor`] | ARCHITECTURE §4.1 |
//! | [`metadata`] | ce qu'un driver dit de lui-même, et son formulaire | UX-SPEC |
//! | [`registry`] | [`DriverRegistry`] : enregistrer, retrouver, ordonner | ARCHITECTURE §4 |
//! | [`credentials`] | [`Credentials`] : les secrets résolus, et rien d'autre | SECURITY, I-03 |
//! | [`dsn`] | l'URL de connexion, et le mot de passe qui n'en sort jamais | SECURITY, I-03 |
//!
//! # Les trois choix qui gouvernent cette crate
//!
//! **Rien n'est simulé.** Une session déclare ses capacités et refuse ce qu'elle
//! ne sait pas faire. [`Session::begin`] par défaut échoue de deux façons
//! différentes selon que la session déclare ou non
//! [`Capabilities::TRANSACTIONS`](oxyn_core::Capabilities::TRANSACTIONS) — mais
//! elle ne réussit jamais sans rien ouvrir. Laisser croire qu'un `ROLLBACK` a
//! annulé une écriture coûte plus cher que de ne pas savoir le faire.
//!
//! **Le secret ne traverse pas la configuration.** [`ConnectionConfig`](oxyn_core::ConnectionConfig)
//! ne porte qu'une référence ; les identifiants arrivent au driver par
//! [`Credentials`], et l'URL complète n'existe que le temps d'un appel à
//! [`Dsn::expose`]. [`DriverMetadata::validate`] et [`DsnBuilder::from_config`]
//! **refusent** un paramètre qui porterait un mot de passe, plutôt que de le
//! transporter (I-03).
//!
//! **L'interface est engendrée, pas codée par driver.** Un driver décrit ses
//! [`ConnectionField`] ; personne n'écrit un écran de connexion par protocole.
//! C'est ce qui rend le quatorzième driver aussi peu coûteux que le troisième.
//!
//! # Exemple : du formulaire à l'URL
//!
//! ```
//! use oxyn_core::{ConnectionConfig, DriverId};
//! use oxyn_driver::{ConnectionField, DriverFamily, DriverMetadata, DsnBuilder, FieldKind};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! // Ce que le driver déclare une fois pour toutes.
//! let metadonnees =
//!     DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
//!         .with_default_port(5432)
//!         .with_fields([
//!             ConnectionField::new("host", "Hôte", FieldKind::Text).required(),
//!             ConnectionField::new("user", "Utilisateur", FieldKind::Text).required(),
//!             ConnectionField::new("database", "Base", FieldKind::Text).required(),
//!             ConnectionField::new("password", "Mot de passe", FieldKind::Password),
//!         ]);
//!
//! // Ce que l'utilisateur saisit — le mot de passe part au trousseau, pas ici.
//! let connexion = ConnectionConfig::new("caisse", DriverId::postgres())
//!     .with_param("host", "interne.example")
//!     .with_param("user", "app")
//!     .with_param("database", "caisse");
//! metadonnees.validate(&connexion)?;
//!
//! // Et l'URL, dont le mot de passe n'entre qu'à `expose`.
//! let dsn = DsnBuilder::for_driver(&metadonnees, &connexion)?
//!     .with_password("resolu-depuis-le-trousseau")
//!     .build()?;
//!
//! assert!(dsn.has_password());
//! assert!(!dsn.to_string().contains("resolu-depuis-le-trousseau"));
//! assert!(dsn.to_string().ends_with(":5432/caisse"));
//! # Ok(())
//! # }
//! ```

pub mod context;
pub mod credentials;
pub mod dsn;
pub mod metadata;
pub mod registry;
pub mod traits;

pub use context::SessionContext;
pub use credentials::Credentials;
pub use dsn::{Dsn, DsnBuilder, DsnError, DsnParts, ParsedDsn};
pub use metadata::{ConnectionField, DriverFamily, DriverMetadata, FieldKind, looks_like_secret};
pub use registry::DriverRegistry;
pub use traits::{Cursor, Driver, Session};

/// Réexports de `secrecy`, parce qu'ils font partie de la signature de
/// [`Driver::connect`] et de [`Dsn::expose`].
///
/// Un driver doit pouvoir nommer [`SecretString`] et exposer son contenu au
/// moment de le remettre à son client. Le lui faire faire en ajoutant `secrecy`
/// à son propre `Cargo.toml` inviterait à une divergence de version entre deux
/// crates du workspace — et deux types `SecretString` incompatibles se
/// diagnostiquent très mal.
pub use secrecy::{ExposeSecret, SecretString};

/// Ce qu'on importe d'un coup quand on écrit un driver.
///
/// Y compris le vocabulaire du domaine : un driver a besoin de
/// [`Capabilities`](oxyn_core::Capabilities), de
/// [`ExecRequest`](oxyn_core::ExecRequest) et d'[`OxynError`](oxyn_core::OxynError)
/// à chaque fichier, et les importer un par un finit par être contourné.
///
/// ```
/// use oxyn_driver::prelude::*;
/// ```
pub mod prelude {
    pub use oxyn_catalog::CatalogProvider;
    pub use oxyn_core::{
        CancelToken, Capabilities, ConnectionConfig, DriverId, ErrorClass, ExecLimits, ExecRequest,
        ExecStats, OxynError, QueryLanguage, Result, SessionId, SqlDialect, StatementHandle,
    };

    pub use crate::credentials::Credentials;
    pub use crate::dsn::{Dsn, DsnBuilder, DsnError};
    pub use crate::metadata::{ConnectionField, DriverFamily, DriverMetadata, FieldKind};
    pub use crate::registry::DriverRegistry;
    pub use crate::traits::{Cursor, Driver, Session};
    pub use crate::{ExposeSecret, SecretString};
}

#[cfg(test)]
mod tests {
    use oxyn_core::DriverId;

    use crate::{
        ConnectionField, DriverFamily, DriverMetadata, DsnBuilder, ExposeSecret, FieldKind,
        ParsedDsn,
    };

    fn metadonnees() -> DriverMetadata {
        DriverMetadata::new(DriverId::postgres(), "PostgreSQL", DriverFamily::Relational)
            .with_default_port(5432)
            .with_fields([
                ConnectionField::new("host", "Hôte", FieldKind::Text)
                    .required()
                    .with_default("localhost"),
                ConnectionField::new("user", "Utilisateur", FieldKind::Text).required(),
                ConnectionField::new("database", "Base", FieldKind::Text).required(),
                ConnectionField::new("password", "Mot de passe", FieldKind::Password),
                ConnectionField::new(
                    "sslmode",
                    "Mode TLS",
                    FieldKind::Choice(vec!["disable".into(), "require".into()]),
                )
                .with_default("require"),
            ])
    }

    /// Le trajet complet de la crate, sur le scénario qui la met vraiment en
    /// jeu : l'utilisateur colle une URL, Oxyn en tire une connexion
    /// persistable et un secret séparé, puis reconstruit l'URL pour se
    /// connecter.
    #[test]
    fn le_trajet_complet_d_une_url_collee() {
        let mot_de_passe = "hunter2";
        let mut collee = String::from("postgres://app:");
        collee.push_str(mot_de_passe);
        collee.push_str("@interne.example:6432/caisse?sslmode=require");

        let relu = ParsedDsn::parse(&collee).expect("URL valide");
        let (parts, identifiants) = relu.into_parts();

        // 1. Ce qui est persisté ne porte aucun secret, et le driver l'accepte.
        let connexion = parts.to_config("caisse", DriverId::postgres());
        metadonnees()
            .validate(&connexion)
            .expect("la configuration relue est complète et sans secret");
        for (cle, valeur) in &connexion.params {
            assert!(
                valeur != mot_de_passe,
                "le paramètre `{cle}` porte le mot de passe"
            );
        }

        // 2. La connexion vaut production tant que personne n'a dit le
        //    contraire : une URL ne dit rien de l'environnement.
        assert!(connexion.is_production());

        // 3. L'URL se reconstruit, mot de passe réinjecté au dernier moment.
        let dsn = DsnBuilder::for_driver(&metadonnees(), &connexion)
            .expect("aucun secret dans les paramètres")
            .with_credentials(&identifiants)
            .build()
            .expect("URL valide");

        assert!(!dsn.to_string().contains(mot_de_passe), "{dsn}");
        let complete = dsn.expose().expect("l'URL a une autorité");
        assert_eq!(complete.expose_secret(), collee);
    }
}
