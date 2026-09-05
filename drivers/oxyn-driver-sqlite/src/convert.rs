//! Des classes de stockage SQLite vers des colonnes Arrow.
//!
//! C'est le point difficile du driver, et il vient d'un fait que les autres
//! bases n'ont pas : **en SQLite, le type appartient à la valeur, pas à la
//! colonne.** Un `CREATE TABLE t(n INTEGER)` n'empêche pas
//! `INSERT INTO t VALUES('abc')` de ranger du texte. Le type déclaré n'est
//! qu'une *affinité*, c'est-à-dire une préférence de conversion.
//!
//! Arrow, lui, exige un type par colonne, **connu avant le premier lot et
//! stable pour toute la durée du flux**
//! ([`Cursor::schema`](oxyn_driver::Cursor::schema)). Les deux modèles ne se
//! recouvrent pas ; ce module est l'endroit où l'écart est payé, une fois, de
//! façon explicite.
//!
//! # Comment le type d'une colonne est décidé
//!
//! 1. **Ce que le premier lot a vu.** Le curseur met le premier lot de côté en
//!    valeurs brutes et note, colonne par colonne, les classes de stockage
//!    rencontrées. C'est la source la plus fiable : ce sont les données.
//! 2. **L'affinité déclarée**, quand aucune valeur n'a été vue — colonne vide,
//!    ou entièrement `NULL` dans le premier lot.
//! 3. **`Utf8`** en dernier recours : c'est le rendu qui perd le moins.
//!
//! Le treillis de fusion, quand plusieurs classes coexistent :
//!
//! | Classes vues | Type retenu | Pourquoi |
//! |---|---|---|
//! | INTEGER seul | `Int64` | exact |
//! | REAL seul | `Float64` | exact |
//! | TEXT (ou n'importe quoi + TEXT) | `Utf8` | un entier et un flottant se rendent en texte sans perte |
//! | INTEGER **et** REAL | `Utf8` | **aucun `f64` ne tient tous les `i64`** : au-delà de 2⁵³, la conversion corrompt |
//! | n'importe quoi + BLOB | `Binary` | seuls les octets contiennent tout : un blob n'a pas de rendu textuel |
//!
//! Le cas `INTEGER + REAL → Utf8` est celui qu'on rate : c'est exactement « un
//! `u64` au-delà de 2⁵³ ne survit pas à un passage par un flottant »
//! ([drivers.md](../../../.claude/rules/drivers.md)).
//!
//! # Ce que la colonne dit d'elle-même
//!
//! Une colonne dont le type a été déduit, ou dont les classes étaient mêlées, le
//! **déclare** dans les métadonnées de son `Field` Arrow — voir
//! [`METADATA_INFERRED`] et [`METADATA_STORAGE_CLASSES`]. Présenter une
//! inférence comme une vérité du serveur fait écrire des requêtes fausses en
//! confiance ([`DRIVER-CONTRACT` §3](../../../docs/DRIVER-CONTRACT.md)).
//!
//! # Le cas résiduel
//!
//! Une valeur qui apparaît **après** le premier lot peut ne pas tenir dans le
//! type retenu : un BLOB au dix-millième rang d'une colonne vue comme textuelle.
//! Le curseur rend alors [`SqliteError::ColumnConflict`], une erreur permanente
//! qui nomme la colonne et les deux types. C'est bruyant — et c'est le point :
//! l'alternative serait une valeur silencieusement fausse à l'écran.

use std::collections::HashMap;
use std::fmt;
use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{ArrayRef, BinaryBuilder, Float64Builder, Int64Builder, StringBuilder};
use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
use rusqlite::types::ValueRef;

use crate::error::SqliteError;

/// Clé de métadonnée d'un `Field` Arrow : le type de la colonne a été **déduit**
/// des valeurs, pas lu dans une déclaration du schéma.
pub const METADATA_INFERRED: &str = "oxyn.inferred";

/// Clé de métadonnée d'un `Field` Arrow : le type tel que SQLite le déclare
/// (`INTEGER`, `VARCHAR(255)`, `NUMERIC(10,2)`…), quand il en déclare un.
pub const METADATA_DECLARED_TYPE: &str = "oxyn.sqlite.declared_type";

