//! Le vocabulaire du command bus (ADR-0004).
//!
//! Toute action possible dans Oxyn est une valeur [`Command`] typée. L'interface
//! ne fait rien d'autre que produire des `Command` ; **les outils exposés aux
//! agents sont exactement ces mêmes commandes.** Il n'y a donc pas d'API
//! « outils » parallèle à auditer séparément — et pas de second chemin
//! d'exécution mal journalisé.
//!
//! Chaque commande porte son [`Actor`] et sait répondre à trois questions dont
//! le `PolicyGate` dépend entièrement :
//!
//! * [`Command::intent`] — que fait-elle à la base ?
//! * [`Command::mutation_risk`] — sa portée est-elle bornée ?
//! * [`Command::target_connection`] — sur quoi agit-elle ?
//!
//! # Classer sur l'effet, jamais sur le nom
//!
//! `EXPLAIN ANALYZE` **exécute** la requête analysée, `DELETE` compris. Une vue
//! matérialisée se rafraîchit. Une fonction appelée dans un `SELECT` peut
//! écrire. C'est pourquoi [`Command::Execute`] ne devine rien : il lit
//! l'intention *déclarée* dans l'[`ExecRequest`], et l'intention par défaut est
//! [`StatementIntent::Unknown`], qui compte pour mutante.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::connection::ConnectionConfig;
use crate::ids::{
    AgentId, AgentSessionId, ConnectionId, DocumentId, ResultId, SessionId, StatementHandle,
    WorkspaceId,
};
use crate::preview::PreviewShape;
use crate::query::{ExecRequest, MutationRisk, StatementIntent};

/// Qui demande.
///
/// Énumération **fermée** : la dichotomie humain/agent est celle d'ADR-0004, et
/// c'est sur elle que repose toute la politique. Un troisième acteur — un
/// plugin, une automatisation — serait une décision d'ADR, pas une variante
/// ajoutée au fil de l'eau.
///
/// Il n'existe pas de « mode agent » global : l'acteur est porté par la
/// commande. Un état global se désynchronise, et c'est alors le journal d'audit
/// qui ment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Actor {
    /// L'utilisateur, par l'interface.
    Human,
    /// Un agent IA, identifié par son rôle et sa conversation.
    Agent {
        /// Quel agent (SQL, Schema, Performance…).
        id: AgentId,
        /// Dans quelle conversation.
        session: AgentSessionId,
    },
}

impl Actor {
    /// Est-ce un agent ?
    #[must_use]
    pub const fn is_agent(&self) -> bool {
        matches!(self, Self::Agent { .. })
    }

    /// Est-ce l'utilisateur ?
    #[must_use]
    pub const fn is_human(&self) -> bool {
        matches!(self, Self::Human)
    }

    /// Construit un acteur agent.
    #[must_use]
    pub const fn agent(id: AgentId, session: AgentSessionId) -> Self {
        Self::Agent { id, session }
    }
}

impl std::fmt::Display for Actor {
    /// Ne rend que le rôle : l'identifiant de conversation n'a rien à faire
    /// dans un message d'interface.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Human => f.write_str("humain"),
            Self::Agent { .. } => f.write_str("agent"),
        }
    }
}

/// Format d'export d'un jeu de résultats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ExportFormat {
    /// CSV.
    Csv,
    /// TSV.
    Tsv,
    /// JSON, un tableau d'objets.
    Json,
    /// JSON par lignes.
    JsonLines,
    /// Parquet.
    Parquet,
    /// Arrow IPC — le format dans lequel les résultats vivent déjà (ADR-0002),
    /// donc le seul export qui ne convertit rien.
    ArrowIpc,
    /// Instructions `INSERT`.
    Sql,
    /// Tableau Markdown.
    Markdown,
}

impl ExportFormat {
    /// Extension de fichier usuelle.
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Csv => "csv",
            Self::Tsv => "tsv",
            Self::Json => "json",
            Self::JsonLines => "jsonl",
            Self::Parquet => "parquet",
            Self::ArrowIpc => "arrow",
            Self::Sql => "sql",
            Self::Markdown => "md",
        }
    }
}

impl std::fmt::Display for ExportFormat {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.extension())
    }
}

/// One lazily loaded metadata level, independent of the catalog crate.
///
/// Names are raw identifiers, never SQL. The executor validates their paths.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "scope")]
#[non_exhaustive]
pub enum CatalogRefreshScope {
    /// Server identity and the first available hierarchy level.
    Root,
    /// Schemas in a catalog, or directly on a source without catalogs.
    Namespaces {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
    },
    /// Relation summaries only; no fields, indexes or foreign keys.
    Relations {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
    },
    /// Fields and metadata of one explicitly requested relation, plus its
    /// indexes and foreign keys when the session declares `INDEXES` and
    /// `FOREIGN_KEYS`.
    ///
    /// A source that declares neither leaves both unread rather than storing an
    /// empty list: the cache distinguishes "not read" from "none", and a view
    /// that confused them would claim a table has no index because nobody can
    /// tell. Constraints have their own explicit scope.
    Relation {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw relation name.
        relation: String,
    },
    /// Constraints of one relation, gated by the session's `CONSTRAINTS` capability.
    Constraints {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw relation name, never SQL.
        relation: String,
    },
    /// Foreign keys declared by relations that reference this target.
    IncomingForeignKeys {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw target relation name.
        relation: String,
    },
    /// Creation statements for one relation; this remains a metadata read.
    Definition {
        /// Absent when the source has no catalog level.
        catalog: Option<String>,
        /// Absent when the source has no schema level.
        namespace: Option<String>,
        /// Raw target relation name.
        relation: String,
    },
}

