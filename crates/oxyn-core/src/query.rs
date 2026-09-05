//! Ce qu'on demande à une base : langage, intention, risque, limites.
//!
//! Le SQL est **un cas parmi d'autres**, pas le défaut auquel les autres se
//! ramènent (ADR-0003). Une requête porte donc toujours un [`QueryLanguage`]
//! explicite ; un driver qui reçoit un langage qu'il ne déclare pas dans ses
//! capacités refuse, il ne traduit pas.
//!
//! [`StatementIntent`] et [`MutationRisk`] sont les deux entrées du `PolicyGate`.
//! Ils sont **déclarés** par qui construit la requête — l'analyseur de
//! `oxyn-query`, ou l'appelant — et le gate leur fait confiance. D'où la règle
//! qui gouverne ce module : dans le doute, on déclare le plus contraignant.
//! [`StatementIntent::Unknown`] compte pour mutant.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::value::ScalarValue;

/// Langage dans lequel une requête est écrite.
///
/// L'énumération est fermée : ces valeurs sont du vocabulaire du domaine, et un
/// langage se déclare aussi comme une capacité
/// ([`Capabilities`](crate::capabilities::Capabilities)).
///
// TODO(phase 4) : les plugins WASM pourront apporter un langage absent de cette
// liste (PLUGIN-CONTRACT). Ce sera une décision d'ADR, pas un `Other(String)`
// ajouté au fil de l'eau : un langage sans capacité correspondante n'a aucun
// moyen d'être refusé proprement.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QueryLanguage {
    /// SQL, dans un dialecte donné.
    Sql(SqlDialect),
    /// Cypher (Neo4j, Memgraph).
    Cypher,
    /// Gremlin (TinkerPop).
    Gremlin,
    /// Langage de requête documentaire (MongoDB).
    MongoQuery,
    /// Commandes Redis.
    RedisCommand,
    /// Query DSL (Elasticsearch, OpenSearch).
    SearchDsl,
    /// CQL (Cassandra).
    Cql,
    /// PartiQL (DynamoDB).
    PartiQl,
    /// InfluxQL (InfluxDB 1.x).
    InfluxQl,
    /// Flux (InfluxDB 2.x).
    Flux,
}

impl QueryLanguage {
    /// SQL ANSI, sans dialecte particulier.
    pub const SQL: Self = Self::Sql(SqlDialect::Ansi);

    /// Nom stable, utilisable dans un journal ou une interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Sql(_) => "sql",
            Self::Cypher => "cypher",
            Self::Gremlin => "gremlin",
            Self::MongoQuery => "mongo",
            Self::RedisCommand => "redis",
            Self::SearchDsl => "search-dsl",
            Self::Cql => "cql",
            Self::PartiQl => "partiql",
            Self::InfluxQl => "influxql",
            Self::Flux => "flux",
        }
    }

    /// Le dialecte SQL, s'il s'agit de SQL.
    #[must_use]
    pub const fn sql_dialect(&self) -> Option<SqlDialect> {
        match self {
            Self::Sql(d) => Some(*d),
            _ => None,
        }
    }
}

impl std::fmt::Display for QueryLanguage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sql(d) => write!(f, "sql/{d}"),
            other => f.write_str(other.as_str()),
        }
    }
}

/// Dialecte SQL.
///
/// Un dialecte n'est pas un produit : Redshift parle le protocole PostgreSQL
/// mais n'accepte pas la même grammaire, d'où une valeur distincte ici alors
/// qu'il n'y a **pas** de crate de driver distincte (ADR-0003).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SqlDialect {
    /// SQL standard, sans extension propriétaire.
    #[default]
    Ansi,
    /// PostgreSQL.
    Postgres,
    /// MySQL et MariaDB.
    MySql,
    /// SQLite.
    Sqlite,
    /// Microsoft SQL Server (T-SQL).
    SqlServer,
    /// Oracle (PL/SQL).
    Oracle,
    /// ClickHouse.
    ClickHouse,
    /// DuckDB.
    DuckDb,
    /// Snowflake.
    Snowflake,
    /// Google BigQuery.
    BigQuery,
    /// Amazon Redshift.
    Redshift,
}

