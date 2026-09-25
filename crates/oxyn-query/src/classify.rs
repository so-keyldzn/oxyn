//! Classification d'intention : ce qui alimente le `PolicyGate`.
//!
//! C'est le point critique de la crate. Le `PolicyGate` d'`oxyn-core` fait
//! confiance à l'intention qu'on lui donne (`oxyn-core::query`) ; c'est ici
//! qu'elle est établie. Si ce module classe mal, la sécurité tombe : un
//! `DELETE` classé `Read` s'exécute en production sans confirmation.
//!
//! # Les trois règles qui gouvernent ce module
//!
//! **Ce qui ne se lit pas est mutant.** Un texte que `sqlparser` refuse, une
//! instruction dont la forme n'est pas reconnue, un lot vide : tout cela donne
//! [`StatementIntent::Unknown`], qui `is_mutating()` (I-02). Il n'existe aucun
//! chemin par lequel un échec d'analyse produit `Read`.
//!
//! **Le nom de l'instruction ne suffit pas.** `EXPLAIN ANALYZE DELETE FROM t`
//! commence par `EXPLAIN` et supprime toutes les lignes de la table — c'est
//! l'exemple que I-07 donne. `WITH x AS (DELETE FROM t RETURNING *) SELECT *`
//! commence par `WITH` et fait la même chose. On classe donc sur l'AST, pas sur
//! le premier mot, et on descend dans les clauses `WITH` et les corps
//! d'`EXPLAIN`.
//!
//! **Un filet sous l'AST.** Après la classification, une instruction jugée
//! `Read` est relue : si un mot-clé mutant apparaît hors chaîne et hors
//! commentaire alors que l'AST n'a vu aucune mutation, c'est que l'analyse a
//! manqué quelque chose, et l'instruction retombe en `Unknown`. Ce filet ne
//! peut que **restreindre** ; il ne relâche jamais rien.
//!
//! # Agrégation d'un lot
//!
//! Un lot prend l'intention la plus élevée de ses instructions :
//! `Read < Write < {Ddl, Unknown} < Grant`. À égalité entre `Ddl` et `Unknown`,
//! `Unknown` l'emporte : les deux sont aussi contraignants, et `Unknown` dit la
//! vérité — quelque chose n'a pas été compris.

use std::cmp::Ordering;
use std::ops::Range;

use oxyn_core::{ExecRequest, MutationRisk, QueryLanguage, SqlDialect, StatementIntent};
use serde::{Deserialize, Serialize};
use sqlparser::ast::{
    BinaryOperator, CopyIntoSnowflakeKind, Expr, ObjectType, Query, Set, SetExpr, Statement,
    UnaryOperator, UtilityOption, Value,
};
use sqlparser::dialect::Dialect;
use sqlparser::parser::Parser;

use crate::dialect::parser_dialect;
use crate::error::QueryError;
use crate::split::{self, Fragment, Word};

/// Sur quoi repose la classification d'une instruction.
///
/// Sert au journal et à l'interface : une approbation demandée parce qu'on n'a
/// pas su lire l'instruction ne se présente pas comme une approbation demandée
/// parce qu'elle supprime une table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Basis {
    /// L'instruction a été lue et sa forme reconnue.
    Ast,
    /// L'instruction n'a pas pu être lue : classée `Unknown` par défaut.
    Unparsed,
    /// L'AST disait `Read`, un mot-clé mutant nu a été trouvé : déclassée.
    KeywordSweep,
}

impl Basis {
    /// La classification vient-elle d'une lecture réussie et non corrigée ?
    #[must_use]
    pub const fn is_certain(&self) -> bool {
        matches!(self, Self::Ast)
    }

    /// Nom stable, pour l'audit.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Ast => "ast",
            Self::Unparsed => "unparsed",
            Self::KeywordSweep => "keyword-sweep",
        }
    }
}

impl std::fmt::Display for Basis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Une instruction d'un lot, classée.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementInfo {
    /// Le texte de l'instruction, sans son point-virgule.
    pub text: String,
    /// Ses bornes en octets dans le lot d'origine, pour l'éditeur.
    pub span: Range<usize>,
    /// Ce qu'elle fait.
    pub intent: StatementIntent,
    /// Le risque que sa forme laisse voir.
    pub risk: MutationRisk,
    /// D'où vient cette classification.
    pub basis: Basis,
    /// Message de l'analyseur, quand il a refusé de lire.
    pub error: Option<String>,
    /// Whether it begins, ends or marks a transaction: see
    /// [`ExecRequest::transaction_control`].
    pub transaction_control: bool,
}

/// Le résultat de l'analyse d'un lot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    /// L'intention du lot : la plus élevée de ses instructions.
    pub intent: StatementIntent,
    /// Le risque du lot : le plus grave de ses instructions.
    pub risk: MutationRisk,
    /// Whether any of its statements controls a transaction.
    pub transaction_control: bool,
    /// Le détail, instruction par instruction, dans l'ordre du texte.
    pub statements: Vec<StatementInfo>,
}

impl Classification {
    /// Le lot peut-il modifier quelque chose ?
    #[must_use]
    pub const fn is_mutating(&self) -> bool {
        self.intent.is_mutating() || self.risk.is_some()
    }

    /// Le lot est-il certainement en lecture seule ?
    #[must_use]
    pub const fn is_read_only(&self) -> bool {
        self.intent.is_read_only() && !self.risk.is_some()
    }

    /// Toutes les instructions ont-elles été lues et reconnues ?
    ///
    /// `false` signale qu'au moins une instruction est classée par défaut : la
    /// décision reste sûre, mais l'interface a intérêt à le dire.
    #[must_use]
    pub fn is_fully_understood(&self) -> bool {
        !self.statements.is_empty() && self.statements.iter().all(|s| s.basis.is_certain())
    }

    /// Le nombre d'instructions du lot.
    #[must_use]
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Le lot ne contient-il aucune instruction exécutable ?
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// Les instructions qui portent un risque, dans l'ordre du texte.
    pub fn risky(&self) -> impl Iterator<Item = &StatementInfo> {
        self.statements.iter().filter(|s| s.risk.is_some())
    }

    /// Réécrit l'intention et le risque d'une demande d'exécution.
    ///
    /// C'est le geste que fait `oxyn-exec` avant de soumettre au `PolicyGate` :
    /// l'intention **déclarée** par l'appelant n'est pas digne de confiance, un
    /// agent ne peut pas s'auto-déclarer en lecture seule (ARCHITECTURE §8).
    #[must_use]
    pub fn qualify(&self, request: ExecRequest) -> ExecRequest {
        let mut request = request.with_intent(self.intent).with_risk(self.risk);
        request.transaction_control = self.transaction_control;
        request
    }

    /// Le lot dont on ne sait rien : une instruction opaque, `Unknown`.
    ///
    /// Its language is not SQL, so no transaction verb of SQL can be looked
    /// for: `Unknown` already sends it to a human's approval, preview shown.
    fn opaque(text: &str, error: String) -> Self {
        let trimmed = text.trim();
        let start = text.len() - text.trim_start().len();
        Self {
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            transaction_control: false,
            statements: vec![StatementInfo {
                text: trimmed.to_owned(),
                span: start..start + trimmed.len(),
                intent: StatementIntent::Unknown,
                risk: MutationRisk::None,
                basis: Basis::Unparsed,
                error: Some(error),
                transaction_control: false,
            }],
        }
    }
}

