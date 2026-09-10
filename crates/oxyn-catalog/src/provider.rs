//! L'interface d'introspection qu'une session expose.
//!
//! L'introspection est **coûteuse** — des minutes sur un schéma à 20 000 objets
//! (ARCHITECTURE §6). Le trait est donc paresseux et hiérarchique : on ne
//! demande jamais l'arbre entier, on descend d'un palier quand l'utilisateur
//! ouvre un nœud. Rien ici n'est mis en cache ; c'est le rôle de
//! [`CatalogCache`](crate::cache::CatalogCache).
//!
//! # Trois règles qui gouvernent ce trait
//!
//! **Chaque méthode est annulable.** Un `&CancelToken` traverse toutes les
//! signatures. Une introspection de 4 minutes qu'on ne peut pas interrompre
//! occupe une connexion du pool et un verrou après la fermeture de l'onglet
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Une liste vide et « je ne sais pas » sont deux réponses différentes.** Les
//! paliers optionnels ([`list_catalogs`](CatalogProvider::list_catalogs),
//! [`list_namespaces`](CatalogProvider::list_namespaces)) rendent par défaut une
//! liste vide : elle dit « ce palier n'existe pas ici », et l'arborescence le
//! comprend. [`list_indexes`](CatalogProvider::list_indexes) et
//! [`list_foreign_keys`](CatalogProvider::list_foreign_keys) rendent par défaut
//! [`OxynError::NotSupported`] : une liste vide y signifierait « cette table n'a
//! pas d'index », affirmation que le driver n'a pas les moyens de faire. Ne pas
//! savoir faire est une réponse acceptable ; laisser croire ne l'est pas
//! ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **Tout ce qui ressort est une donnée hostile.** Noms d'objets et commentaires
//! peuvent contenir du SQL, des séquences de contrôle de terminal, ou du texte
//! imitant une consigne ([SECURITY, surface d'entrée §2](../../../docs/SECURITY.md)).
//! Le modèle valide ce qu'il peut à la construction ; l'échappement revient à
//! [`CatalogPath::qualify`](crate::path::CatalogPath::qualify) et l'encadrement
//! pour l'IA au point de passage unique de `oxyn-ai`.
//!
//! # Frontière WASM
//!
//! Le trait respecte dès aujourd'hui les contraintes de
//! [PLUGIN-CONTRACT](../../../docs/PLUGIN-CONTRACT.md) : aucun paramètre
//! générique, aucun état partagé implicite, aucun rappel synchrone vers l'hôte,
//! et toute erreur exprimée en valeur. Il reste objet-sûr, donc utilisable
//! derrière `Box<dyn CatalogProvider>` — contrainte dure, pas préférence
//! (ARCHITECTURE §4.1).

use async_trait::async_trait;
use oxyn_core::{CancelToken, OxynError, Result};

use crate::RelationDefinition;
use crate::model::{
    CatalogRef, Constraint, ForeignKey, IncomingForeignKey, Index, NamespaceRef, Relation,
    RelationRef, ServerInfo,
};
use crate::path::CatalogPath;

/// Ce qu'une session sait dire de la structure de sa source.
///
/// Obtenu par
/// [`Session::catalog`](../../../docs/ARCHITECTURE.md) ; une implémentation par
/// driver, jamais une par produit (ADR-0003).
#[async_trait]
pub trait CatalogProvider: Send + Sync {
    /// Identité du serveur et capacités de la session.
    ///
    /// C'est le seul appel qui n'a pas de valeur par défaut : une source qui ne
    /// sait rien dire d'elle-même peut rendre un
    /// [`ServerInfo`] aux champs vides, mais elle doit déclarer ses capacités —
    /// tout le reste de l'interface s'y accroche.
    ///
    /// # Erreurs
    /// Toute erreur de la session : coupure, droits insuffisants, annulation.
    async fn server_info(&self, cancel: &CancelToken) -> Result<ServerInfo>;

    /// Les catalogues visibles.
    ///
    /// Rend une liste **vide** par défaut : c'est la réponse juste pour MySQL,
    /// MongoDB ou Elasticsearch, qui n'ont pas ce palier. Ne pas confondre avec
    /// « aucun catalogue accessible », que le driver signale par une erreur de
    /// droits.
    ///
    /// # Erreurs
    /// Toute erreur de la session.
    async fn list_catalogs(&self, cancel: &CancelToken) -> Result<Vec<CatalogRef>> {
        let _ = cancel;
        Ok(Vec::new())
    }