impl SqlDialect {
    /// Nom stable du dialecte.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ansi => "ansi",
            Self::Postgres => "postgres",
            Self::MySql => "mysql",
            Self::Sqlite => "sqlite",
            Self::SqlServer => "sqlserver",
            Self::Oracle => "oracle",
            Self::ClickHouse => "clickhouse",
            Self::DuckDb => "duckdb",
            Self::Snowflake => "snowflake",
            Self::BigQuery => "bigquery",
            Self::Redshift => "redshift",
        }
    }
}

impl std::fmt::Display for SqlDialect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Ce que fait une instruction, du point de vue de la base.
///
/// L'énumération est **fermée**, à dessein, alors que la convention du dépôt est
/// de marquer les énumérations publiques `#[non_exhaustive]`. La matrice du
/// `PolicyGate` se lit cellule par cellule sur ces cinq valeurs : ajouter une
/// intention doit obliger chaque `match` à être revisité, donc être une rupture
/// visible plutôt qu'un `_ =>` qui décide en silence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StatementIntent {
    /// Lecture seule : `SELECT`, `EXPLAIN`, introspection.
    Read,
    /// Écriture de données : `INSERT`, `UPDATE`, `DELETE`, `MERGE`.
    Write,
    /// Modification de structure : `CREATE`, `ALTER`, `DROP`, `TRUNCATE`.
    Ddl,
    /// Modification de droits : `GRANT`, `REVOKE`, gestion des rôles.
    Grant,
    /// Intention indéterminée. C'est la valeur par défaut, et elle compte pour
    /// **mutante** : une instruction qu'aucun analyseur n'a su classer peut
    /// écrire, et le seul choix sûr est de la traiter comme telle.
    #[default]
    Unknown,
}

impl StatementIntent {
    /// L'instruction peut-elle modifier quelque chose ?
    ///
    /// [`Unknown`](Self::Unknown) répond `true`. Ce n'est pas une approximation
    /// commode : c'est la seule réponse qui ne laisse pas passer une écriture
    /// non reconnue (I-02).
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        !matches!(self, Self::Read)
    }

    /// L'instruction est-elle certainement en lecture seule ?
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        matches!(self, Self::Read)
    }

    /// Nom stable, pour l'audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Ddl => "ddl",
            Self::Grant => "grant",
            Self::Unknown => "unknown",
        }
    }
}

impl std::fmt::Display for StatementIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Forme de mutation dont la portée n'est pas bornée par l'instruction.
///
/// Ce n'est pas une mesure de gravité au jugé : chaque variante correspond à une
/// forme syntaxique reconnaissable dont l'effet ne se limite pas aux lignes que
/// l'utilisateur croit viser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MutationRisk {
    /// Rien de particulier : mutation bornée, ou lecture.
    #[default]
    None,
    /// `UPDATE` sans `WHERE` : toutes les lignes de la table.
    UnboundedUpdate,
    /// `DELETE` sans `WHERE` : toutes les lignes de la table.
    UnboundedDelete,
    /// `TRUNCATE` : vidage, souvent non journalisé et non annulable.
    Truncate,
    /// `DROP` d'un objet : la structure disparaît avec les données.
    DropObject,
}

impl MutationRisk {
    /// Y a-t-il un risque à signaler ?
    #[must_use]
    pub const fn is_some(&self) -> bool {
        !matches!(self, Self::None)
    }

    /// Motif d'approbation, montrable tel quel à l'utilisateur.
    #[must_use]
    pub const fn reason(&self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::UnboundedUpdate => Some("UPDATE sans clause WHERE : toutes les lignes"),
            Self::UnboundedDelete => Some("DELETE sans clause WHERE : toutes les lignes"),
            Self::Truncate => Some("TRUNCATE : vidage complet de la table"),
            Self::DropObject => Some("DROP : suppression de l'objet et de ses données"),
        }
    }
}

impl std::fmt::Display for MutationRisk {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason().unwrap_or("aucun risque signalé"))
    }
}

