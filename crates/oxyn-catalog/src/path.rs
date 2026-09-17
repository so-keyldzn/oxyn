//! Chemin qualifié dans la hiérarchie du catalogue.
//!
//! La hiérarchie d'ARCHITECTURE §6 compte cinq paliers, dont **trois** portent
//! un nom : `Catalog`, `Namespace`, `Relation`. Chacun peut manquer, et pas
//! seulement les derniers :
//!
//! | Système | Catalogue | Espace de noms | Relation | Rendu |
//! |---|---|---|---|---|
//! | PostgreSQL | `base` | `schema` | `table` | `base.schema.table` |
//! | MySQL | — | `base` | `table` | `base.table` |
//! | MongoDB | — | `base` | `collection` | `base.collection` |
//! | Elasticsearch | — | — | `index` | `index` |
//! | Neo4j | `base` | — | `label` | `base..label` |
//!
//! # La convention qui rend le rendu réversible
//!
//! Un chemin se lit **par la droite** : le dernier segment est toujours la
//! relation, l'avant-dernier l'espace de noms, le premier le catalogue. Un
//! segment vide dénote un palier absent. C'est ce qui permet à
//! [`Display`](fmt::Display) et [`FromStr`] d'être exactement inverses l'un de
//! l'autre — y compris pour un trou au milieu (`base..label`) et pour un chemin
//! qui s'arrête avant la relation (`base.schema.`).
//!
//! Sans cette convention, `base.label` serait relu comme
//! `espace_de_noms.relation` et le palier catalogue de Neo4j disparaîtrait en
//! silence à chaque aller-retour par le disque.
//!
//! # Citation
//!
//! Un nom de table peut contenir un point, un guillemet, un point-virgule.
//! `"users"; DROP TABLE audit; --` est un nom de table légal dans PostgreSQL.
//! [`CatalogPath::qualify`] est donc **la seule** façon de composer un
//! identifiant qualifié pour une requête ([I-10] et
//! [`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md)) : elle cite chaque
//! segment selon le dialecte visé. Concaténer `format!("{path}")` dans du SQL
//! est le défaut que cet invariant interdit.
//!
//! [I-10]: ../../../CLAUDE.md

use std::fmt;
use std::fmt::Write as _;
use std::str::FromStr;

use oxyn_core::query::SqlDialect;
use serde::{Deserialize, Serialize};

/// Nombre de paliers nommables dans un chemin.
const PALIERS: usize = 3;

/// Échec d'analyse ou de construction d'un chemin de catalogue.
///
/// Le texte fautif n'est **jamais** repris dans le message, par cohérence avec
/// [`IdParseError`](oxyn_core::IdParseError) : un nom d'objet peut contenir des
/// séquences d'échappement de terminal, et un message d'erreur finit dans un
/// journal ([SECURITY, surface d'entrée §2](../../../docs/SECURITY.md)).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error("invalid catalog path: {detail}")]
pub struct CatalogPathError {
    detail: &'static str,
}

impl CatalogPathError {
    /// Construit une erreur d'analyse.
    #[must_use]
    pub const fn new(detail: &'static str) -> Self {
        Self { detail }
    }

