//! Ce qu'une session sait faire (ADR-0003).
//!
//! Oxyn n'a pas d'abstraction nivelante : Redis n'a pas de schéma, Neo4j pas de
//! tables, Elasticsearch pas de SQL. Un dénominateur commun réduirait chaque base
//! à sa plus pauvre expression. À la place, chaque session **déclare** ce qu'elle
//! sait faire, et l'interface comme les agents s'y conforment.
//!
//! Deux conséquences qui se ratent facilement :
//!
//! * les capacités s'évaluent **par session**, pas par driver. La version du
//!   serveur, ses extensions et les droits du compte connecté changent ce qui
//!   est disponible : le même driver PostgreSQL parle à une base 12 sans `MERGE`
//!   et à une base 17 qui l'a ;
//! * **ne rien simuler.** Un driver qui émule les transactions par un
//!   enchaînement de requêtes laisse l'utilisateur croire qu'un `ROLLBACK` a
//!   annulé son écriture. Ne pas savoir faire est une réponse acceptable ;
//!   laisser croire ne l'est pas.
//!
//! Ce type vit dans `oxyn-core` et non dans `oxyn-driver` pour que l'interface
//! et les agents puissent lire des capacités sans dépendre d'une implémentation
//! de driver.

use std::fmt;

use bitflags::bitflags;
use serde::{Deserialize, Serialize};

use crate::error::{OxynError, Result};
use crate::query::QueryLanguage;