    /// Les espaces de noms d'un catalogue, ou du serveur quand ce palier
    /// n'existe pas (`catalog` vaut alors `None`).
    ///
    /// Rend une liste **vide** par défaut, pour la même raison que
    /// [`Self::list_catalogs`].
    ///
    /// # Erreurs
    /// Toute erreur de la session.
    async fn list_namespaces(
        &self,
        catalog: Option<&str>,
        cancel: &CancelToken,
    ) -> Result<Vec<NamespaceRef>> {
        let _ = (catalog, cancel);
        Ok(Vec::new())
    }

    /// Les relations d'un espace de noms.
    ///
    /// `namespace` peut être un chemin vide (Elasticsearch : ni catalogue ni
    /// espace de noms), un catalogue seul (Neo4j) ou un espace de noms complet.
    /// Il ne désigne **jamais** une relation.
    ///
    /// Rend des [`RelationRef`] et non des [`Relation`] : décrire les champs de
    /// 20 000 tables prend des minutes, et l'arborescence n'en a pas besoin pour
    /// s'afficher.
    ///
    /// # Erreurs
    /// Toute erreur de la session, y compris un espace de noms inexistant.
    async fn list_relations(
        &self,
        namespace: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<RelationRef>>;

    /// La description complète d'une relation : champs, types, volumétrie.
    ///
    /// Pour une source sans schéma, l'inférence par échantillonnage est marquée
    /// champ par champ ([`Field::inferred`](crate::model::Field::inferred)) ;
    /// elle n'est jamais présentée comme une déclaration du serveur.
    ///
    /// # Erreurs
    /// Toute erreur de la session, y compris une relation inexistante.
    async fn describe_relation(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Relation>;

    /// Les index d'une relation.
    ///
    /// Rend [`OxynError::NotSupported`] par défaut : une liste vide affirmerait
    /// que la relation n'a pas d'index, et un driver qui ne sait pas
    /// introspecter les index ne peut pas l'affirmer. Un driver qui sait le
    /// faire déclare [`Capabilities::INDEXES`](oxyn_core::Capabilities::INDEXES).
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session ne sait pas introspecter les
    /// index, ou toute erreur de la session.
    async fn list_indexes(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Index>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "INDEXES".to_owned(),
        })
    }

