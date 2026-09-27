//! Qualified path in the catalog hierarchy.
//!
//! The hierarchy of ARCHITECTURE §6 has five levels, **three** of which carry
//! a name: `Catalog`, `Namespace`, `Relation`. Each one can be missing, and
//! not only the last ones:
//!
//! | System | Catalog | Namespace | Relation | Rendering |
//! |---|---|---|---|---|
//! | PostgreSQL | `base` | `schema` | `table` | `base.schema.table` |
//! | MySQL | — | `base` | `table` | `base.table` |
//! | MongoDB | — | `base` | `collection` | `base.collection` |
//! | Elasticsearch | — | — | `index` | `index` |
//! | Neo4j | `base` | — | `label` | `base..label` |
//!
//! # The convention that makes the rendering reversible
//!
//! A path is read **from the right**: the last segment is always the
//! relation, the one before the namespace, the first the catalog. An empty
//! segment denotes a missing level. That is what lets
//! [`Display`](fmt::Display) and [`FromStr`] be exact inverses of each other —
//! including for a hole in the middle (`base..label`) and for a path that
//! stops before the relation (`base.schema.`).
//!
//! Without this convention, `base.label` would be read back as
//! `namespace.relation` and Neo4j's catalog level would silently vanish on
//! every round trip through disk.
//!
//! # Quoting
//!
//! A table name can contain a dot, a double quote, a semicolon.
//! `"users"; DROP TABLE audit; --` is a legal table name in PostgreSQL.
//! [`CatalogPath::qualify`] is therefore **the only** way to compose a
//! qualified identifier for a query ([I-10] and
//! [`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md)): it quotes each
//! segment according to the target dialect. Concatenating `format!("{path}")`
//! into SQL is the defect this invariant forbids.
//!
//! [I-10]: ../../../CLAUDE.md

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use oxyn_core::query::SqlDialect;
use serde::{Deserialize, Serialize};

/// Number of nameable levels in a path.
const PALIERS: usize = 3;

/// Failure to parse or build a catalog path.
///
/// The faulty text is **never** repeated in the message, for consistency with
/// [`IdParseError`](oxyn_core::IdParseError): an object name can contain
/// terminal escape sequences, and an error message ends up in a log
/// ([SECURITY, input surface §2](../../../docs/SECURITY.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("invalid catalog path: {detail}")]
pub struct CatalogPathError {
    detail: &'static str,
}

impl CatalogPathError {
    /// Builds a parse error.
    #[must_use]
    pub const fn new(detail: &'static str) -> Self {
        Self { detail }
    }

    /// Reason for the rejection, without repeating the faulty value.
    #[must_use]
    pub const fn detail(&self) -> &'static str {
        self.detail
    }
}

impl From<CatalogPathError> for oxyn_core::OxynError {
    fn from(err: CatalogPathError) -> Self {
        Self::Config(err.to_string())
    }
}

/// Checks that a level name is usable.
///
/// Two refusals, and only two:
///
/// * the empty string, because it denotes a missing level in the rendering as
///   in the cache keys;
/// * control characters, because an object name is hostile input
///   ([SECURITY, input surface §2](../../../docs/SECURITY.md)) and an ANSI
///   escape sequence in a table name repaints the terminal or the log line
///   that displays it.
///
/// Dots, double quotes, spaces and semicolons are **accepted**: they are legal
/// names, and [`CatalogPath::qualify`] knows how to quote them.
pub(crate) fn validate_segment(name: &str) -> Result<(), CatalogPathError> {
    if name.is_empty() {
        return Err(CatalogPathError::new("a named level cannot be empty"));
    }
    if name.chars().any(char::is_control) {
        return Err(CatalogPathError::new(
            "a level name contains a control character",
        ));
    }
    Ok(())
}