bitflags! {
    /// Capacités déclarées par une session (ou, par défaut, par un driver).
    ///
    /// Un drapeau absent signifie « je ne sais pas faire », jamais « je ferai
    /// semblant ».
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
    pub struct Capabilities: u64 {
        // ── Introspection du catalogue ──────────────────────────────────────
        /// La source expose des schémas nommés.
        const SCHEMAS              = 1 << 0;
        /// La source expose des tables ou collections.
        const TABLES               = 1 << 1;
        /// La source expose des vues.
        const VIEWS                = 1 << 2;
        /// La source expose des vues matérialisées.
        const MATERIALIZED_VIEWS   = 1 << 3;
        /// Les index sont introspectables.
        const INDEXES              = 1 << 4;
        /// Les contraintes (unicité, vérification) sont introspectables.
        const CONSTRAINTS          = 1 << 5;
        /// Les clés étrangères sont introspectables — c'est ce qui permet de
        /// construire un diagramme de relations sans le deviner.
        const FOREIGN_KEYS         = 1 << 6;
        /// Fonctions et procédures stockées sont introspectables.
        const ROUTINES             = 1 << 7;
        /// Les déclencheurs sont introspectables.
        const TRIGGERS             = 1 << 8;
        /// Les séquences sont introspectables.
        const SEQUENCES            = 1 << 9;
        /// Les types définis par l'utilisateur sont introspectables.
        const USER_TYPES           = 1 << 10;
        /// Les commentaires d'objets sont lisibles.
        const COMMENTS             = 1 << 11;
        /// Les droits et rôles sont introspectables.
        const PERMISSIONS          = 1 << 12;
        /// Une estimation du nombre de lignes est disponible sans compter.
        const ROW_COUNT_ESTIMATE   = 1 << 13;
        /// Foreign keys declared by other relations can be found from their target.
        const INCOMING_FOREIGN_KEYS = 1 << 14;
        /// Native object creation statements can be read for inspection.
        const OBJECT_DEFINITION     = 1 << 15;

        // ── Exécution ───────────────────────────────────────────────────────
        /// Transactions réelles, avec `ROLLBACK` qui annule vraiment.
        const TRANSACTIONS         = 1 << 16;
        /// Points de reprise à l'intérieur d'une transaction.
        const SAVEPOINTS           = 1 << 17;
        /// Requêtes préparées avec paramètres liés côté serveur.
        const PREPARED_STATEMENTS  = 1 << 18;
        /// Curseurs nommés, pour parcourir un résultat sans le matérialiser.
        const NAMED_CURSORS        = 1 << 19;
        /// Plusieurs instructions dans une seule soumission.
        const MULTIPLE_STATEMENTS  = 1 << 20;
        /// L'annulation atteint la requête **côté serveur**
        /// (`pg_cancel_backend`, `KILL QUERY`, `sqlite3_interrupt`). Sans ce
        /// drapeau, un bouton « Annuler » ne peut pas prétendre couper la
        /// requête (DRIVER-CONTRACT §2).
        const SERVER_SIDE_CANCEL   = 1 << 21;
        /// Les résultats arrivent en flux, sans matérialisation complète.
        const STREAMING            = 1 << 22;
        /// Le nombre de lignes affectées par une écriture est fiable.
        const AFFECTED_ROWS        = 1 << 23;
        /// `EXPLAIN` ou équivalent.
        const EXPLAIN              = 1 << 24;
        /// `EXPLAIN ANALYZE` : le plan est **exécuté**. Ce n'est pas une
        /// lecture inoffensive sur une instruction d'écriture.
        const EXPLAIN_ANALYZE      = 1 << 25;
        /// La session accepte du DDL.
        const DDL                  = 1 << 26;
        /// La session accepte des écritures de données.
        const DML                  = 1 << 27;
        /// La session accepte la gestion des droits.
        const GRANT_REVOKE         = 1 << 28;
        /// Chargement en masse dédié (`COPY`, `LOAD DATA`).
        const BULK_LOAD            = 1 << 29;
        /// Le serveur sait imposer une session en lecture seule — une garantie
        /// bien plus solide qu'un filtrage côté client.
        const READ_ONLY_SESSION    = 1 << 30;
        /// La session sait déclarer où elle résout les noms non qualifiés, et
        /// rendre ce que le serveur a effectivement retenu. Un moteur qui n'a
        /// pas cette capacité n'affiche pas de sélecteur de contexte : ses
        /// schémas se qualifient dans le SQL ([ADR-0019](../../docs/adr/0019-contexte-de-session.md)).
        const SESSION_CONTEXT      = 1 << 31;
        // Les bits 16 à 31 de l'exécution sont pris ; ces deux-là continuent
        // au-delà des langages plutôt que d'en déloger un. Le numéro d'un
        // drapeau est sérialisé : le réattribuer changerait le sens d'une
        // capacité déjà écrite quelque part.
        /// La source sait **ordonner** une lecture d'aperçu. Absente, l'ordre
        /// des lignes reste celui que le moteur choisit, et aucun contrôle de
        /// tri n'est proposé ([ADR-0020](../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
        const PREVIEW_SORT         = 1 << 42;
        /// La source sait **restreindre** une lecture d'aperçu à des lignes qui
        /// vérifient une condition. Séparée du tri : un moteur peut savoir
        /// ordonner sans savoir filtrer, et l'inverse.
        const PREVIEW_FILTER       = 1 << 43;

        // ── Langages acceptés ───────────────────────────────────────────────
        /// SQL.
        const SQL                  = 1 << 32;
        /// Cypher.
        const CYPHER               = 1 << 33;
        /// Gremlin.
        const GREMLIN              = 1 << 34;
        /// Langage documentaire MongoDB.
        const MONGO_QUERY          = 1 << 35;
        /// Commandes Redis.
        const REDIS_COMMAND        = 1 << 36;
        /// Query DSL Elasticsearch / OpenSearch.
        const SEARCH_DSL           = 1 << 37;
        /// CQL.
        const CQL                  = 1 << 38;
        /// PartiQL.
        const PARTIQL              = 1 << 39;
        /// InfluxQL.
        const INFLUXQL             = 1 << 40;
        /// Flux.
        const FLUX                 = 1 << 41;

        // ── Modèle de données ───────────────────────────────────────────────
        /// Modèle relationnel.
        const RELATIONAL           = 1 << 48;
        /// Modèle documentaire.
        const DOCUMENT             = 1 << 49;
        /// Modèle clé-valeur.
        const KEY_VALUE            = 1 << 50;
        /// Modèle graphe.
        const GRAPH                = 1 << 51;
        /// Séries temporelles.
        const TIME_SERIES          = 1 << 52;
        /// La source n'impose pas de schéma.
        const SCHEMALESS           = 1 << 53;
        /// Le schéma exposé est **inféré par échantillonnage**, donc faillible.
        /// Un champ absent de l'échantillon existe peut-être plus loin
        /// (DRIVER-CONTRACT §3).
        const INFERRED_SCHEMA      = 1 << 54;
        /// Recherche vectorielle.
        const VECTOR_SEARCH        = 1 << 55;
        /// Recherche plein texte.
        const FULL_TEXT_SEARCH     = 1 << 56;
    }
}