/// Analyse un lot SQL et en déduit intention et risque.
///
/// Ne renvoie jamais d'erreur : ce qui ne se lit pas est classé
/// [`Unknown`](StatementIntent::Unknown). Pour obtenir le message de
/// l'analyseur, voir [`crate::validate`].
///
/// ```
/// use oxyn_core::{MutationRisk, SqlDialect, StatementIntent};
/// use oxyn_query::classify;
///
/// // Le piège : `EXPLAIN ANALYZE` exécute réellement ce qu'il analyse.
/// let lu = classify("EXPLAIN ANALYZE DELETE FROM commandes", SqlDialect::Postgres);
/// assert_eq!(lu.intent, StatementIntent::Write);
/// assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
///
/// // Sans `ANALYZE`, seul le plan est calculé.
/// let lu = classify("EXPLAIN DELETE FROM commandes", SqlDialect::Postgres);
/// assert_eq!(lu.intent, StatementIntent::Read);
/// ```
#[must_use]
pub fn classify(sql: &str, dialect: SqlDialect) -> Classification {
    let grammar = parser_dialect(dialect);
    let mut statements: Vec<StatementInfo> = Vec::new();
    let mut aggregate: Option<Facts> = None;

    for fragment in split::split(sql, dialect) {
        let info = classify_fragment(&fragment, dialect, grammar);
        let facts = Facts::new(info.intent, info.risk);
        aggregate = Some(match aggregate {
            Some(previous) => previous.merge(facts),
            None => facts,
        });
        statements.push(info);
    }

    // Un lot sans instruction — texte vide, que des commentaires, ou découpage
    // qui a tout écarté — vaut `Unknown` et non `Read`. Un bogue du découpage
    // qui perdrait des instructions ne doit pas ouvrir la porte : « aucune
    // instruction » n'est pas une preuve de lecture seule.
    let facts = aggregate.unwrap_or(Facts::UNKNOWN);

    Classification {
        intent: facts.intent,
        risk: facts.risk,
        transaction_control: statements.iter().any(|s| s.transaction_control),
        statements,
    }
}

/// Analyse un texte dans son langage déclaré.
///
/// Un langage qui n'est pas du SQL n'est pas analysé : le résultat est
/// `Unknown`, donc mutant. Supposer qu'une commande Redis ou un Cypher
/// « ressemble à une lecture » serait exactement l'erreur que I-02 interdit.
#[must_use]
pub fn classify_language(language: QueryLanguage, sql: &str) -> Classification {
    match language.sql_dialect() {
        Some(dialect) => classify(sql, dialect),
        None => Classification::opaque(
            sql,
            format!("language not parsed by oxyn-query: {language}"),
        ),
    }
}

/// Vérifie que tout le texte se lit dans le dialecte demandé.
///
/// C'est le pendant strict de [`classify`] : là où celui-ci retombe en
/// silence sur `Unknown`, celui-ci rend le message de l'analyseur et les
/// bornes de l'instruction fautive, pour que l'éditeur puisse la souligner.
///
/// **Ne jamais s'en servir pour décider d'une autorisation.** Un texte qui
/// échoue ici n'est pas « refusé » : il est `Unknown`, donc soumis à
/// approbation comme n'importe quelle mutation.
///
/// # Erreurs
///
/// [`QueryError::Syntax`] à la première instruction que l'analyseur refuse.
pub fn validate(sql: &str, dialect: SqlDialect) -> Result<(), QueryError> {
    let grammar = parser_dialect(dialect);
    for fragment in split::split(sql, dialect) {
        if fragment.unreadable_comment {
            return Err(QueryError::Syntax {
                message: UNREADABLE_COMMENT.to_owned(),
                span: fragment.span,
            });
        }
        if let Err(err) = Parser::parse_sql(grammar, fragment.text) {
            return Err(QueryError::Syntax {
                message: err.to_string(),
                span: fragment.span,
            });
        }
    }
    Ok(())
}

/// Reclassifie une demande d'exécution à partir de son seul texte.
///
/// `oxyn-exec` appelle ceci avant chaque soumission au `PolicyGate` :
/// l'intention portée par la `Command` vient de l'appelant, et un agent est un
/// appelant (ARCHITECTURE §8, I-07).
#[must_use]
pub fn reclassify(request: &ExecRequest) -> Classification {
    classify_language(request.language, &request.text)
}

// ─────────────────────────────────────────────────────────────────────────────
// Faits d'une instruction
// ─────────────────────────────────────────────────────────────────────────────

/// Intention et risque, transportés ensemble parce qu'ils se déduisent
/// ensemble et s'agrègent ensemble.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Facts {
    intent: StatementIntent,
    risk: MutationRisk,
}

impl Facts {
    const READ: Self = Self::new(StatementIntent::Read, MutationRisk::None);
    const WRITE: Self = Self::new(StatementIntent::Write, MutationRisk::None);
    const DDL: Self = Self::new(StatementIntent::Ddl, MutationRisk::None);
    const GRANT: Self = Self::new(StatementIntent::Grant, MutationRisk::None);
    const UNKNOWN: Self = Self::new(StatementIntent::Unknown, MutationRisk::None);

    const fn new(intent: StatementIntent, risk: MutationRisk) -> Self {
        Self { intent, risk }
    }

    /// Le pire des deux, champ par champ.
    fn merge(self, other: Self) -> Self {
        Self {
            intent: max_intent(self.intent, other.intent),
            risk: max_risk(self.risk, other.risk),
        }
    }
}

/// `Read < Write < {Ddl, Unknown} < Grant`.
///
/// `Unknown` est placé au niveau de `Ddl` : c'est ce que dit ARCHITECTURE §8.
const fn intent_rank(intent: StatementIntent) -> u8 {
    match intent {
        StatementIntent::Read => 0,
        StatementIntent::Write => 1,
        StatementIntent::Ddl | StatementIntent::Unknown => 2,
        StatementIntent::Grant => 3,
    }
}

fn max_intent(a: StatementIntent, b: StatementIntent) -> StatementIntent {
    match intent_rank(a).cmp(&intent_rank(b)) {
        Ordering::Greater => a,
        Ordering::Less => b,
        // À rang égal, `Unknown` gagne : aussi contraignant, et plus honnête.
        Ordering::Equal => {
            if a == StatementIntent::Unknown || b == StatementIntent::Unknown {
                StatementIntent::Unknown
            } else {
                a
            }
        }
    }
}