/// Quoting style of an identifier.
///
/// The style is not a cosmetic detail: it is what separates a table preview
/// from running `DROP TABLE audit` ([I-10]).
///
/// [I-10]: ../../../CLAUDE.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteStyle {
    /// ANSI double quotes: `"name"`, inner double quote doubled. Default, and
    /// safe fallback for any unknown dialect.
    #[default]
    Double,
    /// Backticks (MySQL, BigQuery): `` `name` ``, inner backtick doubled.
    Backtick,
    /// Square brackets (T-SQL): `[name]`, closing bracket doubled.
    Bracket,
    /// **No quoting.**
    ///
    /// Reserved for sources that do not compose query text from an object
    /// name — MongoDB, Redis, Elasticsearch, where the name is a document field
    /// or a URL segment, not a fragment of a language. Using it to compose SQL
    /// violates [I-10]; use [`CatalogPath::qualify_sql`], which never picks it.
    ///
    /// [I-10]: ../../../CLAUDE.md
    Bare,
}

impl QuoteStyle {
    /// The quoting style of a SQL dialect.
    ///
    /// An unrecognized dialect falls back on [`Double`](Self::Double): the
    /// fallback of a quoting model is another quoting model, never
    /// [`Bare`](Self::Bare).
    #[must_use]
    pub const fn for_dialect(dialect: SqlDialect) -> Self {
        match dialect {
            SqlDialect::MySql | SqlDialect::BigQuery => Self::Backtick,
            SqlDialect::SqlServer => Self::Bracket,
            // `SqlDialect` is `#[non_exhaustive]`: the wildcard arm is imposed
            // by the language, and it is better here than listing the ANSI
            // dialects, which would silently go stale.
            _ => Self::Double,
        }
    }
}

/// Quotes an identifier according to a style.
///
/// Validates nothing: an empty name or one carrying control characters comes
/// out quoted as is. Validation happens when a [`CatalogPath`] is built.
#[must_use]
pub fn quote_identifier(name: &str, style: QuoteStyle) -> String {
    let (ouvrant, fermant) = match style {
        QuoteStyle::Double => ('"', '"'),
        QuoteStyle::Backtick => ('`', '`'),
        QuoteStyle::Bracket => ('[', ']'),
        QuoteStyle::Bare => return name.to_owned(),
    };
    let mut sortie = String::with_capacity(name.len() + 2);
    sortie.push(ouvrant);
    for c in name.chars() {
        // In all three styles, only the closing character needs doubling — it
        // is the only one that can close the quoting by surprise.
        if c == fermant {
            sortie.push(c);
        }
        sortie.push(c);
    }
    sortie.push(fermant);
    sortie
}

/// The deepest level a path designates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogLevel {
    /// Empty path: the server itself.
    Server,
    /// The path stops at the catalog.
    Catalog,
    /// The path stops at the namespace.
    Namespace,
    /// The path designates a relation.
    Relation,
}

impl CatalogLevel {
    /// Stable name, for the audit and the interface.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Catalog => "catalog",
            Self::Namespace => "namespace",
            Self::Relation => "relation",
        }
    }
}

impl fmt::Display for CatalogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Qualified path, tolerant of missing levels.
///
/// The three levels are independent: a path can have a catalog and a relation
/// without a namespace (Neo4j), or only a relation (Elasticsearch).
///
/// Serialized as a **string** — the [`Display`](fmt::Display) rendering — and
/// not as a three-field object: a workspace file readable without Oxyn is a
/// product invariant (I-11), and the round trip is exact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct CatalogPath {
    catalog: Option<String>,
    namespace: Option<String>,
    relation: Option<String>,
}

