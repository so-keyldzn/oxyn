//! Les trois traits que tout driver implémente.
//!
//! Ce sont ceux d'[`ARCHITECTURE` §4.1](../../../docs/ARCHITECTURE.md) :
//! [`Driver`] ouvre, [`Session`] exécute, [`Cursor`] rend les lots. Une erreur
//! ici se paie quatorze fois — c'est le nombre d'implémentations réelles que la
//! vision suppose (ADR-0003).
//!
//! # Trois contraintes dures, pas des préférences
//!
//! **Objet-sûr.** Les trois traits s'emploient derrière `Box<dyn ...>`. Aucune
//! méthode générique, aucun `impl Trait` en position de retour, aucun `where
//! Self: Sized` sur une méthode appelée à travers l'objet. Un test de ce module
//! le vérifie, parce que la rupture est facile et le message du compilateur
//! l'est moins.
//!
//! **Annulable de bout en bout.** Un `&CancelToken` traverse toute méthode qui
//! peut durer. Le jeton **signale** ; c'est au driver d'en faire un
//! `pg_cancel_backend`, un `KILL QUERY` ou un `sqlite3_interrupt`, et de
//! déclarer [`Capabilities::SERVER_SIDE_CANCEL`] s'il en est capable. Abandonner
//! le futur ne libère ni la connexion ni le verrou
//! ([`DRIVER-CONTRACT` §2](../../../docs/DRIVER-CONTRACT.md)).
//!
//! **En flux, jamais matérialisé.** [`Cursor::next_batch`] rend **un** lot. Un
//! driver qui construit tout le résultat avant de rendre la main fait grimper la
//! RSS jusqu'à l'OOM killer, sur un simple clic dans l'arborescence
//! ([I-06](../../../CLAUDE.md#i-06)).
//!
//! # Frontière WASM
//!
//! Ces traits respectent dès aujourd'hui les contraintes de
//! [PLUGIN-CONTRACT](../../../docs/PLUGIN-CONTRACT.md) : aucun paramètre
//! générique, aucun rappel synchrone vers l'hôte, toute erreur exprimée en
//! valeur. Les corriger en phase 4 coûterait une refonte des quatorze drivers.

use std::time::Duration;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use async_trait::async_trait;
use futures::future::BoxFuture;
use oxyn_catalog::{CatalogPath, CatalogProvider};
use oxyn_core::{
    CancelToken, Capabilities, ConnectionConfig, DriverId, ExecRequest, ExecStats, OxynError,
    PreviewShape, Result, StatementHandle,
};
use oxyn_data::BatchSource;

use crate::context::SessionContext;
use crate::credentials::Credentials;
use crate::metadata::DriverMetadata;

