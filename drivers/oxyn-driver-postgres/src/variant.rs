//! La variante du serveur, et ce qu'elle change aux capacités.
//!
//! Un driver par **protocole**, pas par produit ([ADR-0003]) : Redshift,
//! TimescaleDB, pgvector et Citus parlent tous le protocole PostgreSQL et
//! passent tous par cette crate. Ce qui les distingue n'est pas une
//! implémentation séparée, c'est un jeu de [`Capabilities`] différent — et c'est
//! précisément l'usage pour lequel le modèle « capacités par session » existe.
//!
//! La détection se fait **à la connexion**, une fois, à partir de `version()` et
//! de `pg_extension`. Elle ne se devine pas depuis la configuration : deux
//! connexions vers le même hôte peuvent viser deux bases dont l'une seule a
//! `pgvector` installé.
//!
//! # Ce que la détection ne fait pas
//!
//! Elle ne change **jamais** le comportement d'exécution — pas de réécriture de
//! requête, pas de contournement silencieux. Elle ne fait que déclarer ce que la
//! session sait faire, à charge pour l'interface et les agents de s'y conformer.
//! Un panneau « Plan d'exécution » n'existe pas face à un Redshift qui n'a pas
//! `EXPLAIN ANALYZE` ; il n'est pas grisé sans raison.
//!
//! [ADR-0003]: ../../../docs/adr/0003-driver-capabilities.md

use oxyn_core::{Capabilities, SqlDialect};

/// Nom de l'extension TimescaleDB dans `pg_extension`.
pub const EXT_TIMESCALEDB: &str = "timescaledb";
/// Nom de l'extension pgvector dans `pg_extension` — `vector`, pas `pgvector`.
pub const EXT_VECTOR: &str = "vector";
/// Nom de l'extension Citus.
pub const EXT_CITUS: &str = "citus";
/// Nom de l'extension PostGIS.
pub const EXT_POSTGIS: &str = "postgis";

/// Le produit derrière le protocole.
///
/// Volontairement court : une variante n'existe ici que si elle **change des
/// capacités**. Un produit qui se comporte comme PostgreSQL n'a pas à être
/// nommé, sans quoi cette énumération deviendrait la liste des produits que le
/// découpage par protocole cherche justement à éviter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum PostgresFlavor {
    /// PostgreSQL, ou un produit qui s'en distingue seulement par ses
    /// extensions.
    #[default]
    Postgres,
    /// Amazon Redshift : même protocole, grammaire et catalogue amputés.
    Redshift,
}

impl PostgresFlavor {
    /// Nom stable, pour l'audit et l'affichage.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Postgres => "PostgreSQL",
            Self::Redshift => "Amazon Redshift",
        }
    }

    /// Le dialecte SQL à annoncer à `oxyn-query`.
    ///
    /// Redshift parle le protocole de PostgreSQL sans en accepter la grammaire :
    /// c'est exactement la raison pour laquelle [`SqlDialect`] a une valeur
    /// distincte alors qu'il n'y a pas de crate de driver distincte.
    #[must_use]
    pub const fn dialect(&self) -> SqlDialect {
        match self {
            Self::Postgres => SqlDialect::Postgres,
            Self::Redshift => SqlDialect::Redshift,
        }
    }
}

impl std::fmt::Display for PostgresFlavor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ce qu'une session a appris de son serveur à la connexion.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PostgresVariant {
    /// Le produit reconnu dans la bannière.
    pub flavor: PostgresFlavor,
    /// `version()`, tel quel. Sert au diagnostic ; ne jamais l'analyser ailleurs
    /// qu'ici.
    pub banner: String,
    /// `current_setting('server_version')` : `17.2`, `16.4 (Debian …)`.
    pub server_version: String,
    /// Le numéro de version majeure, quand il se lit.
    pub major_version: Option<u32>,
    /// Les extensions installées **dans la base courante**, en minuscules.
    ///
    /// Vide si `pg_extension` n'est pas lisible : c'est le cas de Redshift, et
    /// d'un compte aux droits restreints. Une liste vide dit « je n'ai rien
    /// trouvé », pas « il n'y a rien ».
    pub extensions: Vec<String>,
}

impl PostgresVariant {
    /// Reconnaît la variante depuis la bannière du serveur et ses extensions.
    ///
    /// `extensions` est normalisé en minuscules et trié : la comparaison
    /// devient stable, et le rendu de diagnostic aussi.
    #[must_use]
    pub fn detect(banner: &str, server_version: &str, extensions: Vec<String>) -> Self {
        let flavor = if banner.to_ascii_lowercase().contains("redshift") {
            PostgresFlavor::Redshift
        } else {
            PostgresFlavor::Postgres
        };

        let mut normalisees: Vec<String> = extensions
            .into_iter()
            .map(|nom| nom.trim().to_ascii_lowercase())
            .filter(|nom| !nom.is_empty())
            .collect();
        normalisees.sort_unstable();
        normalisees.dedup();

        Self {
            flavor,
            banner: banner.to_owned(),
            server_version: server_version.to_owned(),
            major_version: parse_major(server_version),
            extensions: normalisees,
        }
    }