/// Clé de métadonnée d'un `Field` Arrow : les classes de stockage réellement
/// rencontrées, séparées par `|`, quand la colonne en mêle plusieurs.
///
/// Sa présence est le signal qu'une colonne mélange les types et que le rendu
/// retenu est un repli. L'interface doit pouvoir le montrer.
pub const METADATA_STORAGE_CLASSES: &str = "oxyn.sqlite.storage_classes";

/// Le plus grand entier que `f64` représente exactement : 2⁵³.
const EXACT_IN_F64: u64 = 1 << 53;

/// Le type Arrow retenu pour une colonne.
///
/// Quatre valeurs seulement, parce que SQLite n'a que cinq classes de stockage
/// et que `NULL` n'en est pas un type. Ramener un `DATETIME` déclaré vers
/// `Timestamp` serait une conversion, pas une lecture : SQLite y range du texte
/// ou un nombre selon ce que l'application a écrit, et deviner décalerait la
/// donnée ([`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md)). Le type
/// déclaré reste lisible dans [`METADATA_DECLARED_TYPE`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// Entier 64 bits signé.
    Int64,
    /// Flottant double précision.
    Float64,
    /// Texte UTF-8.
    Utf8,
    /// Octets opaques.
    Binary,
}

impl ColumnKind {
    /// Le type Arrow correspondant.
    #[must_use]
    pub fn data_type(self) -> DataType {
        match self {
            Self::Int64 => DataType::Int64,
            Self::Float64 => DataType::Float64,
            Self::Utf8 => DataType::Utf8,
            Self::Binary => DataType::Binary,
        }
    }

    /// Nom stable, celui qui apparaît dans un message d'erreur.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Int64 => "int64",
            Self::Float64 => "float64",
            Self::Utf8 => "utf8",
            Self::Binary => "binary",
        }
    }
}

impl fmt::Display for ColumnKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// L'affinité SQLite d'un type déclaré.
///
/// Applique les cinq règles de détermination d'affinité de SQLite, **dans leur
/// ordre**, y compris leurs conséquences surprenantes : `POINT` contient `INT`,
/// il reçoit donc l'affinité INTEGER. Reproduire la règle est le seul choix
/// juste — c'est ce que le moteur fera de la valeur.
///
/// Rend `None` quand rien n'est déclaré : une colonne sans type déclaré n'a pas
/// d'affinité, elle a l'affinité BLOB au sens de SQLite, mais nous préférons
/// dire « je ne sais pas » et laisser les valeurs décider.
#[must_use]
pub fn affinity(declared: Option<&str>) -> Option<ColumnKind> {
    let declared = declared?.trim();
    if declared.is_empty() {
        return None;
    }
    let upper = declared.to_ascii_uppercase();
    if upper.contains("INT") {
        return Some(ColumnKind::Int64);
    }
    if upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT") {
        return Some(ColumnKind::Utf8);
    }
    if upper.contains("BLOB") {
        return Some(ColumnKind::Binary);
    }
    if upper.contains("REAL") || upper.contains("FLOA") || upper.contains("DOUB") {
        return Some(ColumnKind::Float64);
    }
    // Affinité NUMERIC : SQLite range la valeur en entier, en flottant ou en
    // texte selon ce qui est exact. Aucun type Arrow ne couvre les trois ; le
    // texte est le seul rendu qui ne perd rien — c'est le cas de `DECIMAL(10,2)`.
    Some(ColumnKind::Utf8)
}

/// Les classes de stockage rencontrées dans une colonne.
///
/// `NULL` n'en est pas une : une colonne entièrement nulle n'a rien observé, et
/// c'est une information différente de « elle mélange les types ».
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Observed(u8);

impl Observed {
    const INTEGER: u8 = 1;
    const REAL: u8 = 1 << 1;
    const TEXT: u8 = 1 << 2;
    const BLOB: u8 = 1 << 3;