impl CatalogPath {
    /// The empty path: the server itself.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            catalog: None,
            namespace: None,
            relation: None,
        }
    }

    /// Builds a path from its three levels.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if a present level is empty or contains a
    /// control character.
    pub fn from_levels(
        catalog: Option<String>,
        namespace: Option<String>,
        relation: Option<String>,
    ) -> Result<Self, CatalogPathError> {
        for nom in [&catalog, &namespace, &relation].into_iter().flatten() {
            validate_segment(nom)?;
        }
        Ok(Self {
            catalog,
            namespace,
            relation,
        })
    }

    /// Builds a path from **already validated** levels.
    ///
    /// Reserved to the crate: the model's references ([`RelationRef`] and its
    /// neighbors) validate their name at construction, so rebuilding their path
    /// cannot fail. A fallible function here would force every caller to handle
    /// an impossible error.
    ///
    /// [`RelationRef`]: crate::model::RelationRef
    pub(crate) fn from_validated(
        catalog: Option<String>,
        namespace: Option<String>,
        relation: Option<String>,
    ) -> Self {
        Self {
            catalog,
            namespace,
            relation,
        }
    }

    /// Path designating a catalog.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is empty or contains a control
    /// character.
    pub fn for_catalog(name: impl Into<String>) -> Result<Self, CatalogPathError> {
        Self::from_levels(Some(name.into()), None, None)
    }

    /// Path designating a namespace, under an optional catalog.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if a name is empty or contains a control
    /// character.
    pub fn for_namespace(
        catalog: Option<&str>,
        name: impl Into<String>,
    ) -> Result<Self, CatalogPathError> {
        Self::from_levels(catalog.map(str::to_owned), Some(name.into()), None)
    }

    /// Path designating a relation, under the optional levels that contain it.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if a name is empty or contains a control
    /// character.
    pub fn for_relation(
        catalog: Option<&str>,
        namespace: Option<&str>,
        name: impl Into<String>,
    ) -> Result<Self, CatalogPathError> {
        Self::from_levels(
            catalog.map(str::to_owned),
            namespace.map(str::to_owned),
            Some(name.into()),
        )
    }

    /// The catalog level, if present.
    #[must_use]
    pub fn catalog(&self) -> Option<&str> {
        self.catalog.as_deref()
    }

    /// The namespace level, if present.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// The relation level, if present.
    #[must_use]
    pub fn relation(&self) -> Option<&str> {
        self.relation.as_deref()
    }

    /// The name of the deepest present level.
    #[must_use]
    pub fn leaf(&self) -> Option<&str> {
        self.relation()
            .or_else(|| self.namespace())
            .or_else(|| self.catalog())
    }

    /// The deepest present level.
    #[must_use]
    pub fn level(&self) -> CatalogLevel {
        if self.relation.is_some() {
            CatalogLevel::Relation
        } else if self.namespace.is_some() {
            CatalogLevel::Namespace
        } else if self.catalog.is_some() {
            CatalogLevel::Catalog
        } else {
            CatalogLevel::Server
        }
    }

    /// Does the path designate no level?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.catalog.is_none() && self.namespace.is_none() && self.relation.is_none()
    }

    /// Number of present levels. A hole does not count.
    #[must_use]
    pub fn depth(&self) -> usize {
        usize::from(self.catalog.is_some())
            + usize::from(self.namespace.is_some())
            + usize::from(self.relation.is_some())
    }

    /// The present levels, from the most general to the most precise.
    pub fn segments(&self) -> impl Iterator<Item = &str> + '_ {
        [self.catalog(), self.namespace(), self.relation()]
            .into_iter()
            .flatten()
    }

    /// The path obtained by removing the deepest level.
    ///
    /// Returns `None` for the empty path.
    #[must_use]
    pub fn parent(&self) -> Option<Self> {
        let mut parent = self.clone();
        match self.level() {
            CatalogLevel::Server => return None,
            CatalogLevel::Catalog => parent.catalog = None,
            CatalogLevel::Namespace => parent.namespace = None,
            CatalogLevel::Relation => parent.relation = None,
        }
        Some(parent)
    }

    /// The same path, completed with a namespace level.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is invalid.
    pub fn with_namespace(&self, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            catalog: self.catalog.clone(),
            namespace: Some(name),
            relation: self.relation.clone(),
        })
    }

    /// The same path, completed with a relation level.
    ///
    /// # Errors
    /// Returns [`CatalogPathError`] if the name is invalid.
    pub fn with_relation(&self, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            catalog: self.catalog.clone(),
            namespace: self.namespace.clone(),
            relation: Some(name),
        })
    }

    /// The same path, completed with an **already validated** relation level.
    pub(crate) fn with_validated_relation(&self, name: &str) -> Self {
        Self {
            catalog: self.catalog.clone(),
            namespace: self.namespace.clone(),
            relation: Some(name.to_owned()),
        }
    }

    /// Returns the qualified identifier, each present level quoted according to
    /// `quote`.
    ///
    /// It is **the** way to compose an identifier in a query produced by Oxyn
    /// ([I-10]). Missing levels leave no empty dot: the result is query text,
    /// not a reversible rendering.
    ///
    /// ```
    /// use oxyn_catalog::path::{CatalogPath, QuoteStyle};
    ///
    /// let chemin = CatalogPath::for_relation(None, Some("public"), r#"users"; DROP TABLE audit; --"#)
    ///     .expect("the name is legal, only hostile");
    /// assert_eq!(
    ///     chemin.qualify(QuoteStyle::Double),
    ///     r#""public"."users""; DROP TABLE audit; --""#
    /// );
    /// ```
    ///
    /// [I-10]: ../../../CLAUDE.md
    #[must_use]
    pub fn qualify(&self, quote: QuoteStyle) -> String {
        let mut sortie = String::new();
        for (i, segment) in self.segments().enumerate() {
            if i > 0 {
                sortie.push('.');
            }
            sortie.push_str(&quote_identifier(segment, quote));
        }
        sortie
    }

    /// Returns the qualified identifier for a SQL dialect.
    ///
    /// Never picks [`QuoteStyle::Bare`]: an unknown dialect is quoted ANSI-style
    /// rather than left bare.
    #[must_use]
    pub fn qualify_sql(&self, dialect: SqlDialect) -> String {
        self.qualify(QuoteStyle::for_dialect(dialect))
    }

    /// Is this path located under `prefix`?
    ///
    /// A level missing in `prefix` imposes nothing; a present level must match
    /// exactly. The empty path contains everything.
    #[must_use]
    pub fn starts_with(&self, prefix: &Self) -> bool {
        let correspond = |exige: Option<&str>, reel: Option<&str>| match exige {
            None => true,
            Some(attendu) => reel == Some(attendu),
        };
        correspond(prefix.catalog(), self.catalog())
            && correspond(prefix.namespace(), self.namespace())
            && correspond(prefix.relation(), self.relation())
    }
}