    /// Cette extension est-elle installée ?
    #[must_use]
    pub fn has_extension(&self, name: &str) -> bool {
        let cherche = name.to_ascii_lowercase();
        self.extensions.contains(&cherche)
    }

    /// Le nom de produit à afficher : `PostgreSQL`, `PostgreSQL + TimescaleDB`…
    ///
    /// Les extensions qui changent des capacités sont nommées, les autres non :
    /// l'utilisateur a besoin de savoir pourquoi une surface existe, pas de lire
    /// la liste de ce qui est installé.
    #[must_use]
    pub fn product(&self) -> String {
        let mut nom = self.flavor.as_str().to_owned();
        let mut marques = Vec::new();
        if self.has_extension(EXT_TIMESCALEDB) {
            marques.push("TimescaleDB");
        }
        if self.has_extension(EXT_VECTOR) {
            marques.push("pgvector");
        }
        if self.has_extension(EXT_CITUS) {
            marques.push("Citus");
        }
        if self.has_extension(EXT_POSTGIS) {
            marques.push("PostGIS");
        }
        if !marques.is_empty() {
            nom.push_str(" + ");
            nom.push_str(&marques.join(", "));
        }
        nom
    }

    /// Le dialecte SQL de cette session.
    #[must_use]
    pub const fn dialect(&self) -> SqlDialect {
        self.flavor.dialect()
    }

    /// Les capacités de cette session.
    ///
    /// Part du socle PostgreSQL ([`base_capabilities`]), retire ce que la
    /// variante n'a pas, ajoute ce que les extensions apportent. L'ordre compte :
    /// une extension n'a jamais à réactiver ce que la variante a retiré.
    #[must_use]
    pub fn capabilities(&self) -> Capabilities {
        let mut capacites = base_capabilities();

        if self.flavor == PostgresFlavor::Redshift {
            capacites.remove(redshift_missing());
        }

        if self.has_extension(EXT_TIMESCALEDB) {
            capacites.insert(Capabilities::TIME_SERIES);
        }
        if self.has_extension(EXT_VECTOR) {
            capacites.insert(Capabilities::VECTOR_SEARCH);
        }

        capacites
    }
}

/// Ce qu'une session PostgreSQL sait faire, avant toute variante.
///
/// # Quatre absences délibérées
///
/// **`MULTIPLE_STATEMENTS`** : ce driver n'emploie que le protocole étendu, qui
/// prépare **une** instruction par soumission. Le découpage d'un lot appartient
/// à `oxyn-query`. Déclarer la capacité obligerait à basculer sur le protocole
/// simple, où le schéma n'est connu qu'après la première ligne — donc à renoncer
/// au premier affichage sous 100 ms.
///
/// **`TRANSACTIONS`** et **`SAVEPOINTS`** : une transaction vit sur **une**
/// connexion, or une session Oxyn s'appuie sur un bassin et en emprunte une par
/// exécution. Déclarer la capacité sans épingler une connexion laisserait
/// l'utilisateur croire qu'un `ROLLBACK` a annulé son écriture, ce que le
/// contrat interdit explicitement. La transaction *interne* à une exécution en
/// lecture seule, elle, existe bien : c'est [`Capabilities::READ_ONLY_SESSION`].
///
/// **`NAMED_CURSORS`** et **`BULK_LOAD`** : `DECLARE`/`FETCH` et `COPY` ne sont
/// pas implémentés. Le flux passe par le portail du protocole étendu, qui suffit
/// à [`Capabilities::STREAMING`].
///
// TODO(phase 1) : épingler une connexion par session pour ouvrir TRANSACTIONS et
// SAVEPOINTS, et implémenter `COPY` pour BULK_LOAD. Débloque : l'édition de
// données avec prévisualisation du DML (IMPLEMENTATION-PLAN, phase 1).
#[must_use]
pub fn base_capabilities() -> Capabilities {
    Capabilities::SCHEMAS
        | Capabilities::TABLES
        | Capabilities::VIEWS
        | Capabilities::MATERIALIZED_VIEWS
        | Capabilities::INDEXES
        | Capabilities::CONSTRAINTS
        | Capabilities::FOREIGN_KEYS
        | Capabilities::ROUTINES
        | Capabilities::TRIGGERS
        | Capabilities::SEQUENCES
        | Capabilities::USER_TYPES
        | Capabilities::COMMENTS
        | Capabilities::PERMISSIONS
        | Capabilities::ROW_COUNT_ESTIMATE
        | Capabilities::PREPARED_STATEMENTS
        | Capabilities::SERVER_SIDE_CANCEL
        | Capabilities::STREAMING
        | Capabilities::AFFECTED_ROWS
        | Capabilities::EXPLAIN
        | Capabilities::EXPLAIN_ANALYZE
        | Capabilities::DDL
        | Capabilities::DML
        | Capabilities::GRANT_REVOKE
        | Capabilities::READ_ONLY_SESSION
        | Capabilities::SQL
        | Capabilities::RELATIONAL
        | Capabilities::FULL_TEXT_SEARCH
}