/// Un protocole de base de données, pas un produit.
///
/// `postgres` couvre Redshift, TimescaleDB et pgvector ; `mysql` couvre MariaDB
/// (ADR-0003). Un driver est sans état partagé : c'est [`Session`] qui porte une
/// connexion ouverte.
///
/// # Ce qu'un driver n'a pas le droit de faire
///
/// Lire une variable d'environnement, écrire un fichier, ouvrir une fenêtre,
/// journaliser une valeur liée, ou retenter tout seul. La politique de reprise
/// appartient à l'appelant, seul à savoir si l'opération est rejouable
/// ([`DRIVER-CONTRACT`](../../../docs/DRIVER-CONTRACT.md)).
#[async_trait]
pub trait Driver: Send + Sync + 'static {
    /// L'identifiant du protocole. Doit être égal à
    /// [`DriverMetadata::id`] — [`DriverRegistry::register`](crate::registry::DriverRegistry::register)
    /// le vérifie, parce qu'une divergence rendrait le driver introuvable après
    /// son enregistrement.
    fn id(&self) -> DriverId;

    /// Ce que le driver dit de lui-même, formulaire de connexion compris.
    ///
    /// La valeur est **empruntée** : elle est construite une fois, à la
    /// création du driver, pas à chaque affichage de la liste.
    fn metadata(&self) -> &DriverMetadata;

    /// Les capacités que le driver peut offrir **au mieux**.
    ///
    /// C'est un plafond indicatif, pas une promesse : ce qui fait foi est
    /// [`Session::capabilities`], évalué après connexion. Le même driver
    /// PostgreSQL parle à une base 12 sans `MERGE` et à une base 17 qui l'a
    /// (ADR-0003).
    fn capabilities(&self) -> Capabilities;

    /// Ouvre une session.
    ///
    /// `config` ne porte **aucun secret** : les identifiants arrivent par
    /// `credentials`, résolus depuis le trousseau du système par l'appelant. Un
    /// driver ne va jamais chercher lui-même un mot de passe, ni dans
    /// l'environnement, ni dans un fichier.
    ///
    /// # Divergence assumée avec ARCHITECTURE §4.1
    ///
    /// Le document ne montre que `(cfg, ct)`. Cette signature-là ne permet
    /// d'authentifier personne : `ConnectionConfig` ne porte qu'une *référence*
    /// de secret, que seul `oxyn-secrets` sait résoudre — et faire dépendre les
    /// drivers du trousseau du système renverserait le sens des dépendances.
    /// Le paramètre est donc explicite. À reporter dans ARCHITECTURE §4.1.
    ///
    /// # Erreurs
    /// [`OxynError::Connection`] si le serveur est injoignable,
    /// [`OxynError::Authentication`] s'il refuse les identifiants,
    /// [`OxynError::Cancelled`] si `cancel` se déclenche pendant la poignée de
    /// main, [`OxynError::Config`] si la configuration est inutilisable.
    async fn connect(
        &self,
        config: &ConnectionConfig,
        credentials: &Credentials,
        cancel: &CancelToken,
    ) -> Result<Box<dyn Session>>;
}

/// Une connexion ouverte.
///
/// Les méthodes prennent `&self` : une session est partageable, et la
/// synchronisation d'un client qui ne l'est pas appartient au driver. Elle est
/// `Send + Sync` parce qu'elle vit sur le runtime Tokio pendant que le thread
/// d'interface lit le tampon de résultats (ARCHITECTURE §9).
#[async_trait]
pub trait Session: Send + Sync {
    /// Ce que **cette** session sait faire.
    ///
    /// Évalué après connexion, à partir de la version du serveur, de ses
    /// extensions et des droits du compte — pas déduit du driver (ADR-0003).
    /// Un drapeau absent signifie « je ne sais pas faire », jamais « je ferai
    /// semblant ».
    fn capabilities(&self) -> Capabilities;

    /// Exécute une demande et rend un curseur sur ses lots.
    ///
    /// Rend la main **dès que le schéma est connu**, sans attendre la première
    /// ligne : c'est ce qui permet à la grille de dessiner ses colonnes pendant
    /// que les données arrivent.
    ///
    /// Le driver frappe une [`StatementHandle`] pour cette exécution et
    /// l'expose par [`Cursor::handle`] : c'est elle que [`Session::cancel`]
    /// vise.
    ///
    /// L'implémentation **refuse** un langage qu'elle ne déclare pas
    /// ([`Capabilities::require_language`]) ; elle ne traduit pas.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si le langage ou une capacité requise
    /// manque, [`OxynError::Query`] si le serveur rejette l'instruction,
    /// [`OxynError::Cancelled`] si `cancel` se déclenche.
    async fn execute(&self, request: ExecRequest, cancel: &CancelToken) -> Result<Box<dyn Cursor>>;