/// Gravité d'un risque, du moins au plus grave.
///
/// L'ordre ne sert qu'à choisir **le motif affiché** quand un lot en porte
/// plusieurs : pour le `PolicyGate`, tout risque non nul vaut approbation.
/// `DROP` passe devant `TRUNCATE` parce qu'il emporte la structure avec les
/// données ; une variante ajoutée plus tard à l'énumération (elle est
/// `#[non_exhaustive]`) est traitée comme la plus grave, faute de savoir.
const fn risk_rank(risk: MutationRisk) -> u8 {
    match risk {
        MutationRisk::None => 0,
        MutationRisk::UnboundedUpdate => 1,
        MutationRisk::UnboundedDelete => 2,
        MutationRisk::Truncate => 3,
        MutationRisk::DropObject => 4,
        _ => u8::MAX,
    }
}

fn max_risk(a: MutationRisk, b: MutationRisk) -> MutationRisk {
    if risk_rank(b) > risk_rank(a) { b } else { a }
}

// ─────────────────────────────────────────────────────────────────────────────
// Classification d'un fragment
// ─────────────────────────────────────────────────────────────────────────────

/// Why a fragment holding an [`unreadable_comment`](Fragment::unreadable_comment)
/// is not read.
const UNREADABLE_COMMENT: &str = "a line comment contains a carriage return followed by text; \
     whether it ends there depends on the server, so the statements it holds cannot be known";

fn classify_fragment(
    fragment: &Fragment<'_>,
    dialect: SqlDialect,
    grammar: &dyn Dialect,
) -> StatementInfo {
    // The parser's own reading of the comment is no better than the splitter's:
    // for this dialect, nobody checked which one the server shares.
    if fragment.unreadable_comment {
        return StatementInfo {
            text: fragment.text.to_owned(),
            span: fragment.span.clone(),
            intent: StatementIntent::Unknown,
            risk: MutationRisk::None,
            basis: Basis::Unparsed,
            error: Some(UNREADABLE_COMMENT.to_owned()),
            // Whether a `COMMIT` hides behind the comment depends on the
            // server: what cannot be read counts as the worst it could be.
            transaction_control: true,
        };
    }
    let (facts, basis, error) = match Parser::parse_sql(grammar, fragment.text) {
        Ok(parsed) if !parsed.is_empty() => {
            let facts = parsed
                .iter()
                .map(statement_facts)
                .reduce(Facts::merge)
                .unwrap_or(Facts::UNKNOWN);

            // Un `EXPLAIN` d'une mutation contient forcément le mot-clé mutant,
            // et sa forme a déjà été tranchée par l'AST — avec ou sans
            // `ANALYZE`. Laisser le filet agir ferait d'`EXPLAIN DELETE …`,
            // qui ne fait que calculer un plan, une instruction à approuver.
            let sweepable = parsed.iter().all(|statement| {
                !matches!(
                    statement,
                    Statement::Explain { .. } | Statement::ExplainTable { .. }
                )
            });

            if facts == Facts::READ && sweepable && hides_a_mutation(fragment.text, dialect) {
                tracing::debug!(
                    span = ?fragment.span,
                    "mutating keyword outside the AST: statement downgraded to Unknown"
                );
                (Facts::UNKNOWN, Basis::KeywordSweep, None)
            } else {
                (facts, Basis::Ast, None)
            }
        }
        // `split` ne rend que des fragments porteurs de code : n'obtenir aucune
        // instruction est déjà une anomalie.
        Ok(_) => (
            Facts::UNKNOWN,
            Basis::Unparsed,
            Some("no statement was read".to_owned()),
        ),
        Err(err) => {
            tracing::debug!(
                span = ?fragment.span,
                error = %err,
                "statement could not be parsed: classified as Unknown"
            );
            (Facts::UNKNOWN, Basis::Unparsed, Some(err.to_string()))
        }
    };

    StatementInfo {
        text: fragment.text.to_owned(),
        span: fragment.span.clone(),
        intent: facts.intent,
        risk: facts.risk,
        basis,
        error,
        transaction_control: controls_a_transaction(fragment.text, dialect),
    }
}