    /// Enregistre une valeur.
    ///
    /// Un TEXT qui n'est pas de l'UTF-8 valide compte pour des **octets** : il
    /// ne peut pas rejoindre une colonne `Utf8`, et le découvrir ici évite de
    /// s'en apercevoir au milieu du flux.
    pub(crate) fn observe(&mut self, value: ValueRef<'_>) {
        self.0 |= match value {
            ValueRef::Null => 0,
            ValueRef::Integer(_) => Self::INTEGER,
            ValueRef::Real(_) => Self::REAL,
            ValueRef::Text(bytes) if std::str::from_utf8(bytes).is_err() => Self::BLOB,
            ValueRef::Text(_) => Self::TEXT,
            ValueRef::Blob(_) => Self::BLOB,
        };
    }

    /// Aucune valeur non nulle n'a été vue.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// Plusieurs classes coexistent dans la colonne.
    #[must_use]
    pub const fn is_mixed(self) -> bool {
        self.0.count_ones() > 1
    }

    /// Les classes vues, séparées par `|`, dans l'ordre du treillis.
    #[must_use]
    pub fn names(self) -> String {
        let mut sortie = String::new();
        for (bit, nom) in [
            (Self::INTEGER, "integer"),
            (Self::REAL, "real"),
            (Self::TEXT, "text"),
            (Self::BLOB, "blob"),
        ] {
            if self.0 & bit != 0 {
                if !sortie.is_empty() {
                    sortie.push('|');
                }
                sortie.push_str(nom);
            }
        }
        sortie
    }

    /// Le type qui accueille toutes les classes vues sans perte.
    ///
    /// Voir la table du module. `None` quand rien n'a été vu.
    #[must_use]
    fn join(self) -> Option<ColumnKind> {
        if self.is_empty() {
            return None;
        }
        if self.0 & Self::BLOB != 0 {
            // Seuls les octets contiennent tout : un blob n'a pas de rendu
            // textuel, alors qu'un entier, un flottant et un texte ont tous une
            // représentation en octets.
            return Some(ColumnKind::Binary);
        }
        if self.0 & Self::TEXT != 0 {
            return Some(ColumnKind::Utf8);
        }
        if self.0 & Self::INTEGER != 0 && self.0 & Self::REAL != 0 {
            // Le piège : aucun `f64` ne tient tous les `i64`. Une colonne qui
            // mêle exacts et flottants se rend en texte, pas en `Float64`.
            return Some(ColumnKind::Utf8);
        }
        if self.0 & Self::REAL != 0 {
            return Some(ColumnKind::Float64);
        }
        Some(ColumnKind::Int64)
    }
}

/// Ce qui a été décidé pour une colonne du résultat.
#[derive(Debug, Clone)]
pub(crate) struct ColumnPlan {
    /// Nom rendu par SQLite. **Entrée hostile** : alias arbitraire choisi par
    /// l'utilisateur ou nom de colonne venu du schéma.
    pub name: String,
    /// Type déclaré, quand la colonne vient d'une table et non d'une expression.
    pub declared: Option<String>,
    /// Type Arrow retenu.
    pub kind: ColumnKind,
    /// Classes de stockage vues dans le premier lot.
    pub observed: Observed,
    /// Le type vient des valeurs, pas de la déclaration.
    pub inferred: bool,
}

impl ColumnPlan {
    /// Décide du type d'une colonne à partir de ce que le premier lot a vu et
    /// de ce que le schéma déclare.
    pub(crate) fn resolve(name: String, declared: Option<String>, observed: Observed) -> Self {
        let declared_kind = affinity(declared.as_deref());
        let kind = observed
            .join()
            .or(declared_kind)
            // Rien de vu, rien de déclaré : le texte perd le moins.
            .unwrap_or(ColumnKind::Utf8);
        Self {
            name,
            declared,
            kind,
            observed,
            // « Déduit » veut dire « le type retenu n'est pas celui que le
            // schéma implique » — que le schéma se taise, ou qu'il dise autre
            // chose que ce que les valeurs montrent.
            inferred: declared_kind != Some(kind),
        }
    }