    /// Compose a read-only preview, with cancellable metadata I/O if needed.
    ///
    /// Implementations validate `limit` in `1..=1000`, quote every path segment
    /// according to their dialect, and set `read_only` and `max_rows` limits.
    /// This may read column types, but never executes the preview or changes
    /// session state. Unsupported drivers fail explicitly.
    ///
    /// `shape` carries the order, the predicate and the page asked for
    /// ([ADR-0020](../../../docs/adr/0020-apercu-trie-filtre-parcouru.md)). Its
    /// two halves are not alike.
    ///
    /// The **sort** is structured: a column is an identifier the implementation
    /// quotes, because Oxyn composes that fragment and answers for what it
    /// contains ([I-10](../../../CLAUDE.md#i-10)). A column the relation does
    /// not declare is refused, not forwarded in the hope the server rejects it.
    ///
    /// The **predicate** is SQL the user wrote, and it travels through
    /// untouched — neither parsed nor rewritten — exactly like the text of a
    /// console. It still ends up inside a statement Oxyn composes, so the
    /// implementation must make sure it cannot silently swallow what follows
    /// it: an unterminated comment at its end would otherwise eat the very
    /// `LIMIT` that bounds the read.
    ///
    /// What an implementation must not do is ignore part of `shape`. A
    /// predicate silently dropped returns rows the user believes they excluded,
    /// and nothing on screen says otherwise: refuse what the engine cannot
    /// express, with [`OxynError::NotSupported`] naming the missing capability.
    ///
    /// A non-zero [`PreviewShape::offset`] is only meaningful under a total
    /// order — see [`PreviewShape::needs_total_order`]. Implementations
    /// complete the requested sort with a unique key, the primary key when the
    /// catalog declares one, and refuse the page otherwise: without it, two
    /// consecutive pages show the same row twice and skip another, silently.
    /// The completion applies from the first page: ordering page 0 by one
    /// column and page 1 by two would make a row reappear exactly at the
    /// boundary.
    async fn preview_request(
        &self,
        _path: &CatalogPath,
        _limit: u32,
        _shape: &PreviewShape,
        _cancel: &CancelToken,
    ) -> Result<ExecRequest> {
        Err(OxynError::NotSupported {
            capability: "relation preview".to_owned(),
        })
    }

    /// Déclare où cette session résout les noms qu'une instruction ne qualifie
    /// pas, et attend la confirmation du serveur.
    ///
    /// N'est appelée que si [`Capabilities::SESSION_CONTEXT`] est déclaré. Le
    /// driver **cite lui-même** chaque segment : un nom de schéma est une
    /// donnée venue du catalogue, et le concaténer exécuterait ce qu'il
    /// contient ([I-10](../../../CLAUDE.md#i-10)).
    ///
    /// L'implémentation ne touche à rien d'autre. Un changement de contexte qui
    /// viderait au passage un `search_path` composé par l'utilisateur, ou qui
    /// ouvrirait une transaction, ferait plus que ce que son nom annonce — et
    /// c'est précisément l'état de session invisible que le contrat refuse.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si le moteur n'a pas de contexte de session,
    /// [`OxynError::Cancelled`] si `cancel` se déclenche, et l'erreur du
    /// serveur si l'emplacement demandé n'existe pas.
    async fn set_context(&self, _context: &SessionContext, _cancel: &CancelToken) -> Result<()> {
        Err(OxynError::NotSupported {
            capability: "session context".to_owned(),
        })
    }

    /// Ce que le serveur a **confirmé**, jamais ce qui a été demandé.
    ///
    /// `None` tant qu'aucun contexte n'a été déclaré : l'interface montre alors
    /// que la session travaille dans ce que le serveur a choisi à l'ouverture,
    /// ce qui n'est pas la même chose qu'un emplacement choisi.
    ///
    /// Rend une valeur et non une référence : une implémentation garde son
    /// contexte derrière un verrou, parce que `set_context` prend `&self`.
    fn context(&self) -> Option<SessionContext> {
        None
    }

    /// Demande au **serveur** d'interrompre une exécution.
    ///
    /// N'est appelée que si [`Capabilities::SERVER_SIDE_CANCEL`] est déclaré :
    /// une implémentation qui se contente d'abandonner le futur laisse la
    /// requête tourner, la connexion prise et le verrou posé. Au dixième onglet
    /// fermé, la base refuse les connexions et l'utilisateur conclut qu'Oxyn a
    /// cassé sa production.
    ///
    /// Annuler une instruction déjà terminée n'est **pas** une erreur.
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session ne sait pas annuler côté
    /// serveur, ou toute erreur de transport.
    async fn cancel(&self, statement: StatementHandle) -> Result<()>;