/// Ce qu'une instruction fait, d'après sa forme.
///
/// Toute forme non listée retombe sur [`Facts::UNKNOWN`]. C'est délibéré :
/// `sqlparser` connaît plus de cent formes d'instructions, la liste bougera à
/// chaque montée de version, et une forme nouvelle doit demander une
/// approbation, pas passer pour une lecture.
fn statement_facts(statement: &Statement) -> Facts {
    match statement {
        // ── Lecture ─────────────────────────────────────────────────────────
        Statement::Query(query) => query_facts(query),

        Statement::Explain {
            analyze,
            options,
            statement: inner,
            ..
        } => {
            if *analyze || analyze_requested(options.as_deref()) {
                // `EXPLAIN ANALYZE` exécute réellement ce qu'il analyse (I-07) :
                // l'instruction hérite de tout, intention comme risque.
                statement_facts(inner)
            } else {
                Facts::READ
            }
        }

        Statement::ExplainTable { .. }
        | Statement::ShowFunctions { .. }
        | Statement::ShowVariable { .. }
        | Statement::ShowStatus { .. }
        | Statement::ShowVariables { .. }
        | Statement::ShowCreate { .. }
        | Statement::ShowColumns { .. }
        | Statement::ShowCatalogs { .. }
        | Statement::ShowDatabases { .. }
        | Statement::ShowProcessList { .. }
        | Statement::ShowSchemas { .. }
        | Statement::ShowCharset(_)
        | Statement::ShowObjects(_)
        | Statement::ShowTables { .. }
        | Statement::ShowViews { .. }
        | Statement::ShowCollation { .. } => Facts::READ,

        // Contrôle de transaction et changement de contexte : ces instructions
        // ne lisent ni n'écrivent d'elles-mêmes. Les compter comme mutantes
        // ferait demander une approbation pour `BEGIN; SELECT 1; COMMIT;` ; le
        // lot prend de toute façon l'intention de ce qu'il contient. What the
        // transaction verbs do to the session's open transaction is carried
        // apart, by `controls_a_transaction`.
        Statement::StartTransaction { .. }
        | Statement::Commit { .. }
        | Statement::Rollback { .. }
        | Statement::Savepoint { .. }
        | Statement::ReleaseSavepoint { .. }
        | Statement::Use(_) => Facts::READ,

        // ── Écriture ────────────────────────────────────────────────────────
        Statement::Insert(insert) => match &insert.source {
            // `INSERT INTO t WITH d AS (DELETE …) SELECT …` : la source peut
            // porter sa propre mutation.
            Some(source) => Facts::WRITE.merge(query_facts(source)),
            None => Facts::WRITE,
        },

        Statement::Update(update) => Facts::new(
            StatementIntent::Write,
            if where_is_unbounded(update.selection.as_ref()) {
                MutationRisk::UnboundedUpdate
            } else {
                MutationRisk::None
            },
        ),

        Statement::Delete(delete) => Facts::new(
            StatementIntent::Write,
            if where_is_unbounded(delete.selection.as_ref()) {
                MutationRisk::UnboundedDelete
            } else {
                MutationRisk::None
            },
        ),

        // `MERGE` est borné par sa condition `ON` : pas de risque de portée.
        Statement::Merge(_) => Facts::WRITE,

        // `COPY t FROM …` charge dans la table ; `COPY t TO …` exporte.
        Statement::Copy { to, .. } => {
            if *to {
                Facts::READ
            } else {
                Facts::WRITE
            }
        }
        Statement::CopyIntoSnowflake { kind, .. } => match kind {
            CopyIntoSnowflakeKind::Table => Facts::WRITE,
            CopyIntoSnowflakeKind::Location => Facts::READ,
        },

        // ── Structure ───────────────────────────────────────────────────────
        Statement::Truncate(_) => Facts::new(StatementIntent::Ddl, MutationRisk::Truncate),

        Statement::Drop { object_type, .. } => match object_type {
            // `DROP ROLE` / `DROP USER` touchent aux droits, pas au schéma.
            ObjectType::Role | ObjectType::User => Facts::GRANT,
            _ => Facts::new(StatementIntent::Ddl, MutationRisk::DropObject),
        },

        Statement::DropFunction(_)
        | Statement::DropDomain(_)
        | Statement::DropProcedure { .. }
        | Statement::DropSecret { .. }
        | Statement::DropPolicy(_)
        | Statement::DropConnector { .. }
        | Statement::DropExtension(_)
        | Statement::DropOperator(_)
        | Statement::DropOperatorFamily(_)
        | Statement::DropOperatorClass(_)
        | Statement::DropTrigger(_) => Facts::new(StatementIntent::Ddl, MutationRisk::DropObject),

        Statement::CreateView(_)
        | Statement::CreateTable(_)
        | Statement::CreateVirtualTable { .. }
        | Statement::CreateIndex(_)
        | Statement::CreateSecret { .. }
        | Statement::CreateServer(_)
        | Statement::CreatePolicy(_)
        | Statement::CreateConnector(_)
        | Statement::CreateOperator(_)
        | Statement::CreateOperatorFamily(_)
        | Statement::CreateOperatorClass(_)
        | Statement::CreateExtension(_)
        | Statement::CreateCollation(_)
        | Statement::CreateSchema { .. }
        | Statement::CreateDatabase { .. }
        | Statement::CreateFunction(_)
        | Statement::CreateTrigger(_)
        | Statement::CreateProcedure { .. }
        | Statement::CreateMacro { .. }
        | Statement::CreateStage { .. }
        | Statement::CreateSequence { .. }
        | Statement::CreateDomain(_)
        | Statement::CreateType { .. }
        | Statement::AlterTable(_)
        | Statement::AlterSchema(_)
        | Statement::AlterIndex { .. }
        | Statement::AlterView { .. }
        | Statement::AlterFunction(_)
        | Statement::AlterType(_)
        | Statement::AlterCollation(_)
        | Statement::AlterOperator(_)
        | Statement::AlterOperatorFamily(_)
        | Statement::AlterOperatorClass(_)
        | Statement::AlterPolicy(_)
        | Statement::AlterConnector { .. }
        | Statement::RenameTable(_)
        | Statement::Comment { .. }
        | Statement::AttachDatabase { .. }
        | Statement::AttachDuckDBDatabase { .. }
        | Statement::DetachDuckDBDatabase { .. } => Facts::DDL,

        // ── Droits ──────────────────────────────────────────────────────────
        Statement::Grant(_)
        | Statement::Revoke(_)
        | Statement::Deny(_)
        | Statement::CreateRole(_)
        | Statement::AlterRole { .. }
        | Statement::CreateUser(_)
        | Statement::AlterUser(_) => Facts::GRANT,

        // `SET ROLE` et `SET SESSION AUTHORIZATION` changent l'identité sous
        // laquelle tout le reste s'exécute.
        Statement::Set(Set::SetRole { .. } | Set::SetSessionAuthorization(_)) => Facts::GRANT,
        // Les autres `SET` ne sont pas anodins non plus : `SET TRANSACTION READ
        // WRITE` défait une session en lecture seule. Faute de pouvoir les
        // distinguer un par un, ils restent `Unknown`.
        Statement::Set(_) => Facts::UNKNOWN,

        // ── Le reste ────────────────────────────────────────────────────────
        // `ANALYZE`, `VACUUM`, `PRAGMA`, curseurs, `EXECUTE` d'une instruction
        // préparée, appels de procédure : autant de formes qui peuvent écrire
        // sans le dire.
        // TODO(phase 1) : affiner au cas par cas quand les drivers réels
        // montreront lesquelles comptent, avec un test par forme ajoutée.
        _ => Facts::UNKNOWN,
    }
}

/// Ce qu'une requête fait, clauses `WITH` comprises.
///
/// PostgreSQL autorise `WITH x AS (DELETE FROM t RETURNING *) SELECT * FROM x`.
/// L'instruction commence par `WITH`, `sqlparser` la rend comme une `Query`, et
/// elle supprime des lignes.
fn query_facts(query: &Query) -> Facts {
    let mut facts = Facts::READ;
    if let Some(with) = &query.with {
        for cte in &with.cte_tables {
            facts = facts.merge(query_facts(&cte.query));
        }
    }
    facts.merge(set_expr_facts(&query.body))
}

fn set_expr_facts(body: &SetExpr) -> Facts {
    match body {
        // Une sous-requête d'un `SELECT` ne peut pas muter : seule la clause
        // `WITH` de plus haut niveau le peut, et elle est traitée ailleurs.
        // Sauf `SELECT … INTO t` : PostgreSQL et SQL Server y créent la table
        // `t` et la remplissent. C'est un `CREATE TABLE AS` qui commence par
        // `SELECT` ; lu comme une lecture, il passait sans la confirmation qui
        // nomme la connexion (I-02).
        SetExpr::Select(select) if select.into.is_some() => Facts::DDL,
        SetExpr::Select(_) | SetExpr::Values(_) | SetExpr::Table(_) => Facts::READ,
        SetExpr::Query(inner) => query_facts(inner),
        SetExpr::SetOperation { left, right, .. } => {
            set_expr_facts(left).merge(set_expr_facts(right))
        }
        SetExpr::Insert(statement)
        | SetExpr::Update(statement)
        | SetExpr::Delete(statement)
        | SetExpr::Merge(statement) => statement_facts(statement),
    }
}