/// Longest focus accepted by [`Command::DescribeCatalog`], in bytes.
///
/// A focus is a few search words. The bound keeps what an agent writes there
/// from growing the audit journal or the search without limit.
pub const MAX_CATALOG_FOCUS_BYTES: usize = 256;

/// Une action soumise au bus.
///
/// Une commande fait **une** chose. Une commande « pratique » qui en emballe
/// trois contourne la politique : le `PolicyGate` ne décide que sur ce qu'il
/// voit, et il verrait une lecture là où il y a une écriture cachée.
///
/// L'énumération est **fermée**, contrairement à la convention du dépôt. Le bus
/// d'exécution dispatche sur un `match` exhaustif : ajouter une commande doit
/// faire échouer sa compilation, sinon il existe une commande que rien
/// n'exécute — ou pire, qu'un `_ =>` avale en silence après que le `PolicyGate`
/// l'a pourtant autorisée.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "command")]
pub enum Command {
    /// Ouvrir une session sur une connexion configurée.
    Connect {
        /// La connexion visée.
        connection: ConnectionId,
    },

    /// Fermer les sessions d'une connexion.
    Disconnect {
        /// La connexion visée.
        connection: ConnectionId,
    },

    /// Close one session without disconnecting sibling consoles or the catalog.
    CloseSession {
        /// Owning connection, checked before closing.
        connection: ConnectionId,
        /// Session to release.
        session: SessionId,
    },

    /// Déclarer où une session résout les noms non qualifiés.
    ///
    /// Les paliers voyagent en chaînes plutôt qu'en `CatalogPath` :
    /// `oxyn-core` ne dépend pas d'`oxyn-catalog`, et l'exécuteur reconstruit
    /// le chemin à la frontière, comme pour [`PreviewRelation`](Self::PreviewRelation).
    SetSessionContext {
        /// La connexion dont la politique s'applique.
        connection: ConnectionId,
        /// La session visée. Le contexte ne quitte jamais celle-ci.
        session: SessionId,
        /// Palier catalogue, quand le moteur en a un.
        catalog: Option<String>,
        /// Palier espace de noms — un schéma, là où il y en a.
        namespace: Option<String>,
    },

    /// Exécuter une instruction.
    Execute {
        /// La connexion visée.
        connection: ConnectionId,
        /// La session sur laquelle exécuter.
        session: SessionId,
        /// Ce qu'il faut exécuter. Encadré pour ne pas faire grossir toutes les
        /// autres variantes.
        request: Box<ExecRequest>,
    },

    /// Read a bounded preview using identifiers quoted by the session driver.
    PreviewRelation {
        /// Connection whose policy applies.
        connection: ConnectionId,
        /// Open session belonging to that connection.
        session: SessionId,
        /// Catalog from the metadata path, when present.
        catalog: Option<String>,
        /// Namespace from the metadata path, when present.
        namespace: Option<String>,
        /// Exact relation name, never SQL text.
        relation: String,
        /// Maximum rows requested, in `1..=1000`.
        limit: u32,
        /// Ordre, filtres et page demandés.
        ///
        /// Vide pour l'aperçu automatique d'une table qu'on vient de
        /// sélectionner. Le driver refuse ce qu'il ne sait pas faire plutôt que
        /// de l'ignorer : un filtre silencieusement abandonné rendrait des
        /// lignes que l'utilisateur croit avoir exclues
        /// ([ADR-0020](../../docs/adr/0020-apercu-trie-filtre-parcouru.md)).
        shape: PreviewShape,
    },

    /// Annuler une exécution en cours.
    ///
    /// Toujours autorisée : refuser une annulation ne protège rien et laisse
    /// une requête tourner côté serveur.
    Cancel {
        /// La connexion visée.
        connection: ConnectionId,
        /// L'exécution à interrompre.
        statement: StatementHandle,
    },

    /// Relire le catalogue d'une connexion.
    RefreshCatalog {
        /// La connexion visée.
        connection: ConnectionId,
    },

    /// Read the connection's **local** catalog cache, without contacting the
    /// server.
    ///
    /// What an agent asks for when it needs the structure of the database. The
    /// executor hands back the cache itself, never a rendering: what of it
    /// reaches a prompt is decided in `oxyn-ai`, under the connection's
    /// privacy tier ([I-04](../../../CLAUDE.md#i-04)). Its outcome carries no
    /// row value, because the catalog holds none.
    DescribeCatalog {
        /// Connection whose cache is read, and whose policy applies.
        connection: ConnectionId,
        /// Words to orient the selection of relations — a table name, a
        /// topic. Search terms, never query text: nothing composes a
        /// statement from them. Bounded by [`MAX_CATALOG_FOCUS_BYTES`].
        focus: Option<String>,
    },

    /// Refresh one metadata level through an existing session.
    RefreshCatalogScope {
        /// Connection whose policy and cache apply.
        connection: ConnectionId,
        /// Explicit lazy scope; never recursively loads descendants.
        scope: CatalogRefreshScope,
    },

    /// Inspect a bounded text page of one existing value, without executing SQL.
    InspectResultValue {
        /// Connection owning the result.
        connection: ConnectionId,
        /// Existing result.
        result: ResultId,
        /// Global zero-based row position.
        row: usize,
        /// Original Arrow column position, regardless of UI visibility.
        column: usize,
        /// UTF-8 byte offset in the formatted value.
        offset: usize,
    },