    /// L'introspection de cette session.
    ///
    /// Emprunté, jamais construit à la demande : l'arborescence l'appelle à
    /// chaque nœud ouvert.
    fn catalog(&self) -> &dyn CatalogProvider;

    /// Vérifie que la connexion est vivante, et rend l'aller-retour mesuré.
    ///
    /// # Erreurs
    /// Toute erreur de transport. Une session dont le `ping` échoue est
    /// considérée comme perdue.
    async fn ping(&self) -> Result<Duration>;

    /// Ferme la session proprement.
    ///
    /// Consomme la session : une session fermée ne se réutilise pas. Une erreur
    /// de fermeture se signale mais ne se rattrape pas — les ressources locales
    /// sont libérées dans tous les cas.
    ///
    /// # Erreurs
    /// Toute erreur de transport rencontrée à la fermeture.
    async fn close(self: Box<Self>) -> Result<()>;

    /// Ouvre une transaction.
    ///
    /// # L'implémentation par défaut, et ce qu'elle protège
    ///
    /// Elle ne fait rien d'utile — **à dessein**. Elle commence par exiger
    /// [`Capabilities::TRANSACTIONS`] : une session qui ne le déclare pas rend
    /// [`OxynError::NotSupported`], qui est une réponse honnête. Une session qui
    /// le déclare **et** n'a pas redéfini cette méthode rend
    /// [`OxynError::Internal`], parce que c'est un bug du driver.
    ///
    /// Ce qu'il ne faut surtout pas faire, c'est réussir sans rien ouvrir :
    /// l'utilisateur croirait qu'un `ROLLBACK` a annulé son écriture. Ne pas
    /// savoir faire est une réponse acceptable ; laisser croire ne l'est pas
    /// ([`DRIVER-CONTRACT` §5](../../../docs/DRIVER-CONTRACT.md)).
    ///
    /// # Erreurs
    /// [`OxynError::NotSupported`] si la session n'a pas
    /// [`Capabilities::TRANSACTIONS`], [`OxynError::Internal`] si elle l'a sans
    /// implémenter la méthode, ou toute erreur du serveur.
    async fn begin(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("begin"))
    }

    /// Valide la transaction en cours.
    ///
    /// Même garde que [`begin`](Self::begin).
    ///
    /// # Erreurs
    /// Celles de [`begin`](Self::begin), plus le rejet du serveur si aucune
    /// transaction n'est ouverte.
    async fn commit(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("commit"))
    }

    /// Annule la transaction en cours.
    ///
    /// Même garde que [`begin`](Self::begin). C'est la méthode dont l'échec
    /// silencieux coûte le plus cher : un `ROLLBACK` qui ne rejoue rien laisse
    /// l'écriture appliquée.
    ///
    /// # Erreurs
    /// Celles de [`begin`](Self::begin), plus le rejet du serveur si aucune
    /// transaction n'est ouverte.
    async fn rollback(&self, cancel: &CancelToken) -> Result<()> {
        let _ = cancel;
        self.capabilities().require(Capabilities::TRANSACTIONS)?;
        Err(unimplemented_transaction("rollback"))
    }
}

/// Un flux de lots Arrow.
///
/// `Send` mais pas `Sync` : un curseur est déplacé sur la tâche qui le draine,
/// jamais partagé. La contre-pression et le débordement sur disque sont l'usage
/// de `oxyn-data` ; le curseur, lui, ne fait que rendre le lot suivant quand on
/// le lui demande.
#[async_trait]
pub trait Cursor: Send {
    /// La poignée de l'exécution qui alimente ce curseur.
    ///
    /// C'est ce que [`Session::cancel`] vise. **Ajout par rapport à
    /// ARCHITECTURE §4.1**, où rien ne dit d'où vient la
    /// [`StatementHandle`] : sans elle, l'annulation côté serveur n'a pas de
    /// cible. À reporter dans le document.
    fn handle(&self) -> StatementHandle;