    /// Raison du rejet, sans reprendre la valeur fautive.
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

/// Vérifie qu'un nom de palier est utilisable.
///
/// Deux refus, et seulement deux :
///
/// * la chaîne vide, parce qu'elle dénote l'absence de palier dans le rendu
///   comme dans les clés du cache ;
/// * les caractères de contrôle, parce qu'un nom d'objet est une entrée hostile
///   ([SECURITY, surface d'entrée §2](../../../docs/SECURITY.md)) et qu'une
///   séquence d'échappement ANSI dans un nom de table repeint le terminal ou la
///   ligne de journal qui l'affiche.
///
/// Les points, guillemets, espaces et points-virgules sont **acceptés** : ce
/// sont des noms légaux, et [`CatalogPath::qualify`] sait les citer.
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

/// Style de citation d'un identifiant.
///
/// Le style n'est pas un détail cosmétique : c'est ce qui sépare un aperçu de
/// table d'une exécution de `DROP TABLE audit` ([I-10]).
///
/// [I-10]: ../../../CLAUDE.md
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum QuoteStyle {
    /// Guillemets doubles ANSI : `"nom"`, guillemet interne doublé. Défaut, et
    /// repli sûr pour tout dialecte inconnu.
    #[default]
    Double,
    /// Accents graves (MySQL, BigQuery) : `` `nom` ``, accent interne doublé.
    Backtick,
    /// Crochets (T-SQL) : `[nom]`, crochet fermant doublé.
    Bracket,
    /// **Aucune citation.**
    ///
    /// Réservé aux sources qui ne composent pas de texte de requête à partir
    /// d'un nom d'objet — MongoDB, Redis, Elasticsearch, où le nom est un champ
    /// de document ou un segment d'URL, pas un fragment de langage. L'employer
    /// pour composer du SQL viole [I-10] ; utiliser
    /// [`CatalogPath::qualify_sql`], qui ne le choisit jamais.
    ///
    /// [I-10]: ../../../CLAUDE.md
    Bare,
}

impl QuoteStyle {
    /// Le style de citation d'un dialecte SQL.
    ///
    /// Un dialecte non reconnu retombe sur [`Double`](Self::Double) : le repli
    /// d'un modèle de citation est un autre modèle de citation, jamais
    /// [`Bare`](Self::Bare).
    #[must_use]
    pub const fn for_dialect(dialect: SqlDialect) -> Self {
        match dialect {
            SqlDialect::MySql | SqlDialect::BigQuery => Self::Backtick,
            SqlDialect::SqlServer => Self::Bracket,
            // `SqlDialect` est `#[non_exhaustive]` : le bras générique est
            // imposé par le langage, et il vaut mieux ici que l'énumération des
            // dialectes ANSI, qui se périmerait en silence.
            _ => Self::Double,
        }
    }
}

/// Cite un identifiant selon un style.
///
/// Ne valide rien : un nom vide ou porteur de caractères de contrôle ressort
/// cité tel quel. La validation est faite à la construction d'un
/// [`CatalogPath`].
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
        // Dans les trois styles, seul le caractère fermant a besoin d'être
        // doublé — c'est le seul qui peut clore la citation par surprise.
        if c == fermant {
            sortie.push(c);
        }
        sortie.push(c);
    }
    sortie.push(fermant);
    sortie
}

/// Le palier le plus profond qu'un chemin désigne.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogLevel {
    /// Chemin vide : le serveur lui-même.
    Server,
    /// Le chemin s'arrête au catalogue.
    Catalog,
    /// Le chemin s'arrête à l'espace de noms.
    Namespace,
    /// Le chemin désigne une relation.
    Relation,
}