    /// Load one already-produced result page without executing a query.
    ReadResultPage {
        /// Connection that owns the result and supplies its policy.
        connection: ConnectionId,
        /// Existing result, including a result whose query has finished.
        result: ResultId,
        /// Zero-based batch position, never a row number or SQL offset.
        batch: usize,
    },

    /// Écrire un jeu de résultats dans un fichier.
    Export {
        /// La connexion d'où vient le résultat, pour l'audit.
        connection: ConnectionId,
        /// Le résultat à exporter.
        result: ResultId,
        /// Le format.
        format: ExportFormat,
        /// Le fichier de destination.
        destination: PathBuf,
    },

    /// Read versioned display preferences without contacting a database server.
    ReadWorkspacePreferences {
        /// Owning workspace.
        workspace: WorkspaceId,
    },
    /// Save a newer display snapshot. Only the human may change their interface.
    WriteWorkspacePreferences {
        /// Owning workspace.
        workspace: WorkspaceId,
        /// Validated, monotonically revised snapshot.
        snapshot: Box<crate::PreferencesSnapshot>,
    },

    /// List bounded local document summaries without loading query bodies.
    ListQueryDocuments {
        workspace: WorkspaceId,
        filter: Box<crate::DocumentFilter>,
    },
    /// Persist a draft and optionally its named copy, without executing it.
    SaveQueryDocument {
        workspace: WorkspaceId,
        update: Box<crate::QueryDocumentUpdate>,
    },
    /// Close a working document, retaining a barrier against late writes.
    CloseQueryDocument {
        /// Optional optimistic concurrency check, applied atomically with closing.
        #[serde(default)]
        expected_revision: Option<u64>,
        workspace: WorkspaceId,
        document: DocumentId,
        revision: u64,
        discard: bool,
    },
    /// Delete a named query locally; never deletes database objects.
    DeleteQueryDocument {
        workspace: WorkspaceId,
        document: DocumentId,
        revision: u64,
    },
    /// Search the user's local history, not a server query log.
    ReadHistory { filter: Box<crate::HistoryFilter> },
    /// Load a full selected historical statement separately from its list row.
    ReadHistoryEntry { entry: i64 },
    /// Record that the user inspected the server state of a write whose outcome
    /// was unknown, so it stops warning at each launch. Local only: nothing is
    /// sent to the server and nothing is retried (I-13).
    ReconcileHistoryEntry { entry: i64 },
    /// List the connections history recorded, including ones since removed.
    ///
    /// Separate from [`Command::ReadHistory`] on purpose: that one answers
    /// "which executions match these filters", this one answers "which
    /// connections may be filtered on". Folding them together would make a
    /// single command whose two answers have no reason to be paged alike.
    ListHistoryConnections {
        /// Workspace against which existence is resolved, not a history scope.
        workspace: WorkspaceId,
        /// Page bounds; history may name many connections.
        filter: crate::HistoryConnectionFilter,
    },
    /// Reopen an existing result without creating a query or a session.
    OpenRetainedResult {
        connection: ConnectionId,
        result: ResultId,
    },

    /// Ouvrir un document du workspace.
    OpenDocument {
        /// Le workspace propriétaire.
        workspace: WorkspaceId,
        /// Le document.
        document: DocumentId,
    },

    /// Écrire un document du workspace.
    ///
    /// N'atteint aucune base : voir [`Command::intent`] pour ce que cela
    /// implique — et ne l'implique pas.
    WriteDocument {
        /// Le workspace propriétaire.
        workspace: WorkspaceId,
        /// Le document.
        document: DocumentId,
        /// Le nouveau contenu.
        text: String,
    },

    /// Enregistrer une nouvelle connexion.
    CreateConnection {
        /// La configuration, sans secret en clair.
        config: Box<ConnectionConfig>,
    },

    /// Open a session on a configuration, then close it at once.
    ///
    /// Nothing is saved and no session outlives the command: it answers "would
    /// this connection open?" before the user commits to it (UX-SPEC,
    /// « Navigation du premier workspace »). It reaches the server, so the
    /// `PolicyGate` refuses it to an [`Actor::Agent`]: an agent that could
    /// point it at the host of its choice would hold an exfiltration channel,
    /// the same one [`CreateConnection`](Self::CreateConnection) guards.
    TestConnection {
        /// The configuration to try, carrying a secret reference and never a
        /// secret in clear.
        config: Box<ConnectionConfig>,
    },

    /// Modifier une connexion existante.
    UpdateConnection {
        /// La configuration complète après modification.
        config: Box<ConnectionConfig>,
    },

    /// Supprimer une connexion du workspace.
    DeleteConnection {
        /// La connexion visée.
        connection: ConnectionId,
    },

    /// List the AI providers declared on this machine, without contacting any
    /// of them.
    ///
    /// This is what decides whether the AI workspace exists at all: an empty
    /// list is the default installation, not a failure
    /// ([ADR-0023](../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    /// It resolves no name and reports no reach — a stored classification would
    /// be yesterday's DNS answer applied to today's request.
    ListAiProviders,

    /// Declare an AI provider, or replace the declaration bearing its
    /// identifier.
    SaveAiProvider {
        /// The declaration, carrying a secret reference and never a key.
        /// Boxed like [`CreateConnection`](Self::CreateConnection): it would
        /// otherwise widen every other variant.
        config: Box<crate::ai::AiProviderConfig>,
    },