/// Bornes imposées à une exécution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecLimits {
    /// Nombre de lignes au-delà duquel le résultat est tronqué. `None` = pas de
    /// borne, ce qui n'est raisonnable que pour un export.
    pub max_rows: Option<usize>,
    /// Délai au bout duquel l'exécution est abandonnée **et annulée côté
    /// serveur**. Un délai qui ne fait qu'abandonner le futur laisse la requête
    /// tourner (DRIVER-CONTRACT §2).
    pub timeout: Option<Duration>,
    /// Interdit toute écriture pour cette exécution.
    pub read_only: bool,
}

impl ExecLimits {
    /// Délai par défaut.
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
    /// Nombre de lignes par défaut.
    pub const DEFAULT_MAX_ROWS: usize = 10_000;

    /// Limites sans borne ni protection : à réserver aux exports et aux
    /// traitements par lots, jamais à une exécution déclenchée par un clic.
    #[must_use]
    pub const fn unbounded() -> Self {
        Self {
            max_rows: None,
            timeout: None,
            read_only: false,
        }
    }

    /// Autorise l'écriture.
    #[must_use]
    pub fn writable(mut self) -> Self {
        self.read_only = false;
        self
    }

    /// Remplace le nombre maximal de lignes.
    #[must_use]
    pub fn with_max_rows(mut self, max_rows: impl Into<Option<usize>>) -> Self {
        self.max_rows = max_rows.into();
        self
    }

    /// Remplace le délai.
    #[must_use]
    pub fn with_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
        self.timeout = timeout.into();
        self
    }
}

impl Default for ExecLimits {
    /// Le défaut est **prudent**, pas commode : borné en lignes, borné en
    /// temps, et en lecture seule.
    ///
    /// Comme pour le marquage des connexions (SECURITY), le défaut est la
    /// valeur la plus contraignante. Une écriture est toujours quelque chose
    /// que l'appelant a demandé explicitement.
    fn default() -> Self {
        Self {
            max_rows: Some(Self::DEFAULT_MAX_ROWS),
            timeout: Some(Self::DEFAULT_TIMEOUT),
            read_only: true,
        }
    }
}

/// Une demande d'exécution complète.
///
/// Le `Debug` est **manuel** : les paramètres liés sont masqués. Le texte de la
/// requête est conservé — c'est sa *forme*, et l'observabilité l'autorise
/// explicitement — mais les valeurs liées sont exactement ce que I-03 interdit
/// de journaliser.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct ExecRequest {
    /// Langage de la requête.
    pub language: QueryLanguage,
    /// Texte de la requête, tel que l'utilisateur ou l'appelant l'a écrit.
    pub text: String,
    /// Paramètres liés, dans l'ordre des emplacements.
    pub params: Vec<ScalarValue>,
    /// Intention déclarée.
    pub intent: StatementIntent,
    /// Risque déclaré.
    pub risk: MutationRisk,
    /// Bornes d'exécution.
    pub limits: ExecLimits,
}

impl ExecRequest {
    /// Construit une demande avec les défauts prudents : intention
    /// [`Unknown`](StatementIntent::Unknown), aucun risque signalé, limites par
    /// défaut.
    ///
    /// L'intention par défaut étant mutante, une demande non qualifiée passe
    /// par une approbation plutôt que de s'exécuter silencieusement.
    #[must_use]
    pub fn new(language: QueryLanguage, text: impl Into<String>) -> Self {
        Self {
            language,
            text: text.into(),
            params: Vec::new(),
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            limits: ExecLimits::default(),
        }
    }

    /// Déclare l'intention.
    #[must_use]
    pub fn with_intent(mut self, intent: StatementIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Déclare le risque.
    #[must_use]
    pub fn with_risk(mut self, risk: MutationRisk) -> Self {
        self.risk = risk;
        self
    }

    /// Fixe les paramètres liés.
    #[must_use]
    pub fn with_params(mut self, params: Vec<ScalarValue>) -> Self {
        self.params = params;
        self
    }

    /// Fixe les limites.
    #[must_use]
    pub fn with_limits(mut self, limits: ExecLimits) -> Self {
        self.limits = limits;
        self
    }

    /// La demande peut-elle modifier quelque chose ?
    ///
    /// Vrai dès que l'intention est mutante **ou** qu'un risque est signalé :
    /// une déclaration incohérente (intention `Read` et risque `Truncate`) est
    /// tranchée du côté prudent.
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        self.intent.is_mutating() || self.risk.is_some()
    }
}