    /// Le `Field` Arrow, métadonnées comprises.
    ///
    /// Le champ est **toujours** déclaré nullable : SQLite rend `NULL` pour
    /// n'importe quelle expression, et une colonne `NOT NULL` vue à travers une
    /// jointure externe l'est tout autant.
    fn field(&self) -> Field {
        let mut metadata = HashMap::new();
        if let Some(declared) = &self.declared {
            metadata.insert(METADATA_DECLARED_TYPE.to_owned(), declared.clone());
        }
        if self.inferred {
            metadata.insert(METADATA_INFERRED.to_owned(), "true".to_owned());
        }
        if self.observed.is_mixed() {
            metadata.insert(METADATA_STORAGE_CLASSES.to_owned(), self.observed.names());
        }
        Field::new(self.name.as_str(), self.kind.data_type(), true).with_metadata(metadata)
    }
}

/// Le schéma Arrow d'un ensemble de colonnes résolues.
pub(crate) fn schema_of(plans: &[ColumnPlan]) -> SchemaRef {
    Arc::new(Schema::new(
        plans.iter().map(ColumnPlan::field).collect::<Vec<_>>(),
    ))
}

/// Une valeur mise de côté le temps de résoudre le type de sa colonne.
///
/// Ne peut pas être `rusqlite::types::Value` : celui-ci range le TEXT dans un
/// `String`, donc perd — ou refuse — un texte qui n'est pas de l'UTF-8 valide.
/// Ici les octets sont conservés tels quels.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum ProbeValue {
    /// Absence de valeur.
    Null,
    /// Entier 64 bits.
    Integer(i64),
    /// Flottant double précision.
    Real(f64),
    /// Texte, **en octets** : rien ne garantit qu'il soit de l'UTF-8 valide.
    Text(Vec<u8>),
    /// Octets opaques.
    Blob(Vec<u8>),
}

impl ProbeValue {
    /// Copie une valeur empruntée au moteur.
    pub(crate) fn capture(value: ValueRef<'_>) -> Self {
        match value {
            ValueRef::Null => Self::Null,
            ValueRef::Integer(i) => Self::Integer(i),
            ValueRef::Real(x) => Self::Real(x),
            ValueRef::Text(bytes) => Self::Text(bytes.to_vec()),
            ValueRef::Blob(bytes) => Self::Blob(bytes.to_vec()),
        }
    }

    /// Vue empruntée, telle que le moteur l'aurait rendue.
    pub(crate) fn borrow(&self) -> ValueRef<'_> {
        match self {
            Self::Null => ValueRef::Null,
            Self::Integer(i) => ValueRef::Integer(*i),
            Self::Real(x) => ValueRef::Real(*x),
            Self::Text(bytes) => ValueRef::Text(bytes),
            Self::Blob(bytes) => ValueRef::Blob(bytes),
        }
    }
}

/// Les octets de données d'une valeur.
///
/// C'est **cette** mesure qui borne un lot, pas le nombre de lignes
/// ([`BatchLimits`](crate::BatchLimits)).
#[must_use]
pub(crate) fn value_bytes(value: ValueRef<'_>) -> usize {
    match value {
        ValueRef::Null => 0,
        ValueRef::Integer(_) | ValueRef::Real(_) => size_of::<i64>(),
        ValueRef::Text(bytes) | ValueRef::Blob(bytes) => bytes.len(),
    }
}

/// Le constructeur d'une colonne Arrow, réutilisé d'un lot au suivant.
///
/// Porte un tampon de rendu partagé : convertir un entier en texte allouerait
/// sinon une `String` par valeur, et la conversion vers `RecordBatch` est un
/// chemin **par valeur** ([rust.md](../../../.claude/rules/rust.md)).
#[derive(Debug)]
pub(crate) struct ColumnBuilder {
    inner: Inner,
    scratch: String,
    index: usize,
}

#[derive(Debug)]
enum Inner {
    Int64(Int64Builder),
    Float64(Float64Builder),
    Utf8(StringBuilder),
    Binary(BinaryBuilder),
}

impl ColumnBuilder {
    /// Un constructeur pour le type retenu, dimensionné pour `rows` lignes.
    pub(crate) fn new(kind: ColumnKind, index: usize, rows: usize) -> Self {
        let inner = match kind {
            ColumnKind::Int64 => Inner::Int64(Int64Builder::with_capacity(rows)),
            ColumnKind::Float64 => Inner::Float64(Float64Builder::with_capacity(rows)),
            // La seconde capacité est celle du tampon d'octets : seize octets
            // par ligne est une estimation basse qui évite les premières
            // réallocations sans réserver à l'aveugle.
            ColumnKind::Utf8 => {
                Inner::Utf8(StringBuilder::with_capacity(rows, rows.saturating_mul(16)))
            }
            ColumnKind::Binary => {
                Inner::Binary(BinaryBuilder::with_capacity(rows, rows.saturating_mul(16)))
            }
        };
        Self {
            inner,
            scratch: String::new(),
            index,
        }
    }