    /// List the declared external agents.
    ///
    /// Separate from [`ListAiProviders`](Self::ListAiProviders) because the two
    /// declarations have nothing in common: an external agent has no endpoint,
    /// no model, and above all **no secret reference**
    /// ([ADR-0026](../../docs/adr/0026-agents-externes-acp.md)).
    ListExternalAgents,

    /// Declare an external agent, or replace the declaration bearing its
    /// identifier.
    SaveExternalAgent {
        /// The declaration: a command to run, its arguments, its environment.
        /// **Never a secret** — an external agent carries its own
        /// authentication, which is the point of the mode.
        agent: Box<crate::ai::ExternalAgentConfig>,
    },

    /// Remove a declared external agent.
    ///
    /// Nothing else is revoked: unlike a provider, there is no keychain entry
    /// behind it.
    RemoveExternalAgent {
        /// The declaration to forget.
        id: crate::ai::ProviderId,
    },

    /// Remove a declared AI provider. Documents keep the provenance they were
    /// written with.
    RemoveAiProvider {
        /// The declaration to forget.
        id: crate::ai::ProviderId,
    },
}

impl Command {
    /// Ce que la commande fait **à la base de données**.
    ///
    /// Trois choix méritent d'être explicités, parce qu'ils se relisent mal :
    ///
    /// * [`WriteDocument`](Self::WriteDocument) répond
    ///   [`Read`](StatementIntent::Read). Écrire une requête sauvegardée ne
    ///   touche aucune base ; la qualifier de mutante ferait apparaître une
    ///   demande d'approbation « base de données » là où il n'y a qu'un fichier
    ///   local, et une confirmation qui ne correspond à rien finit par être
    ///   cliquée sans être lue.
    /// * les commandes de gestion de connexion répondent
    ///   [`Ddl`](StatementIntent::Ddl). Ce n'est pas du DDL au sens SQL, mais
    ///   l'effet est du même ordre : un agent qui pourrait créer une connexion
    ///   vers l'hôte de son choix disposerait d'un canal d'exfiltration. Elles
    ///   passent donc par une approbation.
    /// * [`Cancel`](Self::Cancel) répond `Read` : annuler ne modifie rien, et
    ///   une annulation qu'il faut faire approuver n'est pas une annulation.
    /// * [`ReconcileHistoryEntry`](Self::ReconcileHistoryEntry) répond `Read`
    ///   pour la même raison que `WriteDocument` : elle n'écrit que l'état
    ///   local. Ce qui la protège est un **refus** à tout
    ///   [`Actor::Agent`] : elle affirme que l'utilisateur a vérifié le
    ///   serveur, et un agent qui pourrait l'affirmer ferait taire
    ///   l'avertissement d'une écriture que personne n'a regardée (I-13).
    /// * les commandes de déclaration de fournisseur d'IA répondent `Read`,
    ///   comme [`WriteDocument`](Self::WriteDocument) et pour la même raison :
    ///   elles n'atteignent aucune base, et une confirmation « base de
    ///   données » qui ne correspond à rien finit par être cliquée sans être
    ///   lue. Ce qui les protège n'est pas une approbation mais un **refus** :
    ///   le `PolicyGate` interdit [`SaveAiProvider`](Self::SaveAiProvider) et
    ///   [`RemoveAiProvider`](Self::RemoveAiProvider) à un
    ///   [`Actor::Agent`] — un agent qui déclarerait le point
    ///   d'accès par lequel il parle disposerait d'un canal d'exfiltration, et
    ///   c'est la catégorie d'action dont il n'a aucun usage légitime.
    #[must_use]
    pub fn intent(&self) -> StatementIntent {
        match self {
            Self::Execute { request, .. } => request.intent,
            Self::Connect { .. }
            | Self::Disconnect { .. }
            | Self::CloseSession { .. }
            | Self::SetSessionContext { .. }
            | Self::Cancel { .. }
            | Self::RefreshCatalog { .. }
            | Self::DescribeCatalog { .. }
            | Self::RefreshCatalogScope { .. }
            | Self::PreviewRelation { .. }
            | Self::ReadResultPage { .. }
            | Self::InspectResultValue { .. }
            | Self::Export { .. }
            | Self::ReadWorkspacePreferences { .. }
            | Self::WriteWorkspacePreferences { .. }
            | Self::ListQueryDocuments { .. }
            | Self::SaveQueryDocument { .. }
            | Self::CloseQueryDocument { .. }
            | Self::DeleteQueryDocument { .. }
            | Self::ReadHistory { .. }
            | Self::ReadHistoryEntry { .. }
            | Self::ReconcileHistoryEntry { .. }
            | Self::ListHistoryConnections { .. }
            | Self::OpenRetainedResult { .. }
            | Self::OpenDocument { .. }
            | Self::WriteDocument { .. }
            | Self::ListAiProviders
            | Self::SaveAiProvider { .. }
            | Self::RemoveAiProvider { .. }
            | Self::ListExternalAgents
            | Self::SaveExternalAgent { .. }
            | Self::RemoveExternalAgent { .. }
            // Opens and closes a session, writes nothing: the same effect as
            // `Connect`, on a configuration the workspace does not hold yet.
            | Self::TestConnection { .. } => StatementIntent::Read,
            Self::CreateConnection { .. }
            | Self::UpdateConnection { .. }
            | Self::DeleteConnection { .. } => StatementIntent::Ddl,
        }
    }