    /// Reads creation statements without executing them. Unsupported object kinds
    /// return an error rather than an invented or silently partial definition.
    async fn relation_definition(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<RelationDefinition> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "OBJECT_DEFINITION".into(),
        })
    }

    /// Finds foreign keys whose target is this relation. Unsupported discovery
    /// is an error, never a statement that the relation has no incoming keys.
    async fn list_incoming_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<IncomingForeignKey>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "INCOMING_FOREIGN_KEYS".to_owned(),
        })
    }

    /// Constraints declared on one relation, read only on explicit request.
    /// An empty list means no constraints; unsupported introspection is an error.
    async fn list_constraints(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<Constraint>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "CONSTRAINTS".to_owned(),
        })
    }

    /// Les clés étrangères portées par une relation.
    ///
    /// Rend [`OxynError::NotSupported`] par défaut, pour la même raison que
    /// [`Self::list_indexes`]. C'est ce qui permet de tracer un diagramme de
    /// relations sans le deviner ; le deviner à partir des noms de colonnes
    /// produirait des liens faux présentés comme des faits.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session ne sait pas introspecter les
    /// clés étrangères, ou toute erreur de la session.
    async fn list_foreign_keys(
        &self,
        relation: &CatalogPath,
        cancel: &CancelToken,
    ) -> Result<Vec<ForeignKey>> {
        let _ = (relation, cancel);
        Err(OxynError::NotSupported {
            capability: "FOREIGN_KEYS".to_owned(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::pin;
    use std::task::{Context, Poll, Waker};

    use oxyn_core::Capabilities;

    use super::*;
    use crate::model::RelationKind;

    /// Sonde un futur une fois, sans exécuteur.
    ///
    /// `oxyn-catalog` n'a pas `tokio` à son contrat de dépendances, donc pas de
    /// `#[tokio::test]`. Les implémentations d'essai ci-dessous n'attendent rien
    /// et sont prêtes au premier sondage ; la même technique est employée dans
    /// `oxyn_core::cancel`.
    fn resoudre<F: Future>(mut futur: std::pin::Pin<&mut F>) -> F::Output {
        let mut cx = Context::from_waker(Waker::noop());
        match futur.as_mut().poll(&mut cx) {
            Poll::Ready(valeur) => valeur,
            Poll::Pending => panic!("le futur d'essai doit être prêt au premier sondage"),
        }
    }

    /// Le minimum qu'un driver doit fournir : une source plate, sans catalogue
    /// ni espace de noms, à la façon d'Elasticsearch.
    #[derive(Debug)]
    struct SourcePlate;

    #[async_trait]
    impl CatalogProvider for SourcePlate {
        async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
            Ok(ServerInfo::new(
                "SourcePlate",
                "1.0",
                Capabilities::SEARCH_DSL | Capabilities::DOCUMENT,
            ))
        }

        async fn list_relations(
            &self,
            namespace: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Vec<RelationRef>> {
            let reference = RelationRef::new(namespace.clone(), "journaux", RelationKind::Index)?;
            Ok(vec![reference])
        }

        async fn describe_relation(
            &self,
            relation: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Relation> {
            let nom = relation.relation().unwrap_or("");
            Ok(Relation::new(nom, RelationKind::Index))
        }
    }

    #[test]
    fn les_paliers_absents_rendent_une_liste_vide() {
        let source = SourcePlate;
        let jeton = CancelToken::new();

        let catalogues = resoudre(pin!(source.list_catalogs(&jeton))).expect("appel réussi");
        assert!(
            catalogues.is_empty(),
            "une source sans palier catalogue rend une liste vide, pas une erreur"
        );

        let espaces = resoudre(pin!(source.list_namespaces(None, &jeton))).expect("appel réussi");
        assert!(espaces.is_empty());
    }

    #[test]
    fn une_introspection_non_supportee_est_un_refus_pas_une_liste_vide() {
        // DRIVER-CONTRACT §5 : une liste vide affirmerait « pas d'index », ce
        // qu'un driver qui ne sait pas introspecter ne peut pas affirmer.
        let source = SourcePlate;
        let jeton = CancelToken::new();
        let chemin = CatalogPath::for_relation(None, None, "journaux").expect("valide");

        let err = resoudre(pin!(source.list_indexes(&chemin, &jeton)))
            .expect_err("la source ne sait pas introspecter les index");
        assert!(matches!(err, OxynError::NotSupported { .. }));
        assert!(err.to_string().contains("INDEXES"));
        assert!(err.is_user_error(), "ce n'est pas un incident");

        let err =
            resoudre(pin!(source.list_constraints(&chemin, &jeton))).expect_err("unsupported");
        assert!(matches!(err, OxynError::NotSupported { .. }));

        let err = resoudre(pin!(source.list_foreign_keys(&chemin, &jeton)))
            .expect_err("la source ne sait pas introspecter les clés étrangères");
        assert!(err.to_string().contains("FOREIGN_KEYS"));
    }

    #[test]
    fn le_trait_reste_objet_sur() {
        // Contrainte dure d'ARCHITECTURE §4.1 : les traits s'emploient derrière
        // `Box<dyn ...>`. Une méthode générique la romprait en silence.
        let source: Box<dyn CatalogProvider> = Box::new(SourcePlate);
        let jeton = CancelToken::new();
        let info = resoudre(pin!(source.server_info(&jeton))).expect("appel réussi");
        assert_eq!(info.product, "SourcePlate");
    }

    #[test]
    fn les_references_rendues_portent_leur_chemin_complet() {
        let source = SourcePlate;
        let jeton = CancelToken::new();
        let espace = CatalogPath::empty();

        let relations = resoudre(pin!(source.list_relations(&espace, &jeton))).expect("réussi");
        let premiere = relations.first().expect("une relation");
        assert_eq!(premiere.path().to_string(), "journaux");
    }
}