    /// Le schéma des lots.
    ///
    /// Connu **avant** le premier lot, et stable pour toute la durée du flux.
    /// Une source sans schéma l'infère par échantillonnage et marque chaque
    /// champ comme déduit ; elle ne le présente jamais comme une déclaration du
    /// serveur ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
    fn schema(&self) -> SchemaRef;

    /// Le lot suivant, ou `None` quand le flux est épuisé.
    ///
    /// Le lot se dimensionne **en octets, pas en lignes** : mille lignes
    /// portant chacune un BLOB d'un mégaoctet font un gigaoctet, et un
    /// `batch_size` compté en lignes marche sur les tables de démonstration
    /// avant de déclencher l'OOM sur les vraies.
    ///
    /// Le futur doit être abandonnable. Après un abandon, le curseur est
    /// **inutilisable** : il a pu consommer des octets du flux, laissant le
    /// décodeur désynchronisé. Il se détruit, il ne se reprend pas.
    ///
    /// # Erreurs
    /// Toute erreur du serveur ou du décodage, **classée** :
    /// [`OxynError::driver`] avec la bonne
    /// [`ErrorClass`](oxyn_core::ErrorClass). Une expiration côté client
    /// pendant une écriture est [`Ambiguous`](oxyn_core::ErrorClass::Ambiguous),
    /// jamais transitoire (I-13).
    async fn next_batch(&mut self) -> Result<Option<RecordBatch>>;

    /// Ce que le curseur sait de l'exécution.
    ///
    /// Interrogé à la fin du flux pour clore le tampon. Le temps serveur, quand
    /// le serveur le rend, est ce qui permet de distinguer une base lente d'un
    /// réseau lent.
    fn stats(&self) -> ExecStats;
}

/// Un [`Cursor`] est une source de lots pour `oxyn-data`.
///
/// L'adaptation est l'aiguillage entre les deux crates : `oxyn-data` ne dépend
/// pas de `oxyn-driver` — c'est l'inverse — donc c'est ici que le pont se pose.
/// [`BatchSink`](oxyn_data::BatchSink) peut ainsi drainer un curseur de driver
/// avec contre-pression, sans qu'aucun driver n'ait à connaître le tampon.
///
/// L'implémentation vise `Box<dyn Cursor>` et non `dyn Cursor` : c'est sous
/// cette forme que le curseur circule, et `BatchSource` exige `Sized`
/// implicitement pour ses implémenteurs.
impl BatchSource for Box<dyn Cursor> {
    fn schema(&self) -> SchemaRef {
        (**self).schema()
    }

    fn next_batch(&mut self) -> BoxFuture<'_, std::result::Result<Option<RecordBatch>, OxynError>> {
        (**self).next_batch()
    }

    fn stats(&self) -> ExecStats {
        (**self).stats()
    }
}