    /// La connexion visée, quand il y en a une.
    ///
    /// C'est ce que le `PolicyGate` utilise pour retrouver le marquage
    /// d'environnement et le drapeau de lecture seule.
    #[must_use]
    pub fn target_connection(&self) -> Option<ConnectionId> {
        match self {
            Self::ReadHistory { filter } => filter.connection,
            Self::Connect { connection }
            | Self::Disconnect { connection }
            | Self::CloseSession { connection, .. }
            | Self::SetSessionContext { connection, .. }
            | Self::Execute { connection, .. }
            | Self::Cancel { connection, .. }
            | Self::RefreshCatalog { connection }
            | Self::DescribeCatalog { connection, .. }
            | Self::RefreshCatalogScope { connection, .. }
            | Self::PreviewRelation { connection, .. }
            | Self::ReadResultPage { connection, .. }
            | Self::OpenRetainedResult { connection, .. }
            | Self::InspectResultValue { connection, .. }
            | Self::Export { connection, .. }
            | Self::DeleteConnection { connection } => Some(*connection),
            Self::CreateConnection { config }
            | Self::UpdateConnection { config }
            | Self::TestConnection { config } => Some(config.id),
            Self::ListQueryDocuments { .. }
            | Self::SaveQueryDocument { .. }
            | Self::CloseQueryDocument { .. }
            | Self::DeleteQueryDocument { .. }
            | Self::ReadHistoryEntry { .. }
            | Self::ReconcileHistoryEntry { .. }
            | Self::ListHistoryConnections { .. }
            | Self::OpenDocument { .. }
            | Self::WriteDocument { .. }
            | Self::ReadWorkspacePreferences { .. }
            | Self::WriteWorkspacePreferences { .. }
            // Un fournisseur est déclaré **par machine** (ADR-0023) : il n'a
            // pas de connexion visée, et le niveau de confidentialité qui
            // gouverne son usage reste celui de la connexion ouverte.
            | Self::ListAiProviders
            | Self::SaveAiProvider { .. }
            | Self::RemoveAiProvider { .. }
            | Self::ListExternalAgents
            | Self::SaveExternalAgent { .. }
            | Self::RemoveExternalAgent { .. } => None,
        }
    }

    /// Le risque déclaré, pour les commandes qui en portent un.
    #[must_use]
    pub fn mutation_risk(&self) -> MutationRisk {
        match self {
            Self::Execute { request, .. } => request.risk,
            _ => MutationRisk::None,
        }
    }

    /// La commande s'exécute-t-elle **contre le serveur** ?
    ///
    /// Distingue ce qui traverse la frontière externe de ce qui reste dans le
    /// workspace. Le drapeau de lecture seule d'une connexion ne peut protéger
    /// que la première catégorie — refuser de supprimer du workspace une
    /// connexion parce qu'elle est en lecture seule n'aurait aucun sens.
    #[must_use]
    pub const fn touches_database(&self) -> bool {
        matches!(
            self,
            Self::Connect { .. }
                | Self::Disconnect { .. }
                | Self::CloseSession { .. }
                | Self::SetSessionContext { .. }
                | Self::Execute { .. }
                | Self::Cancel { .. }
                | Self::RefreshCatalog { .. }
                | Self::RefreshCatalogScope { .. }
                | Self::PreviewRelation { .. }
                | Self::Export { .. }
                | Self::TestConnection { .. }
        )
    }

    /// La commande peut-elle modifier quelque chose ?
    ///
    /// Vrai dès que l'intention est mutante **ou** qu'un risque est signalé.
    /// Une déclaration incohérente est tranchée du côté prudent.
    #[must_use]
    pub fn is_mutating(&self) -> bool {
        self.intent().is_mutating() || self.mutation_risk().is_some()
    }