/// Writes a segment, quoted if — and only if — the rendering would stay
/// ambiguous.
fn ecrire_segment(f: &mut fmt::Formatter<'_>, nom: &str) -> fmt::Result {
    if !nom.contains('.') && !nom.contains('"') {
        return f.write_str(nom);
    }
    f.write_char('"')?;
    for c in nom.chars() {
        if c == '"' {
            f.write_char('"')?;
        }
        f.write_char(c)?;
    }
    f.write_char('"')
}

impl fmt::Display for CatalogPath {
    /// Renders the path in a form [`FromStr`] can read back.
    ///
    /// Levels missing **between** the first present level and the relation
    /// leave an empty segment (`base..label`), and a path that stops before the
    /// relation keeps its trailing dots (`base.schema.`). That is what makes
    /// the round trip exact; see the module documentation.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let paliers = [self.catalog(), self.namespace(), self.relation()];
        let Some(premier) = paliers.iter().position(Option::is_some) else {
            return Ok(());
        };
        for (i, palier) in paliers.iter().enumerate().skip(premier) {
            if i > premier {
                f.write_char('.')?;
            }
            if let Some(nom) = palier {
                ecrire_segment(f, nom)?;
            }
        }
        Ok(())
    }
}

/// State of the segment parser.
enum EtatLecture {
    /// Outside a quote.
    Normal,
    /// Inside a quote.
    Cite,
    /// Right after a closed quote: only a separator is acceptable.
    ApresCitation,
}