/// `EXPLAIN (ANALYZE, VERBOSE) …` de PostgreSQL passe par les options, pas par
/// le drapeau `analyze` de l'AST.
///
/// L'argument de l'option n'est pas regardé : `EXPLAIN (ANALYZE false) DELETE`
/// est traité comme s'il analysait. Sur-classer coûte une confirmation ;
/// sous-classer exécute le `DELETE`.
fn analyze_requested(options: Option<&[UtilityOption]>) -> bool {
    options.is_some_and(|options| {
        options
            .iter()
            .any(|option| option.name.value.eq_ignore_ascii_case("analyze"))
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Portée d'une mutation
// ─────────────────────────────────────────────────────────────────────────────

/// La clause `WHERE` laisse-t-elle passer toutes les lignes ?
///
/// Absence de `WHERE` et `WHERE` trivialement vrai comptent pareil : `DELETE
/// FROM t WHERE 1=1` supprime exactement ce que `DELETE FROM t` supprime, et
/// c'est la forme qu'on écrit quand on assemble un `WHERE` par morceaux.
fn where_is_unbounded(selection: Option<&Expr>) -> bool {
    match selection {
        None => true,
        Some(expr) => is_trivially_true(expr),
    }
}

/// L'expression vaut-elle vrai sans regarder une seule ligne ?
///
/// Volontairement incomplet : ce n'est pas un évaluateur. Toute expression que
/// cette fonction ne sait pas trancher est déclarée non triviale, ce qui donne
/// [`MutationRisk::None`] — l'écriture reste une écriture, et le `PolicyGate`
/// la traite comme telle ; seul le motif « toutes les lignes » manque.
fn is_trivially_true(expr: &Expr) -> bool {
    match expr {
        Expr::Nested(inner) => is_trivially_true(inner),
        Expr::Value(value) => literal_truth(&value.value) == Some(true),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => is_trivially_false(inner),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Or => is_trivially_true(left) || is_trivially_true(right),
            BinaryOperator::And => is_trivially_true(left) && is_trivially_true(right),
            // `1=1`, `'x'='x'`, et aussi `id = id` : sur une colonne non nulle,
            // ce dernier ne borne rien du tout.
            BinaryOperator::Eq => {
                same_pure_expr(left, right)
                    || compare_numbers(left, right).is_some_and(Ordering::is_eq)
            }
            BinaryOperator::NotEq => compare_numbers(left, right).is_some_and(Ordering::is_ne),
            BinaryOperator::Gt => compare_numbers(left, right).is_some_and(Ordering::is_gt),
            BinaryOperator::Lt => compare_numbers(left, right).is_some_and(Ordering::is_lt),
            BinaryOperator::GtEq => compare_numbers(left, right).is_some_and(Ordering::is_ge),
            BinaryOperator::LtEq => compare_numbers(left, right).is_some_and(Ordering::is_le),
            _ => false,
        },
        _ => false,
    }
}

fn is_trivially_false(expr: &Expr) -> bool {
    match expr {
        Expr::Nested(inner) => is_trivially_false(inner),
        Expr::Value(value) => literal_truth(&value.value) == Some(false),
        Expr::UnaryOp {
            op: UnaryOperator::Not,
            expr: inner,
        } => is_trivially_true(inner),
        Expr::BinaryOp { left, op, right } => match op {
            BinaryOperator::Or => is_trivially_false(left) && is_trivially_false(right),
            BinaryOperator::And => is_trivially_false(left) || is_trivially_false(right),
            BinaryOperator::Eq => compare_numbers(left, right).is_some_and(Ordering::is_ne),
            BinaryOperator::NotEq => {
                same_pure_expr(left, right)
                    || compare_numbers(left, right).is_some_and(Ordering::is_eq)
            }
            _ => false,
        },
        _ => false,
    }
}

/// Vérité d'un littéral. `NULL` n'est ni vrai ni faux : `WHERE NULL` ne
/// sélectionne rien, mais ce n'est pas à cette fonction de le dire.
fn literal_truth(value: &Value) -> Option<bool> {
    match value {
        Value::Boolean(state) => Some(*state),
        // `WHERE 1` de MySQL. `0.0` compte pour faux.
        Value::Number(text, _) => text.parse::<f64>().ok().map(|number| number != 0.0),
        _ => None,
    }
}

/// Les deux côtés sont-ils la même expression sans effet de bord ?
///
/// La comparaison se fait sur le rendu, pour ignorer les positions de source
/// que porte l'AST. Restreinte aux identifiants et aux littéraux : un appel de
/// fonction identique des deux côtés (`random() = random()`) n'est pas vrai.
fn same_pure_expr(left: &Expr, right: &Expr) -> bool {
    is_pure(left) && is_pure(right) && left.to_string() == right.to_string()
}

fn is_pure(expr: &Expr) -> bool {
    match expr {
        Expr::Identifier(_) | Expr::CompoundIdentifier(_) | Expr::Value(_) => true,
        Expr::Nested(inner) => is_pure(inner),
        _ => false,
    }
}

fn compare_numbers(left: &Expr, right: &Expr) -> Option<Ordering> {
    let left = numeric_literal(left)?;
    let right = numeric_literal(right)?;
    left.partial_cmp(&right)
}

fn numeric_literal(expr: &Expr) -> Option<f64> {
    match expr {
        Expr::Nested(inner) => numeric_literal(inner),
        Expr::UnaryOp {
            op: UnaryOperator::Minus,
            expr: inner,
        } => numeric_literal(inner).map(|number| -number),
        Expr::UnaryOp {
            op: UnaryOperator::Plus,
            expr: inner,
        } => numeric_literal(inner),
        Expr::Value(value) => match &value.value {
            Value::Number(text, _) => text.parse::<f64>().ok(),
            _ => None,
        },
        _ => None,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Le filet sous l'AST
// ─────────────────────────────────────────────────────────────────────────────

/// Mots dont la présence nue trahit une mutation que l'AST n'a pas rapportée.
///
/// `CREATE` et `ALTER` en sont absents à dessein : `SHOW CREATE TABLE t` est
/// une lecture, et une forme `CREATE` imbriquée dans une requête n'existe pas.
const MUTATING_WORDS: [&str; 8] = [
    "delete", "update", "insert", "merge", "truncate", "drop", "grant", "revoke",
];

/// Une instruction que l'AST a classée `Read` cache-t-elle une mutation ?
///
/// N'est appelée **que** sur les instructions classées `Read` sans risque, et
/// ne peut que les déclasser en `Unknown`. C'est une défense en profondeur
/// contre une forme d'imbrication non anticipée dans [`statement_facts`] : le
/// coût d'un faux positif est une confirmation, celui d'un faux négatif est un
/// `DELETE` silencieux.
fn hides_a_mutation(text: &str, dialect: SqlDialect) -> bool {
    let words = split::words(text, dialect);
    for (index, word) in words.iter().enumerate() {
        // `TRUNCATE(x, 2)` et `INSERT('abc', 1, 1, 'z')` sont des fonctions.
        if word.call {
            continue;
        }
        if !MUTATING_WORDS
            .iter()
            .any(|keyword| word.text.eq_ignore_ascii_case(keyword))
        {
            continue;
        }
        // `SELECT … FOR UPDATE` verrouille des lignes, il ne les modifie pas.
        if word.text.eq_ignore_ascii_case("update") && locked_for_update(&words, index) {
            continue;
        }
        return true;
    }
    false
}

/// Does the statement begin, end or mark a transaction?
///
/// Read from its leading words rather than from the AST: every such statement
/// opens with its verb, and the verbs the parser does not know — PostgreSQL's
/// `END` and `ABORT`, `PREPARE TRANSACTION`, `XA` — count as much as those it
/// does. Over-reading only costs an agent a refusal; a `BEGIN` opening a
/// procedural block is read as a transaction.
fn controls_a_transaction(text: &str, dialect: SqlDialect) -> bool {
    let words = split::words(text, dialect);
    let mut leading = words.iter().map(|word| word.text);
    let Some(verb) = leading.next() else {
        return false;
    };
    let is = |keyword: &str| verb.eq_ignore_ascii_case(keyword);
    if TRANSACTION_VERBS.iter().any(|keyword| is(keyword)) {
        return true;
    }
    // `START TRANSACTION`, `PREPARE TRANSACTION 'x'`, `SAVE TRAN` — but not
    // `PREPARE plan AS …`, a prepared statement.
    (is("start") || is("prepare") || is("save"))
        && leading.next().is_some_and(|second| {
            ["transaction", "tran"]
                .iter()
                .any(|keyword| second.eq_ignore_ascii_case(keyword))
        })
}

/// Verbs that, leading a statement, always control a transaction.
const TRANSACTION_VERBS: [&str; 8] = [
    "begin",
    "commit",
    "end",
    "rollback",
    "abort",
    "savepoint",
    "release",
    "xa",
];

/// PostgreSQL écrit `FOR UPDATE` mais aussi `FOR NO KEY UPDATE`, d'où une
/// fenêtre de trois mots en arrière.
fn locked_for_update(words: &[Word<'_>], index: usize) -> bool {
    let from = index.saturating_sub(3);
    words.get(from..index).is_some_and(|window| {
        window
            .iter()
            .any(|word| word.text.eq_ignore_ascii_case("for"))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxyn_core::ExecLimits;
    use rstest::rstest;

    fn pg(sql: &str) -> Classification {
        classify(sql, SqlDialect::Postgres)
    }

    // ── La table de cas des règles de base ───────────────────────────────────

    #[rstest]
    #[case("SELECT 1", StatementIntent::Read)]
    #[case("SELECT * FROM clients WHERE id = 1", StatementIntent::Read)]
    #[case("WITH r AS (SELECT 1) SELECT * FROM r", StatementIntent::Read)]
    #[case("SELECT 1 UNION SELECT 2", StatementIntent::Read)]
    #[case("VALUES (1), (2)", StatementIntent::Read)]
    #[case("SHOW TABLES", StatementIntent::Read)]
    #[case("EXPLAIN SELECT 1", StatementIntent::Read)]
    #[case("EXPLAIN ANALYZE SELECT 1", StatementIntent::Read)]
    #[case("COPY clients TO '/tmp/x.csv'", StatementIntent::Read)]
    #[case("INSERT INTO t (a) VALUES (1)", StatementIntent::Write)]
    #[case("UPDATE t SET a = 1 WHERE id = 2", StatementIntent::Write)]
    #[case("DELETE FROM t WHERE id = 2", StatementIntent::Write)]
    #[case(
        "MERGE INTO t USING s ON t.id = s.id WHEN MATCHED THEN UPDATE SET t.a = s.a",
        StatementIntent::Write
    )]
    #[case("COPY t FROM '/tmp/x.csv'", StatementIntent::Write)]
    #[case("SELECT * INTO copie FROM clients", StatementIntent::Ddl)]
    #[case("SELECT a INTO TEMP copie FROM clients", StatementIntent::Ddl)]
    #[case(
        "WITH r AS (SELECT 1) SELECT * INTO copie FROM r",
        StatementIntent::Ddl
    )]
    #[case("CREATE TABLE t (a int)", StatementIntent::Ddl)]
    #[case("ALTER TABLE t ADD COLUMN b int", StatementIntent::Ddl)]
    #[case("DROP TABLE t", StatementIntent::Ddl)]
    #[case("TRUNCATE TABLE t", StatementIntent::Ddl)]
    #[case("CREATE INDEX i ON t (a)", StatementIntent::Ddl)]
    #[case("GRANT SELECT ON t TO lecteur", StatementIntent::Grant)]
    #[case("REVOKE SELECT ON t FROM lecteur", StatementIntent::Grant)]
    #[case("CREATE ROLE analyste", StatementIntent::Grant)]
    #[case("ALTER ROLE analyste WITH LOGIN", StatementIntent::Grant)]
    #[case("SET ROLE analyste", StatementIntent::Grant)]
    #[case("DROP ROLE analyste", StatementIntent::Grant)]
    #[case("VACUUM FULL", StatementIntent::Unknown)]
    #[case("ceci n'est pas du SQL", StatementIntent::Unknown)]
    fn la_table_des_regles(#[case] sql: &str, #[case] attendu: StatementIntent) {
        let lu = pg(sql);
        assert_eq!(lu.intent, attendu, "{sql} → {lu:?}");
    }

    #[test]
    fn describe_et_show_sont_des_lectures() {
        assert_eq!(
            classify("DESCRIBE clients", SqlDialect::MySql).intent,
            StatementIntent::Read
        );
        assert_eq!(
            classify("SHOW CREATE TABLE clients", SqlDialect::MySql).intent,
            StatementIntent::Read
        );
    }

    #[test]
    fn rename_est_du_ddl() {
        assert_eq!(
            classify("RENAME TABLE a TO b", SqlDialect::MySql).intent,
            StatementIntent::Ddl
        );
    }

    // ── Les pièges ───────────────────────────────────────────────────────────

    /// I-07, mot pour mot : `EXPLAIN ANALYZE` exécute la requête qu'il analyse.
    #[test]
    fn explain_analyze_d_un_delete_n_est_pas_une_lecture() {
        let lu = pg("EXPLAIN ANALYZE DELETE FROM commandes");
        assert_eq!(lu.intent, StatementIntent::Write, "{lu:?}");
        assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
        assert!(lu.is_mutating());
        assert!(!lu.is_read_only());
    }

    /// La forme PostgreSQL passe par les options et non par le mot-clé nu.
    #[test]
    fn explain_avec_options_analyze_n_est_pas_une_lecture() {
        for sql in [
            "EXPLAIN (ANALYZE) DELETE FROM commandes",
            "EXPLAIN (ANALYZE, VERBOSE) DELETE FROM commandes",
            "EXPLAIN (VERBOSE, ANALYZE true) UPDATE t SET a = 1",
        ] {
            let lu = pg(sql);
            assert_eq!(lu.intent, StatementIntent::Write, "{sql} → {lu:?}");
        }
    }

    #[test]
    fn explain_sans_analyze_reste_une_lecture() {
        assert_eq!(
            pg("EXPLAIN DELETE FROM commandes").intent,
            StatementIntent::Read
        );
        assert_eq!(
            pg("EXPLAIN (VERBOSE) DELETE FROM commandes").intent,
            StatementIntent::Read
        );
    }

    /// L'instruction commence par `WITH` et supprime toutes les lignes.
    #[test]
    fn un_with_qui_supprime_n_est_pas_une_lecture() {
        let lu =
            pg("WITH partis AS (DELETE FROM commandes RETURNING *) SELECT count(*) FROM partis");
        assert_eq!(lu.intent, StatementIntent::Write, "{lu:?}");
        assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
    }

    #[test]
    fn un_with_suivi_d_un_delete_n_est_pas_une_lecture() {
        let lu = pg(
            "WITH cibles AS (SELECT id FROM t) DELETE FROM u WHERE id IN (SELECT id FROM cibles)",
        );
        assert_eq!(lu.intent, StatementIntent::Write, "{lu:?}");
        assert_eq!(lu.risk, MutationRisk::None, "le DELETE est borné");
    }

    #[test]
    fn un_with_qui_met_a_jour_est_signale() {
        let lu = pg("WITH m AS (UPDATE t SET a = 1 RETURNING *) SELECT * FROM m");
        assert_eq!(lu.intent, StatementIntent::Write);
        assert_eq!(lu.risk, MutationRisk::UnboundedUpdate);
    }

    #[test]
    fn un_sql_invalide_est_inconnu_donc_mutant() {
        let lu = pg("SELEKT * FORM t");
        assert_eq!(lu.intent, StatementIntent::Unknown);
        assert!(lu.is_mutating(), "Unknown doit compter pour mutant");
        assert_eq!(lu.statements.len(), 1);
        assert_eq!(lu.statements[0].basis, Basis::Unparsed);
        assert!(lu.statements[0].error.is_some());
        assert!(!lu.is_fully_understood());
    }

    /// Le point-virgule d'un commentaire ne doit pas fabriquer une seconde
    /// instruction — et surtout pas une qui se lirait comme une lecture.
    #[test]
    fn un_commentaire_contenant_un_point_virgule_ne_coupe_pas() {
        let lu = pg("DELETE FROM t -- garder ; ceci\n");
        assert_eq!(lu.statements.len(), 1, "{lu:?}");
        assert_eq!(lu.intent, StatementIntent::Write);
        assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
    }

    #[test]
    fn un_point_virgule_dans_une_chaine_ne_coupe_pas() {
        let lu = pg("SELECT ';' FROM t");
        assert_eq!(lu.statements.len(), 1);
        assert_eq!(lu.intent, StatementIntent::Read);
    }

    // ── Détection de portée ──────────────────────────────────────────────────

    #[rstest]
    #[case("DELETE FROM t", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 1=1", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 1 = 1", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE true", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE TRUE", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE (1=1)", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 'x' = 'x'", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE id = id", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE id = 1 OR 1=1", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 1=1 AND true", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE NOT false", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 1 <> 2", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE 2 > 1", MutationRisk::UnboundedDelete)]
    #[case("DELETE FROM t WHERE id = 1", MutationRisk::None)]
    #[case("DELETE FROM t WHERE 1=1 AND id = 2", MutationRisk::None)]
    #[case("DELETE FROM t WHERE actif", MutationRisk::None)]
    #[case("DELETE FROM t WHERE 1 = 2", MutationRisk::None)]
    fn la_portee_d_un_delete(#[case] sql: &str, #[case] attendu: MutationRisk) {
        assert_eq!(pg(sql).risk, attendu, "{sql}");
    }

    #[rstest]
    #[case("UPDATE t SET a = 1", MutationRisk::UnboundedUpdate)]
    #[case("UPDATE t SET a = 1 WHERE 1=1", MutationRisk::UnboundedUpdate)]
    #[case("UPDATE t SET a = 1 WHERE true", MutationRisk::UnboundedUpdate)]
    #[case("UPDATE t SET a = 1 WHERE id = 2", MutationRisk::None)]
    fn la_portee_d_un_update(#[case] sql: &str, #[case] attendu: MutationRisk) {
        assert_eq!(pg(sql).risk, attendu, "{sql}");
    }

    #[test]
    fn truncate_et_drop_portent_leur_risque() {
        assert_eq!(pg("TRUNCATE TABLE t").risk, MutationRisk::Truncate);
        assert_eq!(pg("DROP TABLE t").risk, MutationRisk::DropObject);
        assert_eq!(pg("DROP VIEW v").risk, MutationRisk::DropObject);
        assert_eq!(pg("DROP SCHEMA s CASCADE").risk, MutationRisk::DropObject);
    }

    #[test]
    fn un_appel_de_fonction_homonyme_ne_declenche_rien() {
        // `TRUNCATE(x, 2)` est une fonction numérique, pas un vidage de table.
        let lu = classify("SELECT TRUNCATE(1.234, 2)", SqlDialect::MySql);
        assert_eq!(lu.intent, StatementIntent::Read, "{lu:?}");
        assert_eq!(lu.risk, MutationRisk::None);
    }

    /// `SELECT … FOR UPDATE` verrouille des lignes sans les modifier : le
    /// filet par mots-clés ne doit pas le déclasser.
    #[test]
    fn un_verrou_de_ligne_reste_une_lecture() {
        let lu = pg("SELECT * FROM t WHERE id = 1 FOR UPDATE");
        assert_eq!(lu.intent, StatementIntent::Read, "{lu:?}");
        assert_eq!(lu.statements[0].basis, Basis::Ast);
    }

    /// Le filet ne s'applique pas à un `EXPLAIN`, dont la forme est déjà
    /// tranchée : sans lui, `EXPLAIN DELETE …` demanderait une approbation
    /// pour un calcul de plan.
    #[test]
    fn le_filet_epargne_un_explain() {
        let lu = pg("EXPLAIN DELETE FROM commandes");
        assert_eq!(lu.intent, StatementIntent::Read, "{lu:?}");
        assert_eq!(lu.statements[0].basis, Basis::Ast);
    }

    #[test]
    fn un_mot_clef_mutant_dans_une_chaine_ne_declenche_rien() {
        let lu = pg("SELECT * FROM audit WHERE action = 'DELETE FROM clients'");
        assert_eq!(lu.intent, StatementIntent::Read, "{lu:?}");
    }

    // ── Agrégation d'un lot ──────────────────────────────────────────────────

    #[test]
    fn un_lot_prend_l_intention_la_plus_elevee() {
        let lu = pg("SELECT 1; UPDATE t SET a = 1 WHERE id = 2; SELECT 2");
        assert_eq!(lu.intent, StatementIntent::Write);
        assert_eq!(lu.statements.len(), 3);
        assert_eq!(lu.statements[0].intent, StatementIntent::Read);
        assert_eq!(lu.statements[1].intent, StatementIntent::Write);
    }

    #[test]
    fn un_lot_prend_le_risque_le_plus_grave() {
        let lu = pg("UPDATE t SET a = 1; DROP TABLE u");
        assert_eq!(lu.intent, StatementIntent::Ddl);
        assert_eq!(lu.risk, MutationRisk::DropObject);
        assert_eq!(lu.risky().count(), 2);
    }

    #[test]
    fn une_instruction_illisible_contamine_le_lot() {
        let lu = pg("SELECT 1; ceci n'est pas du SQL");
        assert_eq!(lu.intent, StatementIntent::Unknown);
        assert!(lu.is_mutating());
    }

    #[test]
    fn unknown_l_emporte_sur_ddl_a_rang_egal() {
        let lu = pg("DROP TABLE t; ceci n'est pas du SQL");
        assert_eq!(lu.intent, StatementIntent::Unknown, "{lu:?}");
        // Le risque du `DROP` reste visible malgré tout.
        assert_eq!(lu.risk, MutationRisk::DropObject);
    }

    #[test]
    fn grant_l_emporte_sur_tout() {
        let lu = pg("SELECT 1; DROP TABLE t; GRANT SELECT ON u TO r");
        assert_eq!(lu.intent, StatementIntent::Grant);
    }

    #[test]
    fn une_transaction_ne_gonfle_pas_une_lecture() {
        let lu = pg("BEGIN; SELECT 1; COMMIT");
        assert_eq!(lu.intent, StatementIntent::Read, "{lu:?}");
        assert!(lu.is_read_only());
    }

    #[test]
    fn une_transaction_ne_masque_pas_une_ecriture() {
        let lu = pg("BEGIN; DELETE FROM t; COMMIT");
        assert_eq!(lu.intent, StatementIntent::Write);
        assert_eq!(lu.risk, MutationRisk::UnboundedDelete);
    }

    /// Read, yet settling what the session holds: the gate refuses these to
    /// an agent (issue #19). Parsed or not — `END` and `ABORT` are
    /// PostgreSQL's own spellings of `COMMIT` and `ROLLBACK`.
    #[rstest]
    #[case::begin("BEGIN")]
    #[case::start("START TRANSACTION READ WRITE")]
    #[case::commit("COMMIT")]
    #[case::commit_commente("/* fin */ commit")]
    #[case::end("END")]
    #[case::rollback("ROLLBACK")]
    #[case::abort("ABORT")]
    #[case::savepoint("SAVEPOINT s1")]
    #[case::rollback_to("ROLLBACK TO SAVEPOINT s1")]
    #[case::release("RELEASE SAVEPOINT s1")]
    #[case::prepare_transaction("PREPARE TRANSACTION 'x'")]
    #[case::commit_prepared("COMMIT PREPARED 'x'")]
    #[case::in_a_batch("SELECT 1; ROLLBACK")]
    fn le_controle_de_transaction_est_signale(#[case] sql: &str) {
        let lu = pg(sql);
        assert!(lu.transaction_control, "{lu:?}");
        let request = lu.qualify(ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            sql,
        ));
        assert!(request.transaction_control, "qualify carries it");
    }

    #[rstest]
    #[case::lecture("SELECT 1")]
    #[case::ecriture("DELETE FROM t WHERE id = 1")]
    #[case::instruction_preparee("PREPARE plan AS SELECT 1")]
    #[case::nom_de_colonne("SELECT commit, rollback FROM journal")]
    #[case::chaine("SELECT 'COMMIT'")]
    fn le_reste_ne_l_est_pas(#[case] sql: &str) {
        assert!(!pg(sql).transaction_control, "{sql}");
    }

    #[test]
    fn qualify_remplace_ce_que_l_appelant_declare() {
        let mut declare = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "COMMIT");
        declare.transaction_control = false;
        assert!(pg("COMMIT").qualify(declare).transaction_control);

        let mut declare = ExecRequest::new(QueryLanguage::Sql(SqlDialect::Postgres), "SELECT 1");
        declare.transaction_control = true;
        assert!(!pg("SELECT 1").qualify(declare).transaction_control);
    }

    /// Un lot vide ne se lit pas comme une lecture : un découpage qui perdrait
    /// des instructions ne doit pas ouvrir la porte.
    #[test]
    fn un_lot_vide_est_inconnu() {
        for sql in ["", "   ", ";;", "-- juste une note"] {
            let lu = pg(sql);
            assert_eq!(lu.intent, StatementIntent::Unknown, "{sql:?} → {lu:?}");
            assert!(lu.is_empty());
            assert!(lu.is_mutating());
            assert!(!lu.is_fully_understood());
        }
    }

    // ── Langages non SQL ─────────────────────────────────────────────────────

    #[test]
    fn un_langage_non_sql_n_est_pas_devine() {
        let lu = classify_language(QueryLanguage::Cypher, "MATCH (n) RETURN n");
        assert_eq!(lu.intent, StatementIntent::Unknown);
        assert!(lu.is_mutating());
        assert_eq!(lu.statements.len(), 1);
        assert_eq!(lu.statements[0].basis, Basis::Unparsed);
    }

    #[test]
    fn un_sql_declare_passe_par_son_dialecte() {
        let lu = classify_language(QueryLanguage::Sql(SqlDialect::MySql), "SELECT 1");
        assert_eq!(lu.intent, StatementIntent::Read);
    }

    // ── Requalification ──────────────────────────────────────────────────────

    /// Le scénario d'ARCHITECTURE §8 : un agent déclare une lecture, le texte
    /// dit autre chose. C'est le texte qui gagne.
    #[test]
    fn une_intention_declaree_a_tort_est_corrigee() {
        let demande = ExecRequest::new(
            QueryLanguage::Sql(SqlDialect::Postgres),
            "DELETE FROM clients",
        )
        .with_intent(StatementIntent::Read)
        .with_limits(ExecLimits::default());

        let lu = reclassify(&demande);
        let requalifiee = lu.qualify(demande);

        assert_eq!(requalifiee.intent, StatementIntent::Write);
        assert_eq!(requalifiee.risk, MutationRisk::UnboundedDelete);
        assert!(requalifiee.is_mutating());
    }

    // ── Le filet sous l'AST ──────────────────────────────────────────────────

    #[test]
    fn le_filet_reconnait_un_mot_clef_mutant_nu() {
        assert!(hides_a_mutation(
            "SELECT * FROM (DELETE FROM t)",
            SqlDialect::Postgres
        ));
        assert!(hides_a_mutation("SELECT 1; GRANT", SqlDialect::Postgres));
    }

    #[test]
    fn le_filet_ne_se_declenche_pas_a_tort() {
        for (sql, dialecte) in [
            (
                "SELECT * FROM audit WHERE action = 'DELETE FROM t'",
                SqlDialect::Postgres,
            ),
            ("SELECT x /* DROP TABLE t */ FROM t", SqlDialect::Postgres),
            ("SELECT TRUNCATE(1.234, 2)", SqlDialect::MySql),
            ("SELECT * FROM t FOR UPDATE", SqlDialect::Postgres),
            ("SHOW CREATE TABLE t", SqlDialect::MySql),
            ("SELECT deleted_at, drop_date FROM t", SqlDialect::Postgres),
            (r#"SELECT "delete" FROM t"#, SqlDialect::Postgres),
        ] {
            assert!(!hides_a_mutation(sql, dialecte), "faux positif : {sql}");
        }
    }

    // ── Vérification stricte ─────────────────────────────────────────────────

    #[test]
    fn la_verification_stricte_situe_le_fautif() {
        assert!(validate("SELECT 1; SELECT 2", SqlDialect::Postgres).is_ok());

        let sql = "SELECT 1; SELEKT 2";
        let Err(QueryError::Syntax { span, .. }) = validate(sql, SqlDialect::Postgres) else {
            panic!("l'instruction fautive aurait dû être signalée");
        };
        assert_eq!(sql.get(span), Some("SELEKT 2"));
    }

    #[test]
    fn les_bornes_permettent_de_retrouver_l_instruction() {
        let sql = "SELECT 1;\n  DELETE FROM t";
        let lu = pg(sql);
        for statement in &lu.statements {
            assert_eq!(
                sql.get(statement.span.clone()),
                Some(statement.text.as_str())
            );
        }
    }
}
