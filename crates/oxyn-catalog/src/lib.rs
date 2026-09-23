//! Le modèle de métadonnées unifié d'Oxyn, son cache et sa recherche.
//!
//! L'introspection d'une base est **coûteuse** : des minutes sur un schéma à
//! 20 000 objets (ARCHITECTURE §6). Cette crate est ce qui permet de ne la payer
//! qu'une fois — et ce qui rend possibles les deux choses qui en découlent :
//! l'**exploration hors ligne**, et le **contexte d'un agent** construit à
//! partir du catalogue local plutôt que d'un aller-retour serveur à chaque
//! question.
//!
//! # Ce qu'on y trouve
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`model`] | la hiérarchie à cinq paliers, ses relations et ses champs | ARCHITECTURE §6 |
//! | [`path`] | le chemin qualifié, et la citation d'identifiant | DRIVER-CONTRACT §6, I-10 |
//! | [`provider`] | ce qu'une session sait dire de sa structure | DRIVER-CONTRACT §5 |
//! | [`cache`] | l'arbre en mémoire, sa fraîcheur, son invalidation | ARCHITECTURE §6 |
//! | [`mod@search`] | la sélection lexicale des relations pertinentes | ARCHITECTURE §7.4 |
//!
//! # Les quatre choix qui gouvernent cette crate
//!
//! **Les paliers sont optionnels, et pas seulement les derniers.** MySQL n'a pas
//! de catalogue, Neo4j pas d'espace de noms, Elasticsearch ni l'un ni l'autre.
//! Rien n'est comblé par une valeur inventée : un palier absent est absent, et
//! [`CatalogPath`] le rend tel quel — y compris à l'aller-retour par le disque.
//!
//! **Un nom d'objet est une entrée hostile.** Une table nommée
//! `"users"; DROP TABLE audit; --` est légale dans PostgreSQL.
//! [`CatalogPath::qualify`] est le seul chemin par lequel un identifiant rejoint
//! une requête composée par Oxyn (I-10), et [`fn@search`] classe des noms sans
//! jamais composer d'invite.
//!
//! **Une liste vide et « je ne sais pas » sont deux réponses différentes.** Un
//! [`CatalogProvider`] qui ne sait pas introspecter les index refuse ; il ne
//! rend pas une liste vide, qui affirmerait qu'il n'y en a pas
//! ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)). Le cache maintient
//! la distinction : [`CatalogCache::indexes`] rend `None` pour « pas lu » et une
//! tranche vide pour « aucun ».
//!
//! **Un palier jamais lu n'est pas périmé.** [`CatalogCache::stale`] ne réclame
//! que ce qui a déjà été lu au moins une fois — sinon un rafraîchissement de
//! fond décrirait les 20 000 relations que personne n'a ouvertes.
//!
//! # Exemple
//!
//! ```
//! use std::time::Duration;
//!
//! use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, QuoteStyle};
//! use oxyn_catalog::model::{Relation, RelationKind};
//!
//! let mut cache = CatalogCache::new();
//!
//! // Une insertion partielle : on décrit une table sans avoir listé son schéma.
//! let table = CatalogPath::for_relation(Some("caisse"), Some("public"), "clients")?;
//! cache.set_relation(&table, Relation::new("clients", RelationKind::Table))?;
//!
//! // Le SQL composé par Oxyn cite toujours ses identifiants.
//! assert_eq!(table.qualify(QuoteStyle::Double), r#""caisse"."public"."clients""#);
//!
//! // Après un DDL, le sous-arbre est marqué à relire — sans perdre ce qu'il sait.
//! cache.invalidate(&CatalogScope::Relation(table.clone()));
//! assert!(cache.relation(&table).is_some());
//! assert_eq!(cache.stale(Duration::from_secs(3600)), vec![CatalogScope::Relation(table)]);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

pub mod cache;
pub mod definition;
pub mod model;
pub mod nesting;
pub mod path;
pub mod provider;
pub mod search;

pub use cache::{CacheError, CatalogCache, CatalogHandle, CatalogScope, Freshness, SharedCatalog};
pub use definition::{DefinitionSource, RelationDefinition};
pub use model::{
    CatalogRef, Constraint, ConstraintKind, Field, ForeignKey, ForeignKeyTarget,
    IncomingForeignKey, Index, LogicalType, NamespaceRef, ReferentialAction, Relation,
    RelationKind, RelationRef, ServerInfo,
};
pub use path::{CatalogLevel, CatalogPath, CatalogPathError, QuoteStyle, quote_identifier};
pub use provider::CatalogProvider;
pub use search::{MatchKind, SearchHit, SearchOptions, search};

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use oxyn_core::Capabilities;

    use crate::model::{Field, LogicalType, Relation, RelationKind, RelationRef, ServerInfo};
    use crate::{CatalogCache, CatalogPath, CatalogScope, SearchOptions, search};

    /// Le trajet complet de la crate, sur le seul scénario qui les met tous en
    /// jeu : lister, décrire, chercher, subir un DDL, relire ce qui est périmé.
    #[test]
    fn le_trajet_complet_du_catalogue() {
        let mut cache = CatalogCache::new();
        cache.set_server_info(ServerInfo::new(
            "PostgreSQL",
            "17.2",
            Capabilities::SQL | Capabilities::SCHEMAS | Capabilities::INDEXES,
        ));

        let espace = CatalogPath::for_namespace(Some("caisse"), "public").expect("chemin valide");
        cache
            .set_relations(
                &espace,
                vec![
                    RelationRef::new(espace.clone(), "commandes", RelationKind::Table)
                        .expect("nom valide"),
                    RelationRef::new(espace.clone(), "clients", RelationKind::Table)
                        .expect("nom valide"),
                ],
            )
            .expect("un espace de noms");

        let commandes = espace.with_relation("commandes").expect("chemin valide");
        cache
            .set_relation(
                &commandes,
                Relation::new("commandes", RelationKind::Table).with_fields(vec![
                    Field::new("id", 0, LogicalType::INT64, "int8").primary_key(),
                    Field::new("client_id", 1, LogicalType::INT64, "int8").not_null(),
                ]),
            )
            .expect("le chemin nomme une relation");

        // La recherche trouve la table par son nom, et l'autre par son champ.
        let resultats = search(&cache, "client", &SearchOptions::default());
        let noms: Vec<Option<&str>> = resultats.iter().map(|hit| hit.path.relation()).collect();
        assert_eq!(noms, [Some("clients"), Some("commandes")]);

        // Un ALTER TABLE émis depuis Oxyn : le sous-arbre est à relire, mais il
        // reste consultable hors ligne.
        cache.invalidate(&CatalogScope::Relation(commandes.clone()));
        assert!(cache.relation(&commandes).is_some());
        assert_eq!(
            cache.stale(Duration::from_secs(3600)),
            vec![CatalogScope::Relation(commandes)]
        );
    }
}