/// Splits a string into segments, honoring `"…"` quotes.
///
/// Returns `None` for an empty segment, that is a missing level.
fn decouper(entree: &str) -> Result<Vec<Option<String>>, CatalogPathError> {
    let mut segments = Vec::with_capacity(PALIERS);
    let mut courant = String::new();
    let mut etat = EtatLecture::Normal;
    let mut caracteres = entree.chars().peekable();

    while let Some(c) = caracteres.next() {
        match etat {
            EtatLecture::Cite => {
                if c == '"' {
                    if caracteres.peek() == Some(&'"') {
                        courant.push('"');
                        caracteres.next();
                    } else {
                        etat = EtatLecture::ApresCitation;
                    }
                } else {
                    courant.push(c);
                }
            }
            EtatLecture::ApresCitation => {
                if c != '.' {
                    return Err(CatalogPathError::new(
                        "a quoted level must be followed by a separator",
                    ));
                }
                segments.push(cloturer(&mut courant, true)?);
                etat = EtatLecture::Normal;
            }
            EtatLecture::Normal => match c {
                '"' if courant.is_empty() => etat = EtatLecture::Cite,
                '"' => {
                    return Err(CatalogPathError::new(
                        "a quote cannot start in the middle of a level",
                    ));
                }
                '.' => segments.push(cloturer(&mut courant, false)?),
                _ => courant.push(c),
            },
        }
    }

    match etat {
        EtatLecture::Cite => Err(CatalogPathError::new("unterminated quote")),
        EtatLecture::ApresCitation => {
            segments.push(cloturer(&mut courant, true)?);
            Ok(segments)
        }
        EtatLecture::Normal => {
            segments.push(cloturer(&mut courant, false)?);
            Ok(segments)
        }
    }
}

/// Closes the current segment.
fn cloturer(courant: &mut String, etait_cite: bool) -> Result<Option<String>, CatalogPathError> {
    let valeur = std::mem::take(courant);
    if valeur.is_empty() {
        if etait_cite {
            return Err(CatalogPathError::new("a quoted level cannot be empty"));
        }
        return Ok(None);
    }
    validate_segment(&valeur)?;
    Ok(Some(valeur))
}

impl FromStr for CatalogPath {
    type Err = CatalogPathError;

    /// Parses a path by aligning the segments **from the right**: the last
    /// segment is the relation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let segments = decouper(s)?;
        if segments.len() > PALIERS {
            return Err(CatalogPathError::new("more than three levels"));
        }
        let mut paliers: [Option<String>; PALIERS] = [None, None, None];
        let decalage = PALIERS - segments.len();
        for (i, segment) in segments.into_iter().enumerate() {
            if let Some(emplacement) = paliers.get_mut(decalage + i) {
                *emplacement = segment;
            }
        }
        let [catalog, namespace, relation] = paliers;
        Ok(Self {
            catalog,
            namespace,
            relation,
        })
    }
}