/// L'erreur d'une session qui déclare les transactions sans les implémenter.
///
/// C'est un bug du driver, pas une erreur d'usage : le message le dit, pour
/// qu'il ne soit pas montré à l'utilisateur comme une limitation du serveur.
fn unimplemented_transaction(operation: &'static str) -> OxynError {
    OxynError::Internal(format!(
        "the session declares TRANSACTIONS without implementing `{operation}`: \
         a ROLLBACK that undoes nothing leaves the write applied"
    ))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use futures::executor::block_on;
    use oxyn_catalog::model::{Relation, RelationKind, RelationRef, ServerInfo};
    use oxyn_catalog::path::CatalogPath;
    use oxyn_data::{BatchSink, ResultBuffer};

    use super::*;
    use crate::metadata::DriverFamily;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("id", DataType::Int32, false)]))
    }

    fn lot(depart: i32, lignes: i32) -> RecordBatch {
        let ids: Vec<i32> = (depart..depart.saturating_add(lignes)).collect();
        RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(ids))])
            .expect("la colonne correspond au schéma construit juste au-dessus")
    }

    #[derive(Debug)]
    struct CatalogueFactice;

    #[async_trait]
    impl CatalogProvider for CatalogueFactice {
        async fn server_info(&self, _cancel: &CancelToken) -> Result<ServerInfo> {
            Ok(ServerInfo::new(
                "Factice",
                "1.0",
                Capabilities::SQL | Capabilities::RELATIONAL,
            ))
        }

        async fn list_relations(
            &self,
            _namespace: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Vec<RelationRef>> {
            Ok(Vec::new())
        }

        async fn describe_relation(
            &self,
            relation: &CatalogPath,
            _cancel: &CancelToken,
        ) -> Result<Relation> {
            Ok(Relation::new(
                relation.relation().unwrap_or(""),
                RelationKind::Table,
            ))
        }
    }

    #[derive(Debug)]
    struct CurseurFactice {
        handle: StatementHandle,
        restants: Vec<RecordBatch>,
        stats: ExecStats,
    }

    impl CurseurFactice {
        fn new(lots: Vec<RecordBatch>) -> Self {
            Self {
                handle: StatementHandle::new(),
                restants: lots,
                stats: ExecStats::default(),
            }
        }
    }

    #[async_trait]
    impl Cursor for CurseurFactice {
        fn handle(&self) -> StatementHandle {
            self.handle
        }

        fn schema(&self) -> SchemaRef {
            schema()
        }

        async fn next_batch(&mut self) -> Result<Option<RecordBatch>> {
            if self.restants.is_empty() {
                return Ok(None);
            }
            let lot = self.restants.remove(0);
            self.stats.record_batch(
                u64::try_from(lot.num_rows()).unwrap_or(u64::MAX),
                u64::try_from(lot.get_array_memory_size()).unwrap_or(u64::MAX),
            );
            Ok(Some(lot))
        }

        fn stats(&self) -> ExecStats {
            self.stats
        }
    }

    #[derive(Debug)]
    struct SessionFactice {
        capabilities: Capabilities,
        catalogue: CatalogueFactice,
    }

    impl SessionFactice {
        fn new(capabilities: Capabilities) -> Self {
            Self {
                capabilities,
                catalogue: CatalogueFactice,
            }
        }
    }

    #[async_trait]
    impl Session for SessionFactice {
        fn capabilities(&self) -> Capabilities {
            self.capabilities
        }

        async fn execute(
            &self,
            request: ExecRequest,
            _cancel: &CancelToken,
        ) -> Result<Box<dyn Cursor>> {
            self.capabilities.require_language(request.language)?;
            Ok(Box::new(CurseurFactice::new(vec![lot(0, 3), lot(3, 2)])))
        }

        async fn cancel(&self, _statement: StatementHandle) -> Result<()> {
            Ok(())
        }

        fn catalog(&self) -> &dyn CatalogProvider {
            &self.catalogue
        }

        async fn ping(&self) -> Result<Duration> {
            Ok(Duration::from_millis(1))
        }

        async fn close(self: Box<Self>) -> Result<()> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct DriverFactice {
        metadata: DriverMetadata,
    }

    impl DriverFactice {
        fn new() -> Self {
            Self {
                metadata: DriverMetadata::new(
                    DriverId::sqlite(),
                    "SQLite factice",
                    DriverFamily::Relational,
                ),
            }
        }
    }

    #[async_trait]
    impl Driver for DriverFactice {
        fn id(&self) -> DriverId {
            self.metadata.id.clone()
        }

        fn metadata(&self) -> &DriverMetadata {
            &self.metadata
        }

        fn capabilities(&self) -> Capabilities {
            Capabilities::SQL | Capabilities::RELATIONAL
        }

        async fn connect(
            &self,
            _config: &ConnectionConfig,
            _credentials: &Credentials,
            _cancel: &CancelToken,
        ) -> Result<Box<dyn Session>> {
            Ok(Box::new(SessionFactice::new(
                Capabilities::SQL | Capabilities::RELATIONAL | Capabilities::STREAMING,
            )))
        }
    }

    #[test]
    fn les_trois_traits_restent_objet_surs() {
        // Contrainte dure d'ARCHITECTURE §4.1. Une méthode générique la
        // romprait, et le message du compilateur ne dirait pas pourquoi.
        let driver: Box<dyn Driver> = Box::new(DriverFactice::new());
        let config = ConnectionConfig::new("atelier", DriverId::sqlite());
        let identifiants = Credentials::new();
        let jeton = CancelToken::new();

        let session: Box<dyn Session> =
            block_on(driver.connect(&config, &identifiants, &jeton)).expect("connexion factice");
        assert!(session.capabilities().contains(Capabilities::SQL));

        let curseur: Box<dyn Cursor> = block_on(session.execute(
            ExecRequest::new(oxyn_core::QueryLanguage::SQL, "SELECT 1"),
            &jeton,
        ))
        .expect("exécution factice");
        assert_eq!(curseur.schema().fields().len(), 1);

        block_on(session.close()).expect("fermeture");
    }

    #[test]
    fn un_langage_non_declare_est_refuse_pas_traduit() {
        let session = SessionFactice::new(Capabilities::SQL);
        let jeton = CancelToken::new();
        let demande = ExecRequest::new(oxyn_core::QueryLanguage::Cypher, "MATCH (n) RETURN n");

        // Voir `registry.rs` : `expect_err` exigerait `Debug` sur `dyn Cursor`.
        let err = match block_on(session.execute(demande, &jeton)) {
            Ok(_) => panic!("refus attendu : Cypher n'est pas déclaré"),
            Err(err) => err,
        };
        assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
        assert!(err.is_user_error(), "ce n'est pas un incident");
    }

    #[test]
    fn une_session_sans_transactions_le_dit_au_lieu_de_faire_semblant() {
        // DRIVER-CONTRACT §5 : ne pas savoir faire est une réponse acceptable ;
        // laisser croire qu'un ROLLBACK a annulé l'écriture ne l'est pas.
        let session = SessionFactice::new(Capabilities::SQL);
        let jeton = CancelToken::new();

        for issue in [
            block_on(session.begin(&jeton)),
            block_on(session.commit(&jeton)),
            block_on(session.rollback(&jeton)),
        ] {
            let err = issue.expect_err("refus attendu");
            assert!(matches!(err, OxynError::NotSupported { .. }), "{err:?}");
            assert!(err.to_string().contains("TRANSACTIONS"), "{err}");
        }
    }

    #[test]
    fn declarer_les_transactions_sans_les_implementer_est_un_bug_pas_un_succes() {
        // Le pire cas serait de réussir sans rien ouvrir.
        let session = SessionFactice::new(Capabilities::SQL | Capabilities::TRANSACTIONS);
        let jeton = CancelToken::new();

        let err = block_on(session.rollback(&jeton)).expect_err("refus attendu");
        assert!(matches!(err, OxynError::Internal(_)), "{err:?}");
        assert!(
            !err.is_user_error(),
            "c'est un bug du driver, pas une erreur d'usage"
        );
    }

    #[test]
    fn un_curseur_alimente_directement_un_tampon_de_resultats() {
        // C'est l'aiguillage entre `oxyn-driver` et `oxyn-data` : si cette
        // adaptation casse, plus rien ne relie un driver à l'écran.
        let curseur: Box<dyn Cursor> = Box::new(CurseurFactice::new(vec![lot(0, 3), lot(3, 2)]));
        let poignee = curseur.handle();

        let tampon = Arc::new(ResultBuffer::new(BatchSource::schema(&curseur), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = curseur;

        let issue = block_on(puits.drain(&mut source, &CancelToken::new())).expect("drainage");

        assert!(
            issue.is_complete(),
            "la source factice s'épuise : {issue:?}"
        );
        assert_eq!(tampon.row_count(), 5);
        assert_eq!(tampon.batch_count(), 2);

        // La poignée reste celle de l'exécution : c'est elle que vise
        // `Session::cancel`.
        assert_eq!(BatchSource::stats(&source).rows, 5);
        assert_eq!(source.handle(), poignee);
    }

    #[test]
    fn un_curseur_epuise_rend_none_plutot_qu_une_erreur() {
        let mut curseur = CurseurFactice::new(Vec::new());
        let lot = block_on(curseur.next_batch()).expect("pas d'erreur");
        assert!(lot.is_none());
        assert!(curseur.stats().is_empty());
    }
}