    /// Le type retenu.
    pub(crate) fn kind(&self) -> ColumnKind {
        match self.inner {
            Inner::Int64(_) => ColumnKind::Int64,
            Inner::Float64(_) => ColumnKind::Float64,
            Inner::Utf8(_) => ColumnKind::Utf8,
            Inner::Binary(_) => ColumnKind::Binary,
        }
    }

    /// Ajoute une valeur, ou refuse.
    ///
    /// Toutes les conversions acceptées sont **sans perte**. Celles qui
    /// perdraient — un flottant dans une colonne entière, un entier au-delà de
    /// 2⁵³ dans une colonne flottante, un blob dans une colonne textuelle —
    /// rendent [`SqliteError::ColumnConflict`] plutôt qu'une valeur fausse.
    ///
    /// # Erreurs
    /// [`SqliteError::ColumnConflict`] quand la valeur n'a pas de rendu sans
    /// perte dans le type de la colonne.
    pub(crate) fn append(&mut self, value: ValueRef<'_>) -> Result<(), SqliteError> {
        // L'erreur est construite d'avance : elle ne coûte que trois mots, et
        // la produire depuis un bras du `match` demanderait d'emprunter `self`
        // pendant qu'il l'est déjà.
        let conflict = SqliteError::ColumnConflict {
            column: self.index,
            resolved: self.kind().as_str(),
            found: storage_class_name(value),
        };
        // Le tampon de rendu sort du constructeur le temps de l'emprunt, puis
        // regagne sa place : il est réutilisé d'une valeur à la suivante.
        let mut scratch = std::mem::take(&mut self.scratch);
        let issue = append_into(&mut self.inner, &mut scratch, value, conflict);
        self.scratch = scratch;
        issue
    }

    /// Ferme le lot et rend la colonne. Le constructeur redevient vide.
    pub(crate) fn finish(&mut self) -> ArrayRef {
        match &mut self.inner {
            Inner::Int64(b) => Arc::new(b.finish()),
            Inner::Float64(b) => Arc::new(b.finish()),
            Inner::Utf8(b) => Arc::new(b.finish()),
            Inner::Binary(b) => Arc::new(b.finish()),
        }
    }
}

/// Le corps de [`ColumnBuilder::append`], sorti pour que le constructeur et son
/// tampon de rendu soient deux emprunts distincts.
fn append_into(
    inner: &mut Inner,
    scratch: &mut String,
    value: ValueRef<'_>,
    conflict: SqliteError,
) -> Result<(), SqliteError> {
    if matches!(value, ValueRef::Null) {
        match inner {
            Inner::Int64(b) => b.append_null(),
            Inner::Float64(b) => b.append_null(),
            Inner::Utf8(b) => b.append_null(),
            Inner::Binary(b) => b.append_null(),
        }
        return Ok(());
    }

    match (inner, value) {
        (Inner::Int64(b), ValueRef::Integer(i)) => b.append_value(i),
        (Inner::Float64(b), ValueRef::Real(x)) => b.append_value(x),
        (Inner::Float64(b), ValueRef::Integer(i)) => {
            if i.unsigned_abs() > EXACT_IN_F64 {
                return Err(conflict);
            }
            // Borné juste au-dessus : `f64` représente exactement tout entier
            // de valeur absolue ≤ 2⁵³, donc la conversion ne perd rien.
            #[allow(clippy::cast_precision_loss)]
            b.append_value(i as f64);
        }
        (Inner::Utf8(b), ValueRef::Text(bytes)) => {
            let Ok(texte) = std::str::from_utf8(bytes) else {
                return Err(conflict);
            };
            b.append_value(texte);
        }
        (Inner::Utf8(b), ValueRef::Integer(_) | ValueRef::Real(_)) => {
            render(scratch, value);
            b.append_value(&*scratch);
        }
        (Inner::Binary(b), ValueRef::Text(bytes) | ValueRef::Blob(bytes)) => b.append_value(bytes),
        (Inner::Binary(b), ValueRef::Integer(_) | ValueRef::Real(_)) => {
            render(scratch, value);
            b.append_value(scratch.as_bytes());
        }
        _ => return Err(conflict),
    }
    Ok(())
}