/// Emballage d'affichage : rend un compte, jamais les valeurs.
struct Masque(usize);

impl std::fmt::Debug for Masque {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<{} valeur(s) liée(s) masquée(s)>", self.0)
    }
}

impl std::fmt::Debug for ExecRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExecRequest")
            .field("language", &self.language)
            .field("text", &self.text)
            .field("params", &Masque(self.params.len()))
            .field("intent", &self.intent)
            .field("risk", &self.risk)
            .field("limits", &self.limits)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dans_le_doute_c_est_mutant() {
        assert!(StatementIntent::Unknown.is_mutating());
        assert!(!StatementIntent::Unknown.is_read_only());
        assert!(StatementIntent::default().is_mutating());
    }

    #[test]
    fn seule_la_lecture_n_est_pas_mutante() {
        assert!(!StatementIntent::Read.is_mutating());
        for intent in [
            StatementIntent::Write,
            StatementIntent::Ddl,
            StatementIntent::Grant,
            StatementIntent::Unknown,
        ] {
            assert!(intent.is_mutating(), "{intent} devrait être mutant");
        }
    }

    #[test]
    fn les_limites_par_defaut_sont_prudentes() {
        let limites = ExecLimits::default();
        assert!(limites.read_only, "le défaut doit interdire l'écriture");
        assert!(
            limites.max_rows.is_some(),
            "le défaut doit borner les lignes"
        );
        assert!(limites.timeout.is_some(), "le défaut doit borner le temps");
    }

    #[test]
    fn les_limites_sans_borne_sont_explicites() {
        let limites = ExecLimits::unbounded();
        assert_eq!(limites.max_rows, None);
        assert_eq!(limites.timeout, None);
        assert!(!limites.read_only);
    }

    #[test]
    fn une_demande_non_qualifiee_est_mutante() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "SELECT 1");
        assert!(demande.is_mutating(), "sans intention déclarée, on protège");
    }

    #[test]
    fn une_declaration_incoherente_est_tranchee_du_cote_prudent() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "TRUNCATE t")
            .with_intent(StatementIntent::Read)
            .with_risk(MutationRisk::Truncate);
        assert!(
            demande.is_mutating(),
            "un risque signalé l'emporte sur une intention de lecture"
        );
    }

    #[test]
    fn le_debug_masque_les_valeurs_liees() {
        let demande = ExecRequest::new(QueryLanguage::SQL, "SELECT * FROM users WHERE ssn = $1")
            .with_params(vec![ScalarValue::Text("123-45-6789".into())]);
        let rendu = format!("{demande:?}");
        assert!(
            !rendu.contains("123-45-6789"),
            "une valeur liée a fuité dans le Debug : {rendu}"
        );
        assert!(rendu.contains("1 valeur(s) liée(s) masquée(s)"));
        assert!(
            rendu.contains("SELECT * FROM users"),
            "la forme de la requête reste journalisable"
        );
    }

    #[test]
    fn le_langage_se_rend_avec_son_dialecte() {
        assert_eq!(
            QueryLanguage::Sql(SqlDialect::Postgres).to_string(),
            "sql/postgres"
        );
        assert_eq!(QueryLanguage::Cypher.to_string(), "cypher");
        assert_eq!(
            QueryLanguage::Sql(SqlDialect::Redshift).sql_dialect(),
            Some(SqlDialect::Redshift)
        );
        assert_eq!(QueryLanguage::Flux.sql_dialect(), None);
    }

    #[test]
    fn chaque_risque_porte_un_motif_affichable() {
        for risque in [
            MutationRisk::UnboundedUpdate,
            MutationRisk::UnboundedDelete,
            MutationRisk::Truncate,
            MutationRisk::DropObject,
        ] {
            assert!(risque.is_some());
            assert!(risque.reason().is_some(), "{risque:?} sans motif");
        }
        assert!(!MutationRisk::None.is_some());
        assert!(MutationRisk::None.reason().is_none());
    }
}