impl CatalogLevel {
    /// Nom stable, pour l'audit et l'interface.
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

/// Chemin qualifié, tolérant aux paliers absents.
///
/// Les trois paliers sont indépendants : un chemin peut avoir un catalogue et
/// une relation sans espace de noms (Neo4j), ou seulement une relation
/// (Elasticsearch).
///
/// Sérialisé sous forme de **chaîne** — le rendu de [`Display`](fmt::Display) —
/// et non d'objet à trois champs : un fichier de workspace lisible sans Oxyn est
/// un invariant du produit (I-11), et l'aller-retour est exact.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize)]
#[serde(into = "String", try_from = "String")]
pub struct CatalogPath {
    catalog: Option<String>,
    namespace: Option<String>,
    relation: Option<String>,
}

impl CatalogPath {
    /// Le chemin vide : le serveur lui-même.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            catalog: None,
            namespace: None,
            relation: None,
        }
    }

    /// Construit un chemin à partir de ses trois paliers.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si un palier présent est vide ou contient un
    /// caractère de contrôle.
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

    /// Construit un chemin à partir de paliers **déjà validés**.
    ///
    /// Réservé à la crate : les références du modèle ([`RelationRef`] et ses
    /// voisines) valident leur nom à la construction, et reconstruire leur
    /// chemin ne peut donc pas échouer. Une fonction faillible ici obligerait
    /// chaque appelant à traiter une erreur impossible.
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

    /// Chemin désignant un catalogue.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est vide ou contient un caractère
    /// de contrôle.
    pub fn for_catalog(name: impl Into<String>) -> Result<Self, CatalogPathError> {
        Self::from_levels(Some(name.into()), None, None)
    }

    /// Chemin désignant un espace de noms, sous un catalogue éventuel.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si un nom est vide ou contient un caractère
    /// de contrôle.
    pub fn for_namespace(
        catalog: Option<&str>,
        name: impl Into<String>,
    ) -> Result<Self, CatalogPathError> {
        Self::from_levels(catalog.map(str::to_owned), Some(name.into()), None)
    }

    /// Chemin désignant une relation, sous les paliers éventuels qui la
    /// contiennent.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si un nom est vide ou contient un caractère
    /// de contrôle.
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

    /// Le palier catalogue, s'il est présent.
    #[must_use]
    pub fn catalog(&self) -> Option<&str> {
        self.catalog.as_deref()
    }

    /// Le palier espace de noms, s'il est présent.
    #[must_use]
    pub fn namespace(&self) -> Option<&str> {
        self.namespace.as_deref()
    }

    /// Le palier relation, s'il est présent.
    #[must_use]
    pub fn relation(&self) -> Option<&str> {
        self.relation.as_deref()
    }

    /// Le nom du palier le plus profond présent.
    #[must_use]
    pub fn leaf(&self) -> Option<&str> {
        self.relation()
            .or_else(|| self.namespace())
            .or_else(|| self.catalog())
    }

    /// Le palier le plus profond présent.
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

    /// Le chemin ne désigne-t-il aucun palier ?
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.catalog.is_none() && self.namespace.is_none() && self.relation.is_none()
    }

    /// Nombre de paliers présents. Un trou ne compte pas.
    #[must_use]
    pub fn depth(&self) -> usize {
        usize::from(self.catalog.is_some())
            + usize::from(self.namespace.is_some())
            + usize::from(self.relation.is_some())
    }

    /// Les paliers présents, du plus général au plus précis.
    pub fn segments(&self) -> impl Iterator<Item = &str> + '_ {
        [self.catalog(), self.namespace(), self.relation()]
            .into_iter()
            .flatten()
    }

    /// Le chemin obtenu en retirant le palier le plus profond.
    ///
    /// Rend `None` pour le chemin vide.
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

    /// Le même chemin, complété d'un palier espace de noms.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est invalide.
    pub fn with_namespace(&self, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            catalog: self.catalog.clone(),
            namespace: Some(name),
            relation: self.relation.clone(),
        })
    }

    /// Le même chemin, complété d'un palier relation.
    ///
    /// # Erreurs
    /// Renvoie [`CatalogPathError`] si le nom est invalide.
    pub fn with_relation(&self, name: impl Into<String>) -> Result<Self, CatalogPathError> {
        let name = name.into();
        validate_segment(&name)?;
        Ok(Self {
            catalog: self.catalog.clone(),
            namespace: self.namespace.clone(),
            relation: Some(name),
        })
    }

    /// Le même chemin, complété d'un palier relation **déjà validé**.
    pub(crate) fn with_validated_relation(&self, name: &str) -> Self {
        Self {
            catalog: self.catalog.clone(),
            namespace: self.namespace.clone(),
            relation: Some(name.to_owned()),
        }
    }

    /// Rend l'identifiant qualifié, chaque palier présent cité selon `quote`.
    ///
    /// C'est **la** façon de composer un identifiant dans une requête produite
    /// par Oxyn ([I-10]). Les paliers absents ne laissent pas de point vide : le
    /// résultat est du texte de requête, pas un rendu réversible.
    ///
    /// ```
    /// use oxyn_catalog::path::{CatalogPath, QuoteStyle};
    ///
    /// let chemin = CatalogPath::for_relation(None, Some("public"), r#"users"; DROP TABLE audit; --"#)
    ///     .expect("le nom est légal, seulement hostile");
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

    /// Rend l'identifiant qualifié pour un dialecte SQL.
    ///
    /// Ne choisit jamais [`QuoteStyle::Bare`] : un dialecte inconnu est cité en
    /// ANSI plutôt que laissé nu.
    #[must_use]
    pub fn qualify_sql(&self, dialect: SqlDialect) -> String {
        self.qualify(QuoteStyle::for_dialect(dialect))
    }

    /// Ce chemin est-il situé sous `prefix` ?
    ///
    /// Un palier absent dans `prefix` n'impose rien ; un palier présent doit
    /// correspondre exactement. Le chemin vide contient tout.
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