/// Ce que Redshift n'a pas, malgré le protocole commun.
///
/// * pas de vues matérialisées exposées par `relkind = 'm'` ;
/// * ni déclencheurs, ni séquences, ni types définis par l'utilisateur ;
/// * `EXPLAIN` existe, `EXPLAIN ANALYZE` non — le plan n'est jamais exécuté ;
/// * pas de `tsvector`, donc pas de recherche plein texte native.
#[must_use]
fn redshift_missing() -> Capabilities {
    Capabilities::MATERIALIZED_VIEWS
        | Capabilities::TRIGGERS
        | Capabilities::SEQUENCES
        | Capabilities::USER_TYPES
        | Capabilities::EXPLAIN_ANALYZE
        | Capabilities::FULL_TEXT_SEARCH
}

/// Les capacités que le **driver** annonce avant toute connexion.
///
/// Un plafond indicatif, pas une promesse : ce qui fait foi est
/// [`PostgresVariant::capabilities`], évalué une fois la session ouverte. On y
/// met donc l'union de ce qu'une session peut offrir au mieux.
#[must_use]
pub fn driver_capabilities() -> Capabilities {
    base_capabilities() | Capabilities::TIME_SERIES | Capabilities::VECTOR_SEARCH
}

/// Le numéro de version majeure d'une chaîne `server_version`.
///
/// PostgreSQL écrit `17.2`, `16.4 (Debian 16.4-1)`, parfois `9.6.24` — et
/// Redshift annonce `8.0.2`. On ne lit que le premier nombre, et on ne suppose
/// rien de la suite.
fn parse_major(server_version: &str) -> Option<u32> {
    let tete: String = server_version
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    tete.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    const BANNIERE_PG: &str =
        "PostgreSQL 17.2 on aarch64-apple-darwin, compiled by Apple clang 16.0.0, 64-bit";
    const BANNIERE_REDSHIFT: &str = "PostgreSQL 8.0.2 on i686-pc-linux-gnu, compiled by GCC gcc (GCC) 3.4.2, Redshift 1.0.75008";

    #[test]
    fn un_postgres_nu_declare_le_socle() {
        let variante = PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new());
        assert_eq!(variante.flavor, PostgresFlavor::Postgres);
        assert_eq!(variante.major_version, Some(17));
        assert_eq!(variante.dialect(), SqlDialect::Postgres);
        assert_eq!(variante.product(), "PostgreSQL");
        assert_eq!(variante.capabilities(), base_capabilities());
    }

    #[test]
    fn le_socle_ne_promet_pas_ce_que_le_driver_ne_fait_pas() {
        // « Ne pas savoir faire est une réponse acceptable ; laisser croire ne
        // l'est pas » : la session ne déclare ni transactions, ni multi-
        // instructions, ni COPY, parce qu'elle ne les implémente pas.
        let socle = base_capabilities();
        for absente in [
            Capabilities::TRANSACTIONS,
            Capabilities::SAVEPOINTS,
            Capabilities::MULTIPLE_STATEMENTS,
            Capabilities::NAMED_CURSORS,
            Capabilities::BULK_LOAD,
        ] {
            assert!(
                !socle.contains(absente),
                "{absente} ne doit pas être déclarée"
            );
        }
    }

    #[test]
    fn l_annulation_cote_serveur_est_declaree_parce_qu_elle_existe() {
        // `pg_cancel_backend` : sans ce drapeau, le bouton « Annuler » ne peut
        // pas prétendre couper la requête (DRIVER-CONTRACT §2).
        assert!(base_capabilities().contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(base_capabilities().contains(Capabilities::STREAMING));
    }

    #[test]
    fn pgvector_ouvre_la_recherche_vectorielle_sur_la_session_qui_l_a() {
        // Deux bases du même serveur peuvent différer : c'est tout l'objet du
        // modèle « capacités par session » (ADR-0003).
        let sans = PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new());
        let avec = PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["vector".to_owned()]);

        assert!(!sans.capabilities().contains(Capabilities::VECTOR_SEARCH));
        assert!(avec.capabilities().contains(Capabilities::VECTOR_SEARCH));
        assert_eq!(avec.product(), "PostgreSQL + pgvector");
    }

    #[test]
    fn timescaledb_ouvre_les_series_temporelles() {
        let variante = PostgresVariant::detect(
            BANNIERE_PG,
            "16.4",
            vec!["TimescaleDB".to_owned(), "plpgsql".to_owned()],
        );
        assert!(variante.has_extension(EXT_TIMESCALEDB), "casse insensible");
        assert!(variante.capabilities().contains(Capabilities::TIME_SERIES));
        assert_eq!(variante.product(), "PostgreSQL + TimescaleDB");
    }

    #[test]
    fn citus_se_nomme_sans_changer_de_capacite() {
        // Citus distribue les tables ; il n'ajoute aucune surface qu'Oxyn sache
        // exploiter aujourd'hui. Le dire est plus honnête que d'inventer un
        // drapeau.
        let variante = PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["citus".to_owned()]);
        assert_eq!(variante.capabilities(), base_capabilities());
        assert_eq!(variante.product(), "PostgreSQL + Citus");
    }

    #[test]
    fn redshift_se_reconnait_a_sa_banniere_et_perd_ce_qu_il_n_a_pas() {
        let variante = PostgresVariant::detect(BANNIERE_REDSHIFT, "8.0.2", Vec::new());
        assert_eq!(variante.flavor, PostgresFlavor::Redshift);
        assert_eq!(variante.dialect(), SqlDialect::Redshift);
        assert_eq!(variante.product(), "Amazon Redshift");

        let capacites = variante.capabilities();
        assert!(
            !capacites.contains(Capabilities::EXPLAIN_ANALYZE),
            "Redshift n'exécute pas le plan qu'il explique"
        );
        assert!(!capacites.contains(Capabilities::TRIGGERS));
        assert!(!capacites.contains(Capabilities::FULL_TEXT_SEARCH));

        // Ce qu'il a, il le garde.
        assert!(capacites.contains(Capabilities::SERVER_SIDE_CANCEL));
        assert!(capacites.contains(Capabilities::SQL));
        assert!(capacites.contains(Capabilities::TABLES));
    }

    #[test]
    fn une_extension_n_annule_pas_une_absence_de_la_variante() {
        // L'ordre du calcul compte : pgvector sur Redshift n'y remet pas
        // `EXPLAIN ANALYZE`.
        let variante = PostgresVariant::detect(
            BANNIERE_REDSHIFT,
            "8.0.2",
            vec!["vector".to_owned(), "timescaledb".to_owned()],
        );
        let capacites = variante.capabilities();
        assert!(capacites.contains(Capabilities::VECTOR_SEARCH));
        assert!(!capacites.contains(Capabilities::EXPLAIN_ANALYZE));
    }

    #[test]
    fn la_version_majeure_se_lit_sur_les_formes_reelles() {
        assert_eq!(parse_major("17.2"), Some(17));
        assert_eq!(parse_major("16.4 (Debian 16.4-1.pgdg120+1)"), Some(16));
        assert_eq!(parse_major("9.6.24"), Some(9));
        assert_eq!(parse_major(" 15beta1"), Some(15));
        assert_eq!(parse_major("inconnue"), None, "on ne devine pas");
        assert_eq!(parse_major(""), None);
    }

    #[test]
    fn les_extensions_sont_normalisees_et_dedupliquees() {
        let variante = PostgresVariant::detect(
            BANNIERE_PG,
            "17.2",
            vec![
                "Vector".to_owned(),
                " vector ".to_owned(),
                String::new(),
                "PostGIS".to_owned(),
            ],
        );
        assert_eq!(variante.extensions, ["postgis", "vector"]);
    }

    #[test]
    fn le_plafond_du_driver_couvre_toutes_les_sessions() {
        // Le driver annonce au mieux ; la session fait foi. Aucune session ne
        // doit pouvoir déclarer une capacité que le driver n'annonce pas, sans
        // quoi une surface apparaîtrait sans avoir jamais été prévue.
        let plafond = driver_capabilities();
        for variante in [
            PostgresVariant::detect(BANNIERE_PG, "17.2", Vec::new()),
            PostgresVariant::detect(BANNIERE_PG, "17.2", vec!["vector".to_owned()]),
            PostgresVariant::detect(BANNIERE_PG, "16.4", vec!["timescaledb".to_owned()]),
            PostgresVariant::detect(BANNIERE_REDSHIFT, "8.0.2", Vec::new()),
        ] {
            let session = variante.capabilities();
            assert!(
                plafond.contains(session),
                "{} déclare {} hors du plafond",
                variante.product(),
                session.difference(plafond)
            );
        }
    }
}