impl Capabilities {
    /// Masque de tous les drapeaux de langage.
    ///
    /// Défini hors de la macro pour ne pas apparaître comme un drapeau composite
    /// dans [`Capabilities::iter_names`] — le rendu de [`fmt::Display`] resterait
    /// exact, mais moins précis.
    pub const LANGUAGES: Self = Self::from_bits_retain(
        Self::SQL.bits()
            | Self::CYPHER.bits()
            | Self::GREMLIN.bits()
            | Self::MONGO_QUERY.bits()
            | Self::REDIS_COMMAND.bits()
            | Self::SEARCH_DSL.bits()
            | Self::CQL.bits()
            | Self::PARTIQL.bits()
            | Self::INFLUXQL.bits()
            | Self::FLUX.bits(),
    );

    /// Le drapeau correspondant à un langage de requête.
    #[must_use]
    pub const fn for_language(language: QueryLanguage) -> Self {
        match language {
            QueryLanguage::Sql(_) => Self::SQL,
            QueryLanguage::Cypher => Self::CYPHER,
            QueryLanguage::Gremlin => Self::GREMLIN,
            QueryLanguage::MongoQuery => Self::MONGO_QUERY,
            QueryLanguage::RedisCommand => Self::REDIS_COMMAND,
            QueryLanguage::SearchDsl => Self::SEARCH_DSL,
            QueryLanguage::Cql => Self::CQL,
            QueryLanguage::PartiQl => Self::PARTIQL,
            QueryLanguage::InfluxQl => Self::INFLUXQL,
            QueryLanguage::Flux => Self::FLUX,
        }
    }

    /// Exige la présence de toutes les capacités demandées.
    ///
    /// # Erreurs
    /// Renvoie [`OxynError::NotSupported`] nommant les drapeaux **manquants**,
    /// et eux seuls : un message qui répète tout ce qui était demandé n'aide
    /// personne à comprendre ce qui bloque.
    pub fn require(&self, cap: Capabilities) -> Result<()> {
        let manquantes = cap.difference(*self);
        if manquantes.is_empty() {
            return Ok(());
        }
        Err(OxynError::NotSupported {
            capability: manquantes.to_string(),
        })
    }

    /// La session accepte-t-elle ce langage de requête ?
    #[must_use]
    pub fn supports_language(&self, language: QueryLanguage) -> bool {
        self.contains(Self::for_language(language))
    }

    /// Exige que le langage soit accepté.
    ///
    /// # Erreurs
    /// Renvoie [`OxynError::NotSupported`] si le langage n'est pas déclaré.
    pub fn require_language(&self, language: QueryLanguage) -> Result<()> {
        self.require(Self::for_language(language))
    }
}