/// Écrit le rendu textuel d'un nombre dans le tampon partagé.
///
/// Le `Display` de `f64` produit la plus courte représentation qui se relit à
/// l'identique : la conversion est réversible.
fn render(scratch: &mut String, value: ValueRef<'_>) {
    scratch.clear();
    // Écrire dans une `String` ne peut pas échouer.
    let _ = match value {
        ValueRef::Integer(i) => write!(scratch, "{i}"),
        ValueRef::Real(x) => write!(scratch, "{x}"),
        _ => Ok(()),
    };
}

/// Le nom de la classe de stockage d'une valeur, pour un message d'erreur.
const fn storage_class_name(value: ValueRef<'_>) -> &'static str {
    match value {
        ValueRef::Null => "null",
        ValueRef::Integer(_) => "integer",
        ValueRef::Real(_) => "real",
        ValueRef::Text(_) => "text",
        ValueRef::Blob(_) => "blob",
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, BinaryArray, Float64Array, Int64Array, StringArray};

    use super::*;

    fn observed(values: &[ValueRef<'_>]) -> Observed {
        let mut vues = Observed::default();
        for value in values {
            vues.observe(*value);
        }
        vues
    }

    fn resolve(values: &[ValueRef<'_>], declared: Option<&str>) -> ColumnPlan {
        ColumnPlan::resolve(
            "c".to_owned(),
            declared.map(str::to_owned),
            observed(values),
        )
    }

    #[test]
    fn les_regles_d_affinite_de_sqlite_sont_reproduites_telles_quelles() {
        assert_eq!(affinity(Some("INTEGER")), Some(ColumnKind::Int64));
        assert_eq!(affinity(Some("BIGINT")), Some(ColumnKind::Int64));
        assert_eq!(affinity(Some("VARCHAR(255)")), Some(ColumnKind::Utf8));
        assert_eq!(affinity(Some("TEXT")), Some(ColumnKind::Utf8));
        assert_eq!(affinity(Some("BLOB")), Some(ColumnKind::Binary));
        assert_eq!(
            affinity(Some("DOUBLE PRECISION")),
            Some(ColumnKind::Float64)
        );
        assert_eq!(affinity(Some("REAL")), Some(ColumnKind::Float64));
        // Affinité NUMERIC : ni entier ni flottant ne la couvrent.
        assert_eq!(affinity(Some("DECIMAL(10,2)")), Some(ColumnKind::Utf8));
        // La conséquence surprenante de la règle 1, et elle est correcte :
        // SQLite donne bien l'affinité INTEGER à `POINT`.
        assert_eq!(affinity(Some("POINT")), Some(ColumnKind::Int64));
        // Rien de déclaré : pas d'affinité, on laissera les valeurs décider.
        assert_eq!(affinity(None), None);
        assert_eq!(affinity(Some("   ")), None);
    }

    #[test]
    fn une_colonne_homogene_garde_son_type_exact() {
        assert_eq!(
            resolve(&[ValueRef::Integer(1), ValueRef::Integer(2)], None).kind,
            ColumnKind::Int64
        );
        assert_eq!(
            resolve(&[ValueRef::Real(1.5)], None).kind,
            ColumnKind::Float64
        );
        assert_eq!(
            resolve(&[ValueRef::Text(b"a")], None).kind,
            ColumnKind::Utf8
        );
        assert_eq!(
            resolve(&[ValueRef::Blob(b"\x00\xff")], None).kind,
            ColumnKind::Binary
        );
    }

    #[test]
    fn une_colonne_qui_mele_entiers_et_flottants_tombe_sur_le_texte() {
        // Le piège : aucun `f64` ne tient tous les `i64`. Convertir
        // 9_007_199_254_740_993 en flottant le change en 9_007_199_254_740_992.
        let plan = resolve(&[ValueRef::Integer(1), ValueRef::Real(1.5)], None);
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.observed.is_mixed());
        assert_eq!(plan.observed.names(), "integer|real");
    }

    #[test]
    fn un_blob_impose_les_octets_a_toute_la_colonne() {
        for autre in [
            ValueRef::Integer(1),
            ValueRef::Real(1.0),
            ValueRef::Text(b"a"),
        ] {
            let plan = resolve(&[autre, ValueRef::Blob(b"\x00")], None);
            assert_eq!(
                plan.kind,
                ColumnKind::Binary,
                "seuls les octets contiennent tout"
            );
        }
    }

    #[test]
    fn un_texte_qui_n_est_pas_de_l_utf8_compte_pour_des_octets() {
        // Découvert ici plutôt qu'au milieu du flux : c'est tout l'intérêt de la
        // sonde.
        let plan = resolve(&[ValueRef::Text(b"\xff\xfe")], None);
        assert_eq!(plan.kind, ColumnKind::Binary);
    }

    #[test]
    fn une_colonne_entierement_nulle_retombe_sur_la_declaration() {
        let plan = resolve(&[ValueRef::Null, ValueRef::Null], Some("INTEGER"));
        assert_eq!(plan.kind, ColumnKind::Int64);
        assert!(
            !plan.inferred,
            "le type vient de la déclaration : rien n'est déduit"
        );
        assert!(plan.observed.is_empty());
    }

    #[test]
    fn sans_valeur_ni_declaration_le_texte_est_le_repli() {
        let plan = resolve(&[], None);
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.inferred, "rien ne l'a déclaré : c'est une déduction");
    }

    #[test]
    fn une_colonne_dont_le_type_ne_suit_pas_sa_declaration_se_declare_deduite() {
        // `CREATE TABLE t(n INTEGER)` puis `INSERT INTO t VALUES('abc')` :
        // SQLite l'accepte, et l'interface doit pouvoir dire que le type affiché
        // n'est pas celui du schéma.
        let plan = resolve(&[ValueRef::Text(b"abc")], Some("INTEGER"));
        assert_eq!(plan.kind, ColumnKind::Utf8);
        assert!(plan.inferred);

        let champ = plan.field();
        assert_eq!(
            champ.metadata().get(METADATA_INFERRED).map(String::as_str),
            Some("true")
        );
        assert_eq!(
            champ
                .metadata()
                .get(METADATA_DECLARED_TYPE)
                .map(String::as_str),
            Some("INTEGER")
        );
    }

    #[test]
    fn une_colonne_melangee_le_declare_dans_ses_metadonnees() {
        let plan = resolve(&[ValueRef::Integer(1), ValueRef::Text(b"a")], None);
        let champ = plan.field();
        assert_eq!(
            champ
                .metadata()
                .get(METADATA_STORAGE_CLASSES)
                .map(String::as_str),
            Some("integer|text"),
            "l'interface doit pouvoir montrer que le rendu est un repli"
        );
    }

    #[test]
    fn tout_champ_est_nullable() {
        // SQLite rend NULL pour n'importe quelle expression, et une colonne
        // NOT NULL vue à travers une jointure externe l'est tout autant.
        assert!(
            resolve(&[ValueRef::Integer(1)], Some("INTEGER"))
                .field()
                .is_nullable()
        );
    }

    #[test]
    fn les_conversions_acceptees_sont_sans_perte() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 4);
        b.append(ValueRef::Integer(-42)).expect("entier en texte");
        b.append(ValueRef::Real(0.1)).expect("flottant en texte");
        b.append(ValueRef::Text(b"caf\xc3\xa9")).expect("texte");
        b.append(ValueRef::Null).expect("nul");

        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<StringArray>()
            .expect("colonne Utf8");
        assert_eq!(colonne.value(0), "-42");
        assert_eq!(
            colonne.value(1),
            "0.1",
            "le Display de f64 rend la plus courte forme qui se relit à l'identique"
        );
        assert_eq!(colonne.value(2), "café");
        assert!(colonne.is_null(3));
    }

    #[test]
    fn un_entier_au_dela_de_deux_puissance_53_ne_passe_pas_par_un_flottant() {
        // La corruption silencieuse que ce refus évite : 2⁵³+1 deviendrait 2⁵³.
        let mut b = ColumnBuilder::new(ColumnKind::Float64, 2, 1);
        b.append(ValueRef::Integer(1 << 53))
            .expect("2^53 est exact");

        let err = b
            .append(ValueRef::Integer((1_i64 << 53) + 1))
            .expect_err("refus attendu");
        let SqliteError::ColumnConflict {
            column, resolved, ..
        } = err
        else {
            panic!("mauvaise variante : {err:?}");
        };
        assert_eq!((column, resolved), (2, "float64"));
    }

    #[test]
    fn un_blob_ne_se_glisse_pas_dans_une_colonne_textuelle() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 1, 1);
        let err = b
            .append(ValueRef::Blob(b"\x00\xff"))
            .expect_err("refus attendu");
        assert!(
            matches!(
                err,
                SqliteError::ColumnConflict {
                    found: "blob",
                    resolved: "utf8",
                    ..
                }
            ),
            "{err:?}"
        );
    }

    #[test]
    fn un_texte_invalide_ne_se_glisse_pas_dans_une_colonne_textuelle() {
        let mut b = ColumnBuilder::new(ColumnKind::Utf8, 0, 1);
        let err = b
            .append(ValueRef::Text(b"\xff"))
            .expect_err("refus attendu");
        assert!(matches!(err, SqliteError::ColumnConflict { .. }), "{err:?}");
    }

    #[test]
    fn un_texte_ne_se_glisse_pas_dans_une_colonne_entiere() {
        let mut b = ColumnBuilder::new(ColumnKind::Int64, 0, 1);
        assert!(b.append(ValueRef::Text(b"12")).is_err());
        assert!(b.append(ValueRef::Real(1.0)).is_err());
        assert!(b.append(ValueRef::Blob(b"x")).is_err());
        b.append(ValueRef::Integer(7)).expect("un entier passe");
        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<Int64Array>()
            .expect("colonne Int64");
        assert_eq!(colonne.value(0), 7);
    }

    #[test]
    fn une_colonne_d_octets_accueille_tout_sans_perdre_le_contenu() {
        let mut b = ColumnBuilder::new(ColumnKind::Binary, 0, 4);
        b.append(ValueRef::Blob(b"\x00\xff")).expect("octets");
        b.append(ValueRef::Text(b"abc")).expect("texte en octets");
        b.append(ValueRef::Integer(12)).expect("entier en octets");
        b.append(ValueRef::Null).expect("nul");

        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<BinaryArray>()
            .expect("colonne Binary");
        assert_eq!(colonne.value(0), b"\x00\xff");
        assert_eq!(colonne.value(1), b"abc");
        assert_eq!(colonne.value(2), b"12");
        assert!(colonne.is_null(3));
    }

    #[test]
    fn un_flottant_reste_un_flottant() {
        let mut b = ColumnBuilder::new(ColumnKind::Float64, 0, 2);
        b.append(ValueRef::Real(1.5)).expect("flottant");
        b.append(ValueRef::Integer(3)).expect("entier exact");
        let colonne = b.finish();
        let colonne = colonne
            .as_any()
            .downcast_ref::<Float64Array>()
            .expect("colonne Float64");
        assert!((colonne.value(0) - 1.5).abs() < f64::EPSILON);
        assert!((colonne.value(1) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn un_lot_se_mesure_en_octets_de_donnees() {
        assert_eq!(value_bytes(ValueRef::Null), 0);
        assert_eq!(value_bytes(ValueRef::Integer(1)), 8);
        assert_eq!(value_bytes(ValueRef::Blob(&[0; 1_048_576])), 1_048_576);
    }

    #[test]
    fn une_valeur_mise_de_cote_se_relit_a_l_identique() {
        for valeur in [
            ValueRef::Null,
            ValueRef::Integer(-1),
            ValueRef::Real(2.5),
            ValueRef::Text(b"\xff non-utf8"),
            ValueRef::Blob(b"\x00"),
        ] {
            let capturee = ProbeValue::capture(valeur);
            assert_eq!(capturee.borrow(), valeur);
        }
    }
}