impl TryFrom<String> for CatalogPath {
    type Error = CatalogPathError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<CatalogPath> for String {
    fn from(path: CatalogPath) -> Self {
        path.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table name of [I-10]: legal in PostgreSQL, and a table drop if it
    /// is concatenated.
    const NOM_HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

    #[test]
    fn missing_levels_are_tolerated() {
        let postgres = CatalogPath::for_relation(Some("caisse"), Some("public"), "clients")
            .expect("valid path");
        assert_eq!(postgres.depth(), 3);

        let mysql = CatalogPath::for_relation(None, Some("caisse"), "clients").expect("valid");
        assert_eq!(mysql.depth(), 2);
        assert_eq!(mysql.catalog(), None);

        let elasticsearch = CatalogPath::for_relation(None, None, "journaux").expect("valid");
        assert_eq!(elasticsearch.depth(), 1);
        assert_eq!(elasticsearch.level(), CatalogLevel::Relation);
    }

    #[test]
    fn the_rendering_is_natural_for_common_cases() {
        assert_eq!(
            CatalogPath::for_relation(Some("caisse"), Some("public"), "clients")
                .expect("valid")
                .to_string(),
            "caisse.public.clients"
        );
        assert_eq!(
            CatalogPath::for_relation(None, Some("caisse"), "clients")
                .expect("valid")
                .to_string(),
            "caisse.clients"
        );
        assert_eq!(
            CatalogPath::for_relation(None, None, "journaux")
                .expect("valid")
                .to_string(),
            "journaux"
        );
        assert_eq!(CatalogPath::empty().to_string(), "");
    }

    #[test]
    fn a_hole_in_the_middle_renders_and_reads_back() {
        // Neo4j: a database and a node label, without an intermediate level.
        let neo4j = CatalogPath::for_relation(Some("graphe"), None, "Personne").expect("valid");
        assert_eq!(neo4j.to_string(), "graphe..Personne");

        let relu: CatalogPath = "graphe..Personne".parse().expect("readable");
        assert_eq!(relu, neo4j);
        assert_eq!(relu.catalog(), Some("graphe"));
        assert_eq!(relu.namespace(), None);
    }

    #[test]
    fn a_prefix_path_keeps_its_level() {
        // The case that breaks without the trailing dots: `caisse` alone would
        // be read back as a relation, and the catalog level would vanish.
        let catalogue = CatalogPath::for_catalog("caisse").expect("valid");
        assert_eq!(catalogue.to_string(), "caisse..");
        assert_eq!(
            catalogue
                .to_string()
                .parse::<CatalogPath>()
                .expect("read back"),
            catalogue
        );

        let espace = CatalogPath::for_namespace(Some("caisse"), "public").expect("valid");
        assert_eq!(espace.to_string(), "caisse.public.");
        assert_eq!(
            espace
                .to_string()
                .parse::<CatalogPath>()
                .expect("read back"),
            espace
        );

        let espace_seul = CatalogPath::for_namespace(None, "public").expect("valid");
        assert_eq!(espace_seul.to_string(), "public.");
        assert_eq!(
            espace_seul
                .to_string()
                .parse::<CatalogPath>()
                .expect("read back"),
            espace_seul
        );
    }

    #[test]
    fn round_trip_over_every_level_arrangement() {
        for catalogue in [None, Some("c")] {
            for espace in [None, Some("n")] {
                for relation in [None, Some("r")] {
                    let chemin = CatalogPath::from_levels(
                        catalogue.map(str::to_owned),
                        espace.map(str::to_owned),
                        relation.map(str::to_owned),
                    )
                    .expect("valid");
                    let rendu = chemin.to_string();
                    let relu: CatalogPath = rendu.parse().expect("readable");
                    assert_eq!(relu, chemin, "round trip broken for {rendu:?}");
                }
            }
        }
    }

    #[test]
    fn a_name_containing_a_dot_is_quoted_when_rendered() {
        let chemin = CatalogPath::for_relation(None, Some("public"), "ventes.2026").expect("valid");
        assert_eq!(chemin.to_string(), r#"public."ventes.2026""#);
        let relu: CatalogPath = chemin.to_string().parse().expect("readable");
        assert_eq!(relu, chemin);
        assert_eq!(relu.relation(), Some("ventes.2026"));
    }

    #[test]
    fn a_name_containing_a_double_quote_is_quoted_when_rendered() {
        let chemin = CatalogPath::for_relation(None, None, NOM_HOSTILE).expect("valid");
        let relu: CatalogPath = chemin.to_string().parse().expect("readable");
        assert_eq!(relu, chemin);
        assert_eq!(relu.relation(), Some(NOM_HOSTILE));
    }

    #[test]
    fn qualify_quotes_a_hostile_name() {
        // I-10: SQL composed by Oxyn never concatenates a received identifier.
        let chemin = CatalogPath::for_relation(None, Some("public"), NOM_HOSTILE).expect("valid");

        let ansi = chemin.qualify(QuoteStyle::Double);
        assert_eq!(ansi, r#""public"."users""; DROP TABLE audit; --""#);
        // The semicolon stays inside the quoting: counted in double quotes, the
        // identifier is whole.
        assert_eq!(ansi.matches('"').count() % 2, 0);

        let mysql = chemin.qualify(QuoteStyle::Backtick);
        assert_eq!(mysql, r#"`public`.`users"; DROP TABLE audit; --`"#);

        let tsql = chemin.qualify(QuoteStyle::Bracket);
        assert_eq!(tsql, r#"[public].[users"; DROP TABLE audit; --]"#);
    }

    #[test]
    fn qualify_doubles_the_closing_character() {
        assert_eq!(quote_identifier(r#"a"b"#, QuoteStyle::Double), r#""a""b""#);
        assert_eq!(quote_identifier("a`b", QuoteStyle::Backtick), "`a``b`");
        assert_eq!(quote_identifier("a]b", QuoteStyle::Bracket), "[a]]b]");
        assert_eq!(quote_identifier("a]b", QuoteStyle::Bare), "a]b");
    }

    #[test]
    fn qualify_ignores_missing_levels() {
        let neo4j = CatalogPath::for_relation(Some("graphe"), None, "Personne").expect("valid");
        // No empty `.` in query text: it would be a syntax error, where the
        // reversible rendering needs it.
        assert_eq!(neo4j.qualify(QuoteStyle::Double), r#""graphe"."Personne""#);
    }

    #[test]
    fn qualify_sql_never_leaves_a_bare_name() {
        let chemin = CatalogPath::for_relation(None, None, NOM_HOSTILE).expect("valid");
        for dialecte in [
            SqlDialect::Ansi,
            SqlDialect::Postgres,
            SqlDialect::MySql,
            SqlDialect::Sqlite,
            SqlDialect::SqlServer,
            SqlDialect::Oracle,
            SqlDialect::ClickHouse,
            SqlDialect::DuckDb,
            SqlDialect::Snowflake,
            SqlDialect::BigQuery,
            SqlDialect::Redshift,
        ] {
            let rendu = chemin.qualify_sql(dialecte);
            assert_ne!(rendu, NOM_HOSTILE, "{dialecte} leaves the name bare");
            let premier = rendu.chars().next().expect("non-empty rendering");
            assert!(
                matches!(premier, '"' | '`' | '['),
                "{dialecte}: {rendu} does not start with a quote"
            );
        }
    }

    #[test]
    fn an_empty_or_control_level_is_refused() {
        assert!(CatalogPath::for_relation(None, None, "").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\u{1b}[31mb").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\nb").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\0b").is_err());
    }

    #[test]
    fn an_error_does_not_copy_the_value() {
        let err = "a\u{1b}[2Jb"
            .parse::<CatalogPath>()
            .expect_err("control character");
        assert!(!err.to_string().contains('\u{1b}'), "the value leaked");
    }

    #[test]
    fn malformed_paths_are_refused() {
        assert!("a.b.c.d".parse::<CatalogPath>().is_err());
        assert!(r#""non fermé"#.parse::<CatalogPath>().is_err());
        assert!(r#"a"b""#.parse::<CatalogPath>().is_err());
        assert!(r#""a"b"#.parse::<CatalogPath>().is_err());
        assert!(r#""".t"#.parse::<CatalogPath>().is_err());
    }

    #[test]
    fn parent_climbs_level_by_level() {
        let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valid");
        let espace = relation.parent().expect("a relation has a parent");
        assert_eq!(espace.level(), CatalogLevel::Namespace);
        let catalogue = espace.parent().expect("a namespace has a parent");
        assert_eq!(catalogue.level(), CatalogLevel::Catalog);
        let serveur = catalogue.parent().expect("a catalog has a parent");
        assert!(serveur.is_empty());
        assert_eq!(serveur.parent(), None);
    }

    #[test]
    fn starts_with_ignores_unrequired_levels() {
        let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valid");
        assert!(relation.starts_with(&CatalogPath::empty()));
        assert!(relation.starts_with(&CatalogPath::for_catalog("c").expect("valid")));
        assert!(relation.starts_with(&CatalogPath::for_namespace(Some("c"), "n").expect("valid")));
        assert!(!relation.starts_with(&CatalogPath::for_catalog("autre").expect("valid")));
    }

    #[test]
    fn the_missing_segment_cannot_be_a_name() {
        // The empty string serves as the "missing level" key in the cache
        // (`crate::cache`): it must never be able to designate a real object,
        // otherwise a table named `""` would overwrite the "missing level" node.
        assert!(validate_segment("").is_err());
    }
}