impl fmt::Display for Capabilities {
    /// Liste les capacités actives, séparées par ` | `.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_empty() {
            return f.write_str("(none)");
        }
        let mut premier = true;
        for (nom, _) in self.iter_names() {
            if !premier {
                f.write_str(" | ")?;
            }
            f.write_str(nom)?;
            premier = false;
        }
        // Un bit non nommé ne peut venir que de `from_bits_retain` sur une
        // valeur relue d'un fichier écrit par une version plus récente.
        let nommees = Self::all().intersection(*self);
        let inconnues = self.difference(nommees);
        if !inconnues.is_empty() {
            if !premier {
                f.write_str(" | ")?;
            }
            write!(f, "(inconnues: {:#x})", inconnues.bits())?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::query::SqlDialect;

    #[test]
    fn require_reussit_quand_tout_est_present() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::STREAMING;
        assert!(caps.require(Capabilities::SQL).is_ok());
        assert!(
            caps.require(Capabilities::SQL | Capabilities::TRANSACTIONS)
                .is_ok()
        );
        assert!(caps.require(Capabilities::empty()).is_ok());
    }

    #[test]
    fn require_nomme_seulement_ce_qui_manque() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
        let err = caps
            .require(Capabilities::SQL | Capabilities::SERVER_SIDE_CANCEL)
            .expect_err("SERVER_SIDE_CANCEL est absente");

        let OxynError::NotSupported { capability } = &err else {
            panic!("mauvaise variante d'erreur : {err:?}");
        };
        assert_eq!(capability.as_str(), "SERVER_SIDE_CANCEL");
        assert!(
            !capability.contains("SQL"),
            "le message doit nommer ce qui manque, pas ce qui est demandé"
        );
    }

    #[test]
    fn require_nomme_toutes_les_capacites_manquantes() {
        let caps = Capabilities::SQL;
        let err = caps
            .require(Capabilities::TRANSACTIONS | Capabilities::SAVEPOINTS)
            .expect_err("les deux manquent");
        let message = err.to_string();
        assert!(message.contains("TRANSACTIONS"), "{message}");
        assert!(message.contains("SAVEPOINTS"), "{message}");
    }

    #[test]
    fn display_liste_les_capacites_actives() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS;
        let rendu = caps.to_string();
        assert!(rendu.contains("SQL"), "{rendu}");
        assert!(rendu.contains("TRANSACTIONS"), "{rendu}");
        assert!(rendu.contains(" | "), "{rendu}");
        assert_eq!(Capabilities::empty().to_string(), "(none)");
    }

    #[test]
    fn un_langage_non_declare_est_refuse() {
        // Un driver clé-valeur ne parle pas SQL : on refuse, on ne traduit pas.
        let redis = Capabilities::REDIS_COMMAND | Capabilities::KEY_VALUE;
        assert!(redis.supports_language(QueryLanguage::RedisCommand));
        assert!(!redis.supports_language(QueryLanguage::SQL));

        let err = redis
            .require_language(QueryLanguage::Sql(SqlDialect::Postgres))
            .expect_err("SQL n'est pas déclaré");
        assert!(err.to_string().contains("SQL"));
        assert!(err.is_user_error());
    }

    #[test]
    fn le_dialecte_ne_change_pas_le_drapeau_de_langage() {
        assert_eq!(
            Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Postgres)),
            Capabilities::for_language(QueryLanguage::Sql(SqlDialect::Sqlite)),
            "le dialecte est une nuance de grammaire, pas une capacité distincte"
        );
    }

    #[test]
    fn le_masque_des_langages_couvre_chaque_langage() {
        for langage in [
            QueryLanguage::SQL,
            QueryLanguage::Cypher,
            QueryLanguage::Gremlin,
            QueryLanguage::MongoQuery,
            QueryLanguage::RedisCommand,
            QueryLanguage::SearchDsl,
            QueryLanguage::Cql,
            QueryLanguage::PartiQl,
            QueryLanguage::InfluxQl,
            QueryLanguage::Flux,
        ] {
            let drapeau = Capabilities::for_language(langage);
            assert!(
                Capabilities::LANGUAGES.contains(drapeau),
                "{langage} absent du masque LANGUAGES"
            );
        }
    }

    #[test]
    fn les_drapeaux_ne_se_chevauchent_pas() {
        let mut vus = 0_u64;
        for (nom, drapeau) in Capabilities::all().iter_names() {
            let bits = drapeau.bits();
            assert_eq!(bits.count_ones(), 1, "{nom} n'est pas un drapeau simple");
            assert_eq!(vus & bits, 0, "{nom} réutilise un bit déjà pris");
            vus |= bits;
        }
    }

    #[test]
    fn aller_retour_json() {
        let caps = Capabilities::SQL | Capabilities::TRANSACTIONS | Capabilities::INDEXES;
        let json = serde_json::to_string(&caps).expect("sérialisation");
        let relu: Capabilities = serde_json::from_str(&json).expect("désérialisation");
        assert_eq!(caps, relu);
    }

    /// Deux capacités qui partagent un bit sont la même capacité.
    ///
    /// Rien ne le signale à l'exécution : `bitflags` fusionne deux constantes de
    /// même valeur, si bien qu'`iter_names` n'en rend qu'une et qu'un test
    /// construit sur elle passerait. Une session PostgreSQL déclarant savoir
    /// trier un aperçu annoncerait aussi qu'elle parle Cypher, et une interface
    /// conditionnelle aux capacités dessinerait la mauvaise surface.
    ///
    /// La déclaration est donc relue **dans la source**, seul endroit où les
    /// deux numéros existent encore côte à côte.
    #[test]
    fn aucun_drapeau_ne_partage_son_bit_avec_un_autre() {
        let source = include_str!("capabilities.rs");
        let mut vus: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
        for ligne in source.lines() {
            let Some((gauche, droite)) = ligne.split_once("= 1 << ") else {
                continue;
            };
            let Some(nom) = gauche.trim().strip_prefix("const ") else {
                continue;
            };
            let Some(numero) = droite.split(';').next() else {
                continue;
            };
            let Ok(bit) = numero.trim().parse::<u32>() else {
                continue;
            };
            let nom = nom.trim().to_owned();
            if let Some(precedent) = vus.insert(bit, nom.clone()) {
                panic!("{nom} et {precedent} partagent le bit {bit}");
            }
        }
        assert!(
            vus.len() >= 50,
            "la lecture de la source n'a trouvé que {} drapeaux : le format a changé \
             et ce test ne vérifie plus rien",
            vus.len()
        );
    }
}