    /// Nom stable de la commande, pour le journal d'audit.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Connect { .. } => "Connect",
            Self::Disconnect { .. } => "Disconnect",
            Self::CloseSession { .. } => "CloseSession",
            Self::SetSessionContext { .. } => "SetSessionContext",
            Self::Execute { .. } => "Execute",
            Self::PreviewRelation { .. } => "PreviewRelation",
            Self::ReadResultPage { .. } => "ReadResultPage",
            Self::InspectResultValue { .. } => "InspectResultValue",
            Self::Cancel { .. } => "Cancel",
            Self::RefreshCatalog { .. } => "RefreshCatalog",
            Self::DescribeCatalog { .. } => "DescribeCatalog",
            Self::RefreshCatalogScope { .. } => "RefreshCatalogScope",
            Self::Export { .. } => "Export",
            Self::OpenDocument { .. } => "OpenDocument",
            Self::ListQueryDocuments { .. } => "ListQueryDocuments",
            Self::SaveQueryDocument { .. } => "SaveQueryDocument",
            Self::CloseQueryDocument { .. } => "CloseQueryDocument",
            Self::DeleteQueryDocument { .. } => "DeleteQueryDocument",
            Self::ReadHistory { .. } => "ReadHistory",
            Self::ReadHistoryEntry { .. } => "ReadHistoryEntry",
            Self::ReconcileHistoryEntry { .. } => "ReconcileHistoryEntry",
            Self::ListHistoryConnections { .. } => "ListHistoryConnections",
            Self::OpenRetainedResult { .. } => "OpenRetainedResult",
            Self::ReadWorkspacePreferences { .. } => "ReadWorkspacePreferences",
            Self::WriteWorkspacePreferences { .. } => "WriteWorkspacePreferences",
            Self::WriteDocument { .. } => "WriteDocument",
            Self::CreateConnection { .. } => "CreateConnection",
            Self::TestConnection { .. } => "TestConnection",
            Self::UpdateConnection { .. } => "UpdateConnection",
            Self::DeleteConnection { .. } => "DeleteConnection",
            Self::ListAiProviders => "ListAiProviders",
            Self::SaveAiProvider { .. } => "SaveAiProvider",
            Self::RemoveAiProvider { .. } => "RemoveAiProvider",
            Self::ListExternalAgents => "ListExternalAgents",
            Self::SaveExternalAgent { .. } => "SaveExternalAgent",
            Self::RemoveExternalAgent { .. } => "RemoveExternalAgent",
        }
    }

    /// Le texte de l'instruction, pour la prévisualisation d'une approbation.
    ///
    /// N'expose que ce que l'utilisateur a lui-même écrit : jamais les valeurs
    /// liées (I-03).
    #[must_use]
    pub fn statement_text(&self) -> Option<&str> {
        match self {
            Self::Execute { request, .. } => Some(&request.text),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::DriverId;
    use crate::query::QueryLanguage;

    fn execute(intent: StatementIntent, risk: MutationRisk) -> Command {
        Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "SELECT 1")
                    .with_intent(intent)
                    .with_risk(risk),
            ),
        }
    }

    #[test]
    fn preview_is_a_remote_read_with_its_own_audit_name() {
        let connection = ConnectionId::new();
        let command = Command::PreviewRelation {
            connection,
            session: SessionId::new(),
            catalog: Some("database".into()),
            namespace: Some("schema".into()),
            relation: "table\"; DROP TABLE audit; --".into(),
            limit: 200,
            // Une forme non triviale, pour que l'aller-retour sérialisé porte
            // vraiment sur elle : un `Default` passerait sans rien prouver.
            shape: crate::preview::PreviewShape {
                sort: vec![crate::preview::PreviewSort::descending("id")],
                predicate: Some("note LIKE '100%'".into()),
                offset: 200,
                columns: Some(vec!["id".into(), "note".into()]),
            },
        };
        assert_eq!(command.intent(), StatementIntent::Read);
        assert_eq!(command.target_connection(), Some(connection));
        assert_eq!(command.name(), "PreviewRelation");
        assert!(command.touches_database());
        assert!(!command.is_mutating());
        assert_eq!(command.statement_text(), None);
        let json = serde_json::to_string(&command).expect("serializable command");
        let decoded: Command = serde_json::from_str(&json).expect("typed round trip");
        assert_eq!(decoded, command);
    }

    #[test]
    fn catalog_scopes_are_reads_for_both_actors_even_in_production() {
        use crate::{DefaultPolicy, Environment, PolicyGate};
        let config = ConnectionConfig::new("fixture", DriverId::sqlite()).read_only();
        let gate = DefaultPolicy::new();
        gate.register(&config);
        for command in [
            Command::RefreshCatalog {
                connection: config.id,
            },
            Command::RefreshCatalogScope {
                connection: config.id,
                scope: CatalogRefreshScope::Root,
            },
            Command::RefreshCatalogScope {
                connection: config.id,
                scope: CatalogRefreshScope::Namespaces { catalog: None },
            },
            Command::RefreshCatalogScope {
                connection: config.id,
                scope: CatalogRefreshScope::Relations {
                    catalog: None,
                    namespace: None,
                },
            },
            Command::RefreshCatalogScope {
                connection: config.id,
                scope: CatalogRefreshScope::Relation {
                    catalog: None,
                    namespace: None,
                    relation: "users".into(),
                },
            },
        ] {
            assert_eq!(command.target_connection(), Some(config.id));
            assert!(command.touches_database());
            assert!(!command.is_mutating());
            assert!(command.statement_text().is_none());
            for actor in [
                Actor::Human,
                Actor::agent(AgentId::new(), AgentSessionId::new()),
            ] {
                assert!(
                    gate.authorize(&actor, &command, Environment::Production)
                        .is_allowed()
                );
            }
            let json = serde_json::to_string(&command).expect("serializable command");
            assert_eq!(
                serde_json::from_str::<Command>(&json).expect("round trip"),
                command
            );
        }
    }

    #[test]
    fn un_acteur_se_reconnait() {
        assert!(Actor::Human.is_human());
        assert!(!Actor::Human.is_agent());

        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        assert!(agent.is_agent());
        assert!(!agent.is_human());
    }

    #[test]
    fn l_affichage_d_un_acteur_ne_montre_pas_sa_conversation() {
        let session = AgentSessionId::new();
        let agent = Actor::agent(AgentId::new(), session);
        let rendu = agent.to_string();
        assert_eq!(rendu, "agent");
        assert!(!rendu.contains(&session.to_string()));
    }

    #[test]
    fn execute_relaie_l_intention_declaree_sans_la_deviner() {
        assert_eq!(
            execute(StatementIntent::Read, MutationRisk::None).intent(),
            StatementIntent::Read
        );
        assert_eq!(
            execute(StatementIntent::Ddl, MutationRisk::DropObject).intent(),
            StatementIntent::Ddl
        );
    }

    #[test]
    fn un_explain_analyze_declare_ecriture_reste_une_ecriture() {
        // Le texte dit « EXPLAIN », l'effet est une suppression. C'est
        // l'intention déclarée qui fait foi, jamais le nom de l'instruction.
        let cmd = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "EXPLAIN ANALYZE DELETE FROM events")
                    .with_intent(StatementIntent::Write)
                    .with_risk(MutationRisk::UnboundedDelete),
            ),
        };
        assert!(cmd.is_mutating());
        assert_eq!(cmd.mutation_risk(), MutationRisk::UnboundedDelete);
    }

    #[test]
    fn une_execution_non_qualifiee_est_mutante() {
        let cmd = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(ExecRequest::new(QueryLanguage::SQL, "CALL faire_le_truc()")),
        };
        assert_eq!(cmd.intent(), StatementIntent::Unknown);
        assert!(cmd.is_mutating(), "dans le doute, on protège");
    }

    #[test]
    fn les_commandes_de_lecture_ne_sont_pas_mutantes() {
        let c = ConnectionId::new();
        for cmd in [
            Command::Connect { connection: c },
            Command::Disconnect { connection: c },
            Command::RefreshCatalog { connection: c },
            Command::Cancel {
                connection: c,
                statement: StatementHandle::new(),
            },
            Command::OpenDocument {
                workspace: WorkspaceId::new(),
                document: DocumentId::new(),
            },
        ] {
            assert!(
                !cmd.is_mutating(),
                "{} devrait être non mutante",
                cmd.name()
            );
        }
    }

    #[test]
    fn annuler_n_est_jamais_une_mutation() {
        // Une annulation qu'il faut faire approuver n'est pas une annulation.
        let cmd = Command::Cancel {
            connection: ConnectionId::new(),
            statement: StatementHandle::new(),
        };
        assert_eq!(cmd.intent(), StatementIntent::Read);
        assert!(!cmd.is_mutating());
    }

    #[test]
    fn la_gestion_des_connexions_est_traitee_comme_du_ddl() {
        let cfg = ConnectionConfig::new("nouvelle", DriverId::postgres());
        let cmd = Command::CreateConnection {
            config: Box::new(cfg.clone()),
        };
        assert_eq!(cmd.intent(), StatementIntent::Ddl);
        assert!(cmd.is_mutating());
        assert_eq!(cmd.target_connection(), Some(cfg.id));
        assert!(
            !cmd.touches_database(),
            "créer une connexion n'atteint aucun serveur"
        );
    }

    #[test]
    fn tester_une_connexion_est_une_lecture_du_serveur_refusee_a_un_agent() {
        use crate::{DefaultPolicy, Environment, PolicyGate};

        let config = ConnectionConfig::new("à tester", DriverId::postgres())
            .with_secret_ref("keychain://oxyn/essai");
        let cmd = Command::TestConnection {
            config: Box::new(config.clone()),
        };
        assert_eq!(cmd.intent(), StatementIntent::Read);
        assert!(!cmd.is_mutating(), "nothing is saved or written");
        assert!(cmd.touches_database(), "the server is reached");
        assert_eq!(cmd.target_connection(), Some(config.id));
        assert_eq!(cmd.name(), "TestConnection");
        assert_eq!(cmd.statement_text(), None);

        // The configuration is unknown to the gate, and marked production by
        // default: the human may still try it, since nothing is written.
        let gate = DefaultPolicy::new();
        assert!(
            gate.authorize(&Actor::Human, &cmd, Environment::Production)
                .is_allowed()
        );
        // An agent may not, even on a local connection: choosing the host is
        // the exfiltration channel, whatever the marking.
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());
        let decision = gate.authorize(&agent, &cmd, Environment::Local);
        assert!(decision.is_denied(), "{decision:?}");
        assert!(!decision.requires_approval(), "a refusal (I-02)");

        // I-03: the reference is not in the `Debug`, and a round trip keeps
        // the command whole.
        assert!(!format!("{cmd:?}").contains("keychain://oxyn/essai"));
        let json = serde_json::to_string(&cmd).expect("serializable command");
        assert_eq!(
            serde_json::from_str::<Command>(&json).expect("typed round trip"),
            cmd
        );
    }

    #[test]
    fn ecrire_un_document_ne_touche_aucune_base() {
        let cmd = Command::WriteDocument {
            workspace: WorkspaceId::new(),
            document: DocumentId::new(),
            text: "SELECT 1".into(),
        };
        assert_eq!(cmd.intent(), StatementIntent::Read);
        assert_eq!(cmd.target_connection(), None);
        assert!(!cmd.touches_database());
    }

    #[test]
    fn toute_commande_visant_un_serveur_nomme_sa_connexion() {
        // Sans cela, le PolicyGate ne pourrait pas retrouver le marquage
        // d'environnement ni le drapeau de lecture seule.
        let c = ConnectionId::new();
        let commandes = [
            Command::Connect { connection: c },
            Command::Disconnect { connection: c },
            Command::RefreshCatalog { connection: c },
            Command::Cancel {
                connection: c,
                statement: StatementHandle::new(),
            },
            Command::Export {
                connection: c,
                result: ResultId::new(),
                format: ExportFormat::Csv,
                destination: PathBuf::from("/tmp/export.csv"),
            },
            execute(StatementIntent::Read, MutationRisk::None),
        ];
        for cmd in commandes {
            assert!(cmd.touches_database(), "{}", cmd.name());
            assert!(
                cmd.target_connection().is_some(),
                "{} ne nomme pas sa connexion",
                cmd.name()
            );
        }
    }

    #[test]
    fn la_previsualisation_ne_donne_que_le_texte_ecrit() {
        let cmd = execute(StatementIntent::Read, MutationRisk::None);
        assert_eq!(cmd.statement_text(), Some("SELECT 1"));
        assert_eq!(
            Command::Connect {
                connection: ConnectionId::new()
            }
            .statement_text(),
            None
        );
    }

    #[test]
    fn le_debug_d_une_commande_ne_montre_pas_les_valeurs_liees() {
        let cmd = Command::Execute {
            connection: ConnectionId::new(),
            session: SessionId::new(),
            request: Box::new(
                ExecRequest::new(QueryLanguage::SQL, "INSERT INTO t VALUES ($1)").with_params(
                    vec![crate::value::ScalarValue::Text(
                        "secret-de-l-utilisateur".into(),
                    )],
                ),
            ),
        };
        let rendu = format!("{cmd:?}");
        assert!(
            !rendu.contains("secret-de-l-utilisateur"),
            "valeur liée fuitée : {rendu}"
        );

        // Le `Debug` n'est qu'un des six canaux. Un fichier de workspace en est
        // un autre, et une protection qui ne tient que sur le premier serait
        // défaite par le premier appelant qui persiste une commande.
        let ecrit = serde_json::to_string(&cmd).expect("commande sérialisable");
        assert!(
            !ecrit.contains("secret-de-l-utilisateur"),
            "valeur liée écrite dans un fichier : {ecrit}"
        );
        let relue: Command = serde_json::from_str(&ecrit).expect("commande relisible");
        let Command::Execute { request, .. } = &relue else {
            panic!("la variante est conservée")
        };
        assert!(
            request.params.is_empty(),
            "une commande relue revient sans ses valeurs, et le serveur la refusera"
        );
        assert_eq!(request.text, "INSERT INTO t VALUES ($1)");
    }

    #[test]
    fn declarer_un_fournisseur_n_atteint_aucune_base_et_reste_refuse_a_un_agent() {
        use crate::ai::{AiProviderConfig, AiProviderKind, ProviderId};
        use crate::{DefaultPolicy, Environment, PolicyGate};

        let config = AiProviderConfig::new(
            ProviderId::ollama(),
            AiProviderKind::OpenAiCompatible,
            "Ollama du portable",
            "http://localhost:11434/v1",
            "llama3.2",
        );
        let gate = DefaultPolicy::new();
        let agent = Actor::agent(AgentId::new(), AgentSessionId::new());

        for cmd in [
            Command::ListAiProviders,
            Command::SaveAiProvider {
                config: Box::new(config.clone()),
            },
            Command::RemoveAiProvider {
                id: config.id.clone(),
            },
        ] {
            // Un fournisseur se déclare par machine : aucune connexion visée,
            // aucun serveur atteint (ADR-0023).
            assert_eq!(cmd.target_connection(), None, "{}", cmd.name());
            assert!(!cmd.touches_database(), "{}", cmd.name());
            assert!(!cmd.is_mutating(), "{}", cmd.name());
            assert_eq!(cmd.statement_text(), None, "{}", cmd.name());

            // L'humain n'a pas à confirmer un réglage local : une confirmation
            // qui ne correspond à rien finit par être cliquée sans être lue.
            assert!(
                gate.authorize(&Actor::Human, &cmd, Environment::Production)
                    .is_allowed(),
                "{}",
                cmd.name()
            );

            // L'agent, lui, est refusé sur ce qui **écrit** la déclaration :
            // le point d'accès par lequel il parle ne se choisit pas lui-même.
            let decision = gate.authorize(&agent, &cmd, Environment::Local);
            if matches!(cmd, Command::ListAiProviders) {
                assert!(decision.is_allowed(), "lister ne déclare rien");
            } else {
                assert!(decision.is_denied(), "{} : {decision:?}", cmd.name());
                assert!(
                    !decision.requires_approval(),
                    "un refus, jamais une confirmation renforcée (I-02)"
                );
            }

            let json = serde_json::to_string(&cmd).expect("commande sérialisable");
            assert_eq!(
                serde_json::from_str::<Command>(&json).expect("aller-retour typé"),
                cmd
            );
        }
    }

    #[test]
    fn une_commande_de_fournisseur_ne_transporte_pas_de_cle() {
        // I-03 : le seul champ prévu pour le trousseau est une référence, et le
        // `Debug` de la commande ne la rend pas.
        use crate::ai::{AiProviderConfig, AiProviderKind, ProviderId};

        let cmd = Command::SaveAiProvider {
            config: Box::new(
                AiProviderConfig::new(
                    ProviderId::openai(),
                    AiProviderKind::OpenAi,
                    "OpenAI",
                    "https://api.openai.com/v1",
                    "gpt-5",
                )
                .with_secret_ref("keychain://oxyn/openai"),
            ),
        };
        let rendu = format!("{cmd:?}");
        assert!(
            !rendu.contains("keychain://oxyn/openai"),
            "référence fuitée : {rendu}"
        );
        assert!(rendu.contains("api.openai.com"), "{rendu}");
    }

    #[test]
    fn les_noms_d_audit_sont_uniques() {
        let c = ConnectionId::new();
        let noms = [
            Command::Connect { connection: c }.name(),
            Command::Disconnect { connection: c }.name(),
            execute(StatementIntent::Read, MutationRisk::None).name(),
            Command::DeleteConnection { connection: c }.name(),
        ];
        let mut tries: Vec<&str> = noms.to_vec();
        tries.sort_unstable();
        tries.dedup();
        assert_eq!(
            tries.len(),
            noms.len(),
            "deux commandes portent le même nom"
        );
    }
}