/// Écrit un segment, cité si — et seulement si — le rendu resterait ambigu.
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
    /// Rend le chemin sous une forme relisible par [`FromStr`].
    ///
    /// Les paliers absents **entre** le premier palier présent et la relation
    /// laissent un segment vide (`base..label`), et un chemin qui s'arrête avant
    /// la relation garde ses points de fin (`base.schema.`). C'est ce qui rend
    /// l'aller-retour exact ; voir la documentation du module.
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

/// État de l'analyseur de segments.
enum EtatLecture {
    /// Hors citation.
    Normal,
    /// À l'intérieur d'une citation.
    Cite,
    /// Juste après une citation fermée : seul un séparateur est acceptable.
    ApresCitation,
}

/// Découpe une chaîne en segments, en respectant les citations `"…"`.
///
/// Rend `None` pour un segment vide, c'est-à-dire un palier absent.
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

/// Clôt le segment en cours.
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

    /// Analyse un chemin en alignant les segments **par la droite** : le dernier
    /// segment est la relation.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let segments = decouper(s)?;
        if segments.len() > PALIERS {
            return Err(CatalogPathError::new("plus de trois paliers"));
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

    /// Le nom de table de [I-10] : légal dans PostgreSQL, et une suppression de
    /// table si on le concatène.
    const NOM_HOSTILE: &str = r#"users"; DROP TABLE audit; --"#;

    #[test]
    fn les_paliers_absents_sont_tolerés() {
        let postgres = CatalogPath::for_relation(Some("caisse"), Some("public"), "clients")
            .expect("chemin valide");
        assert_eq!(postgres.depth(), 3);

        let mysql = CatalogPath::for_relation(None, Some("caisse"), "clients").expect("valide");
        assert_eq!(mysql.depth(), 2);
        assert_eq!(mysql.catalog(), None);

        let elasticsearch = CatalogPath::for_relation(None, None, "journaux").expect("valide");
        assert_eq!(elasticsearch.depth(), 1);
        assert_eq!(elasticsearch.level(), CatalogLevel::Relation);
    }

    #[test]
    fn le_rendu_est_naturel_pour_les_cas_courants() {
        assert_eq!(
            CatalogPath::for_relation(Some("caisse"), Some("public"), "clients")
                .expect("valide")
                .to_string(),
            "caisse.public.clients"
        );
        assert_eq!(
            CatalogPath::for_relation(None, Some("caisse"), "clients")
                .expect("valide")
                .to_string(),
            "caisse.clients"
        );
        assert_eq!(
            CatalogPath::for_relation(None, None, "journaux")
                .expect("valide")
                .to_string(),
            "journaux"
        );
        assert_eq!(CatalogPath::empty().to_string(), "");
    }

    #[test]
    fn un_trou_au_milieu_se_rend_et_se_relit() {
        // Neo4j : une base et un label de nœud, sans palier intermédiaire.
        let neo4j = CatalogPath::for_relation(Some("graphe"), None, "Personne").expect("valide");
        assert_eq!(neo4j.to_string(), "graphe..Personne");

        let relu: CatalogPath = "graphe..Personne".parse().expect("relisible");
        assert_eq!(relu, neo4j);
        assert_eq!(relu.catalog(), Some("graphe"));
        assert_eq!(relu.namespace(), None);
    }

    #[test]
    fn un_chemin_prefixe_garde_son_palier() {
        // Le cas qui casse sans les points de fin : `caisse` seul serait relu
        // comme une relation, et le palier catalogue disparaîtrait.
        let catalogue = CatalogPath::for_catalog("caisse").expect("valide");
        assert_eq!(catalogue.to_string(), "caisse..");
        assert_eq!(
            catalogue.to_string().parse::<CatalogPath>().expect("relu"),
            catalogue
        );

        let espace = CatalogPath::for_namespace(Some("caisse"), "public").expect("valide");
        assert_eq!(espace.to_string(), "caisse.public.");
        assert_eq!(
            espace.to_string().parse::<CatalogPath>().expect("relu"),
            espace
        );

        let espace_seul = CatalogPath::for_namespace(None, "public").expect("valide");
        assert_eq!(espace_seul.to_string(), "public.");
        assert_eq!(
            espace_seul
                .to_string()
                .parse::<CatalogPath>()
                .expect("relu"),
            espace_seul
        );
    }

    #[test]
    fn aller_retour_sur_tous_les_agencements_de_paliers() {
        for catalogue in [None, Some("c")] {
            for espace in [None, Some("n")] {
                for relation in [None, Some("r")] {
                    let chemin = CatalogPath::from_levels(
                        catalogue.map(str::to_owned),
                        espace.map(str::to_owned),
                        relation.map(str::to_owned),
                    )
                    .expect("valide");
                    let rendu = chemin.to_string();
                    let relu: CatalogPath = rendu.parse().expect("relisible");
                    assert_eq!(relu, chemin, "aller-retour cassé pour {rendu:?}");
                }
            }
        }
    }

    #[test]
    fn un_nom_contenant_un_point_se_cite_au_rendu() {
        let chemin =
            CatalogPath::for_relation(None, Some("public"), "ventes.2026").expect("valide");
        assert_eq!(chemin.to_string(), r#"public."ventes.2026""#);
        let relu: CatalogPath = chemin.to_string().parse().expect("relisible");
        assert_eq!(relu, chemin);
        assert_eq!(relu.relation(), Some("ventes.2026"));
    }

    #[test]
    fn un_nom_contenant_un_guillemet_se_cite_au_rendu() {
        let chemin = CatalogPath::for_relation(None, None, NOM_HOSTILE).expect("valide");
        let relu: CatalogPath = chemin.to_string().parse().expect("relisible");
        assert_eq!(relu, chemin);
        assert_eq!(relu.relation(), Some(NOM_HOSTILE));
    }

    #[test]
    fn qualify_cite_un_nom_hostile() {
        // I-10 : le SQL composé par Oxyn ne concatène jamais un identifiant reçu.
        let chemin = CatalogPath::for_relation(None, Some("public"), NOM_HOSTILE).expect("valide");

        let ansi = chemin.qualify(QuoteStyle::Double);
        assert_eq!(ansi, r#""public"."users""; DROP TABLE audit; --""#);
        // Le point-virgule reste à l'intérieur de la citation : compté en
        // guillemets, l'identifiant est entier.
        assert_eq!(ansi.matches('"').count() % 2, 0);

        let mysql = chemin.qualify(QuoteStyle::Backtick);
        assert_eq!(mysql, r#"`public`.`users"; DROP TABLE audit; --`"#);

        let tsql = chemin.qualify(QuoteStyle::Bracket);
        assert_eq!(tsql, r#"[public].[users"; DROP TABLE audit; --]"#);
    }

    #[test]
    fn qualify_double_le_caractere_fermant() {
        assert_eq!(quote_identifier(r#"a"b"#, QuoteStyle::Double), r#""a""b""#);
        assert_eq!(quote_identifier("a`b", QuoteStyle::Backtick), "`a``b`");
        assert_eq!(quote_identifier("a]b", QuoteStyle::Bracket), "[a]]b]");
        assert_eq!(quote_identifier("a]b", QuoteStyle::Bare), "a]b");
    }

    #[test]
    fn qualify_ignore_les_paliers_absents() {
        let neo4j = CatalogPath::for_relation(Some("graphe"), None, "Personne").expect("valide");
        // Pas de `.` vide dans du texte de requête : ce serait une erreur de
        // syntaxe, là où le rendu réversible en a besoin.
        assert_eq!(neo4j.qualify(QuoteStyle::Double), r#""graphe"."Personne""#);
    }

    #[test]
    fn qualify_sql_ne_laisse_jamais_un_nom_nu() {
        let chemin = CatalogPath::for_relation(None, None, NOM_HOSTILE).expect("valide");
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
            assert_ne!(rendu, NOM_HOSTILE, "{dialecte} laisse le nom nu");
            let premier = rendu.chars().next().expect("rendu non vide");
            assert!(
                matches!(premier, '"' | '`' | '['),
                "{dialecte} : {rendu} ne commence pas par une citation"
            );
        }
    }

    #[test]
    fn un_palier_vide_ou_de_controle_est_refuse() {
        assert!(CatalogPath::for_relation(None, None, "").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\u{1b}[31mb").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\nb").is_err());
        assert!(CatalogPath::for_relation(None, None, "a\0b").is_err());
    }

    #[test]
    fn une_erreur_ne_recopie_pas_la_valeur() {
        let err = "a\u{1b}[2Jb"
            .parse::<CatalogPath>()
            .expect_err("caractère de contrôle");
        assert!(!err.to_string().contains('\u{1b}'), "la valeur a fuité");
    }

    #[test]
    fn les_chemins_malformes_sont_refuses() {
        assert!("a.b.c.d".parse::<CatalogPath>().is_err());
        assert!(r#""non fermé"#.parse::<CatalogPath>().is_err());
        assert!(r#"a"b""#.parse::<CatalogPath>().is_err());
        assert!(r#""a"b"#.parse::<CatalogPath>().is_err());
        assert!(r#""".t"#.parse::<CatalogPath>().is_err());
    }

    #[test]
    fn parent_remonte_palier_par_palier() {
        let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valide");
        let espace = relation.parent().expect("une relation a un parent");
        assert_eq!(espace.level(), CatalogLevel::Namespace);
        let catalogue = espace.parent().expect("un espace a un parent");
        assert_eq!(catalogue.level(), CatalogLevel::Catalog);
        let serveur = catalogue.parent().expect("un catalogue a un parent");
        assert!(serveur.is_empty());
        assert_eq!(serveur.parent(), None);
    }

    #[test]
    fn starts_with_ignore_les_paliers_non_exiges() {
        let relation = CatalogPath::for_relation(Some("c"), Some("n"), "r").expect("valide");
        assert!(relation.starts_with(&CatalogPath::empty()));
        assert!(relation.starts_with(&CatalogPath::for_catalog("c").expect("valide")));
        assert!(relation.starts_with(&CatalogPath::for_namespace(Some("c"), "n").expect("valide")));
        assert!(!relation.starts_with(&CatalogPath::for_catalog("autre").expect("valide")));
    }

    #[test]
    fn le_segment_absent_ne_peut_pas_etre_un_nom() {
        // La chaîne vide sert de clé « palier absent » dans le cache
        // (`crate::cache`) : elle ne doit jamais pouvoir désigner un objet réel,
        // sinon une table nommée `""` écraserait le nœud « palier absent ».
        assert!(validate_segment("").is_err());
    }
}
