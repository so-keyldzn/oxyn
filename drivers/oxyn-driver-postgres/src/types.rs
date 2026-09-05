//! La correspondance des types PostgreSQL vers Arrow, **et ses pertes**.
//!
//! Le contrat exige que cette table aille dans les deux sens et documente ce
//! qu'elle perd ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)). Le sens
//! « lecture » est ici ; le sens « écriture » — lier un
//! [`ScalarValue`](oxyn_core::ScalarValue) en paramètre — est dans
//! [`crate::session`], parce qu'il dépend de l'encodeur de `sqlx` et non du
//! décodeur.
//!
//! # Les trois règles qui gouvernent ce module
//!
//! **Un type inconnu ne fait jamais échouer la requête.** Il tombe sur
//! [`PgDecoding::Opaque`], qui rend `Utf8` : le texte du serveur si les octets
//! sont de l'UTF-8 valide — ce qui couvre les `enum`, `xml`, `citext`, `ltree`
//! et la plupart des types d'extension —, sinon une transcription hexadécimale.
//! Le **nom du type PostgreSQL est conservé** dans les métadonnées du champ
//! Arrow (`oxyn:pg_type`), avec le mode de repli employé (`oxyn:fallback`) :
//! l'interface peut donc dire « ceci est un `geometry` rendu en hexadécimal »
//! plutôt que de faire passer une chaîne pour une valeur.
//!
//! **`NUMERIC` ne devient jamais un flottant.** Un `NUMERIC` sans précision ne
//! tient dans aucun `f64` ; le convertir corrompt des montants. Il devient une
//! chaîne décimale **exacte**, reconstruite depuis le format binaire du serveur
//! (module interne `numeric`).
//!
//! **Un `timestamp` sans fuseau n'en reçoit pas un.** `timestamp` devient
//! `Timestamp(Microsecond, None)` et `timestamptz` devient
//! `Timestamp(Microsecond, Some("UTC"))`. Aucune conversion vers le fuseau du
//! poste n'a lieu ici, ni ailleurs dans le driver.
//!
//! # Ce que la table perd, explicitement
//!
//! | Type PostgreSQL | Arrow | Perte |
//! |---|---|---|
//! | `numeric` | `Utf8` | aucune sur la valeur ; la précision et l'échelle déclarées ne sont pas portées par le protocole (voir plus bas) |
//! | `uuid` | `Utf8` | aucune : rendu canonique en minuscules avec tirets |
//! | `timetz` | `Utf8` | aucune : heure et décalage rendus tels quels |
//! | `interval` | `Interval(MonthDayNano)` | au-delà de ±292 ans de composante microseconde, la conversion en nanosecondes déborde : le décodage **échoue** au lieu de tronquer |
//! | `money` | `Utf8` | l'unité monétaire dépend de `lc_monetary`, que le protocole ne transmet pas ; la valeur est rendue en unités de base |
//! | tableaux à plus d'une dimension | — | non représentables par une liste Arrow : le décodage **échoue** plutôt que d'aplatir en silence |
//! | `record`, types composites | `Utf8` (opaque) | la structure n'est pas éclatée en `Struct` Arrow |
//!
//! # Pourquoi `numeric` n'est pas un `Decimal128`
//!
//! `Decimal128(p, s)` exige une précision et une échelle **fixées avant la
//! première ligne**. Le `RowDescription` du protocole PostgreSQL porte bien un
//! `atttypmod`, mais `sqlx` 0.9 ne l'expose pas sur [`PgColumn`] — et un
//! `numeric` sans contrainte de colonne n'en a de toute façon pas. Choisir une
//! échelle au jugé arrondirait des montants ; `Utf8` exact ne perd rien.
//!
// TODO(phase 1) : basculer sur `Decimal128(p, s)` pour les colonnes dont le
// catalogue donne une précision et une échelle — le chemin existe déjà via
// `PostgresCatalog::describe_relation`, il manque le lien entre la colonne du
// résultat et la colonne de table (`PgColumn::relation_id`).

use std::collections::HashMap;
use std::sync::Arc;

use arrow::datatypes::{DataType, Field, IntervalUnit, Schema, SchemaRef, TimeUnit};
use sqlx::Column as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgColumn, PgTypeInfo, PgTypeKind};

/// Les OID des types intégrés de PostgreSQL.
///
/// Ceux du catalogue `pg_type` par défaut sont **stables** d'une version à
/// l'autre — contrairement aux OID attribués par `CREATE EXTENSION`, qui
/// dépendent de l'ordre d'installation. C'est pourquoi la reconnaissance se
/// fait par OID pour les types intégrés et par **nom** pour les autres.
pub(crate) mod oid {
    /// `bool`
    pub const BOOL: u32 = 16;
    /// `bytea`
    pub const BYTEA: u32 = 17;
    /// `"char"` (un octet, pas `char(n)`)
    pub const CHAR: u32 = 18;
    /// `name`
    pub const NAME: u32 = 19;
    /// `int8`
    pub const INT8: u32 = 20;
    /// `int2`
    pub const INT2: u32 = 21;
    /// `int4`
    pub const INT4: u32 = 23;
    /// `text`
    pub const TEXT: u32 = 25;
    /// `oid`
    pub const OID: u32 = 26;
    /// `json`
    pub const JSON: u32 = 114;
    /// `xml`
    pub const XML: u32 = 142;
    /// `float4`
    pub const FLOAT4: u32 = 700;
    /// `float8`
    pub const FLOAT8: u32 = 701;
    /// `unknown` : littéral non typé
    pub const UNKNOWN: u32 = 705;
    /// `money`
    pub const MONEY: u32 = 790;
    /// `macaddr`
    pub const MACADDR: u32 = 829;
    /// `inet`
    pub const INET: u32 = 869;
    /// `cidr`
    pub const CIDR: u32 = 650;
    /// `macaddr8`
    pub const MACADDR8: u32 = 774;
    /// `bpchar` (`char(n)`)
    pub const BPCHAR: u32 = 1042;
    /// `varchar`
    pub const VARCHAR: u32 = 1043;
    /// `date`
    pub const DATE: u32 = 1082;
    /// `time`
    pub const TIME: u32 = 1083;
    /// `timestamp`
    pub const TIMESTAMP: u32 = 1114;
    /// `timestamptz`
    pub const TIMESTAMPTZ: u32 = 1184;
    /// `interval`
    pub const INTERVAL: u32 = 1186;
    /// `timetz`
    pub const TIMETZ: u32 = 1266;
    /// `numeric`
    pub const NUMERIC: u32 = 1700;
    /// `uuid`
    pub const UUID: u32 = 2950;
    /// `jsonb`
    pub const JSONB: u32 = 3802;
    /// `void`
    pub const VOID: u32 = 2278;
}

/// Clé de métadonnée portant le nom du type PostgreSQL d'origine.
pub const META_PG_TYPE: &str = "oxyn:pg_type";
/// Clé de métadonnée portant le mode de repli employé pour un type inconnu.
///
/// Vaut `text` quand les octets étaient de l'UTF-8 valide et `hex` sinon. Absente
/// quand le type est reconnu.
pub const META_FALLBACK: &str = "oxyn:fallback";

/// Comment décoder une valeur PostgreSQL vers son tableau Arrow.
///
/// Une variante par **format de fil**, pas par type SQL : `text`, `varchar`,
/// `name` et `xml` partagent [`PgDecoding::Text`] parce que leur représentation
/// binaire est la même suite d'octets UTF-8.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum PgDecoding {
    /// `bool` : un octet.
    Bool,
    /// `int2`.
    Int16,
    /// `int4`.
    Int32,
    /// `int8`.
    Int64,
    /// `oid` : entier non signé sur 32 bits.
    UInt32,
    /// `float4`.
    Float32,
    /// `float8`.
    Float64,
    /// Types dont la représentation binaire est déjà de l'UTF-8 : `text`,
    /// `varchar`, `bpchar`, `name`, `"char"`, `xml`, `json`, `inet`…
    Text,
    /// `jsonb` : un octet de version (valant 1) puis de l'UTF-8.
    Jsonb,
    /// `numeric` : format binaire propre, rendu en décimal exact.
    Numeric,
    /// `uuid` : seize octets, rendus sous forme canonique.
    Uuid,
    /// `timetz` : microsecondes depuis minuit, puis décalage en secondes.
    TimeTz,
    /// `bytea` et tout ce qui reste des octets.
    Bytes,
    /// `date` : jours depuis le 1<sup>er</sup> janvier 2000.
    Date,
    /// `time` : microsecondes depuis minuit.
    Time,
    /// `timestamp` : microsecondes depuis le 1<sup>er</sup> janvier 2000, **sans
    /// fuseau**.
    Timestamp,
    /// `timestamptz` : idem, en UTC.
    TimestampTz,
    /// `interval` : microsecondes, jours, mois.
    Interval,
    /// Tableau à une dimension d'un type décodable.
    List(Box<PgDecoding>),
    /// Type inconnu du driver. Rendu en `Utf8` : texte si les octets sont de
    /// l'UTF-8 valide, hexadécimal sinon. Ne fait **jamais** échouer la requête.
    Opaque,
}

impl PgDecoding {
    /// Le type Arrow correspondant.
    ///
    /// Stable pour toute la durée d'un flux : c'est ce que
    /// [`Cursor::schema`](oxyn_driver::Cursor::schema) promet.
    #[must_use]
    pub fn arrow_type(&self) -> DataType {
        match self {
            Self::Bool => DataType::Boolean,
            Self::Int16 => DataType::Int16,
            Self::Int32 => DataType::Int32,
            Self::Int64 => DataType::Int64,
            Self::UInt32 => DataType::UInt32,
            Self::Float32 => DataType::Float32,
            Self::Float64 => DataType::Float64,
            Self::Text | Self::Jsonb | Self::Numeric | Self::Uuid | Self::TimeTz | Self::Opaque => {
                DataType::Utf8
            }
            Self::Bytes => DataType::Binary,
            Self::Date => DataType::Date32,
            Self::Time => DataType::Time64(TimeUnit::Microsecond),
            Self::Timestamp => DataType::Timestamp(TimeUnit::Microsecond, None),
            // Le fuseau porté par le type Arrow dit « ces microsecondes sont
            // comptées en UTC », pas « affiche-les en UTC » : le rendu reste une
            // décision de l'interface (DRIVER-CONTRACT §7).
            Self::TimestampTz => DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC"))),
            Self::Interval => DataType::Interval(IntervalUnit::MonthDayNano),
            Self::List(element) => {
                DataType::List(Arc::new(Field::new("item", element.arrow_type(), true)))
            }
        }
    }

    /// Le type est-il rendu par un repli faute d'être reconnu ?
    #[must_use]
    pub fn is_opaque(&self) -> bool {
        matches!(self, Self::Opaque)
    }
}

/// Choisit le décodage d'un type PostgreSQL.
///
/// L'ordre des tentatives n'est pas indifférent : l'OID d'abord, parce qu'il est
/// stable pour les types intégrés ; puis la *sorte* du type, pour suivre un
/// domaine jusqu'à son type de base et un tableau jusqu'à son élément ; puis le
/// nom, seul moyen de reconnaître un type d'extension dont l'OID varie d'une
/// installation à l'autre.
#[must_use]
pub fn decoding_for(ty: &PgTypeInfo) -> PgDecoding {
    if let Some(brut) = ty.oid() {
        if let Some(decodage) = decoding_for_oid(brut.0) {
            return decodage;
        }
    }

    match ty.kind() {
        // Un domaine est un type de base plus une contrainte ; la contrainte ne
        // change pas la représentation sur le fil.
        PgTypeKind::Domain(base) => decoding_for(base),
        PgTypeKind::Array(element) => PgDecoding::List(Box::new(decoding_for(element))),
        // La représentation binaire d'un `enum` est son étiquette en UTF-8.
        PgTypeKind::Enum(_) => PgDecoding::Text,
        _ => decoding_for_name(ty.name()),
    }
}

/// Décodage d'un type intégré, reconnu par son OID.
fn decoding_for_oid(brut: u32) -> Option<PgDecoding> {
    let decodage = match brut {
        oid::BOOL => PgDecoding::Bool,
        oid::INT2 => PgDecoding::Int16,
        oid::INT4 => PgDecoding::Int32,
        oid::INT8 => PgDecoding::Int64,
        oid::OID => PgDecoding::UInt32,
        oid::FLOAT4 => PgDecoding::Float32,
        oid::FLOAT8 => PgDecoding::Float64,
        oid::NUMERIC => PgDecoding::Numeric,
        oid::TEXT
        | oid::VARCHAR
        | oid::BPCHAR
        | oid::NAME
        | oid::CHAR
        | oid::XML
        | oid::JSON
        | oid::UNKNOWN
        | oid::INET
        | oid::CIDR
        | oid::MACADDR
        | oid::MACADDR8 => PgDecoding::Text,
        oid::JSONB => PgDecoding::Jsonb,
        oid::BYTEA => PgDecoding::Bytes,
        oid::UUID => PgDecoding::Uuid,
        oid::DATE => PgDecoding::Date,
        oid::TIME => PgDecoding::Time,
        oid::TIMETZ => PgDecoding::TimeTz,
        oid::TIMESTAMP => PgDecoding::Timestamp,
        oid::TIMESTAMPTZ => PgDecoding::TimestampTz,
        oid::INTERVAL => PgDecoding::Interval,
        // `money` est un entier sur 64 bits d'unités de base, mais l'unité
        // dépend de `lc_monetary`, que le protocole ne transmet pas. Le rendre
        // en `Int64` inviterait à additionner des euros et des yens.
        oid::MONEY => PgDecoding::Opaque,
        oid::VOID => PgDecoding::Opaque,
        _ => return None,
    };
    Some(decodage)
}

/// Décodage d'un type d'extension, reconnu par son nom.
///
/// La liste est courte à dessein : elle ne contient que des types dont la
/// représentation binaire est **connue et stable**. Tout le reste tombe sur
/// [`PgDecoding::Opaque`], ce qui n'est pas un échec mais le comportement
/// documenté.
fn decoding_for_name(name: &str) -> PgDecoding {
    match name.to_ascii_lowercase().as_str() {
        // `citext` est un `text` insensible à la casse : mêmes octets.
        "citext" => PgDecoding::Text,
        _ => PgDecoding::Opaque,
    }
}

/// Construit le schéma Arrow d'un résultat, et le plan de décodage qui va avec.
///
/// Les deux sont rendus ensemble parce qu'ils doivent rester alignés : un
/// décodage qui ne correspondrait pas au type déclaré ferait échouer
/// [`RecordBatch::try_new`](arrow::record_batch::RecordBatch::try_new) au
/// premier lot, c'est-à-dire au pire moment.
///
/// Chaque champ porte le nom du type PostgreSQL d'origine dans ses métadonnées
/// ([`META_PG_TYPE`]) : c'est ce qui permet à l'interface de distinguer un
/// `jsonb` d'un `text` alors que les deux sont des colonnes `Utf8`.
#[must_use]
pub fn schema_for(columns: &[PgColumn]) -> (SchemaRef, Vec<PgDecoding>) {
    let mut champs = Vec::with_capacity(columns.len());
    let mut decodages = Vec::with_capacity(columns.len());

    for colonne in columns {
        let type_info = colonne.type_info();
        let decodage = decoding_for(type_info);

        let mut metadonnees = HashMap::with_capacity(2);
        metadonnees.insert(META_PG_TYPE.to_owned(), type_info.name().to_owned());
        if decodage.is_opaque() {
            // Le mode exact (`text` ou `hex`) dépend de la valeur ; on annonce
            // ici que la colonne est un repli, pas lequel.
            metadonnees.insert(META_FALLBACK.to_owned(), "opaque".to_owned());
        }

        // Toute colonne est déclarée nullable : le serveur peut rendre `NULL`
        // dans une colonne `NOT NULL` dès qu'une jointure externe entre en jeu,
        // et un schéma qui l'interdirait ferait échouer la construction du lot.
        champs.push(
            Field::new(colonne.name(), decodage.arrow_type(), true).with_metadata(metadonnees),
        );
        decodages.push(decodage);
    }

    (Arc::new(Schema::new(champs)), decodages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_entiers_gardent_leur_largeur() {
        // Un `int8` promu en `Int32` tronquerait un identifiant de 5 milliards
        // en 705 032 704, sans erreur nulle part.
        assert_eq!(decoding_for_oid(oid::INT2), Some(PgDecoding::Int16));
        assert_eq!(decoding_for_oid(oid::INT4), Some(PgDecoding::Int32));
        assert_eq!(decoding_for_oid(oid::INT8), Some(PgDecoding::Int64));
        assert_eq!(PgDecoding::Int64.arrow_type(), DataType::Int64);
    }

    #[test]
    fn numeric_ne_devient_jamais_un_flottant() {
        // La perte serait silencieuse et porterait sur des montants.
        let decodage = decoding_for_oid(oid::NUMERIC).expect("numeric est un type intégré");
        assert_eq!(decodage, PgDecoding::Numeric);
        assert_eq!(decodage.arrow_type(), DataType::Utf8);
        assert_ne!(decodage.arrow_type(), DataType::Float64);
    }

    #[test]
    fn un_timestamp_sans_fuseau_n_en_recoit_pas_un() {
        // DRIVER-CONTRACT §7 : la corruption serait invisible et permanente.
        assert_eq!(
            PgDecoding::Timestamp.arrow_type(),
            DataType::Timestamp(TimeUnit::Microsecond, None)
        );
        assert_eq!(
            PgDecoding::TimestampTz.arrow_type(),
            DataType::Timestamp(TimeUnit::Microsecond, Some(Arc::from("UTC")))
        );
    }

    #[test]
    fn un_oid_inconnu_tombe_sur_un_repli_pas_sur_une_erreur() {
        // 16 000 est au-delà des types intégrés : c'est un type utilisateur.
        assert_eq!(decoding_for_oid(16_000), None);
        assert_eq!(decoding_for_name("geometry"), PgDecoding::Opaque);
        assert_eq!(PgDecoding::Opaque.arrow_type(), DataType::Utf8);
    }

    #[test]
    fn un_tableau_devient_une_liste_du_meme_element() {
        let liste = PgDecoding::List(Box::new(PgDecoding::Int32));
        assert_eq!(
            liste.arrow_type(),
            DataType::List(Arc::new(Field::new("item", DataType::Int32, true)))
        );
    }

    #[test]
    fn le_type_du_champ_liste_est_celui_que_construit_le_decodeur() {
        // Le nom `item` n'est pas décoratif : `RecordBatch::try_new` compare le
        // champ du schéma à celui du tableau construit, et rejette l'écart.
        let DataType::List(champ) = PgDecoding::List(Box::new(PgDecoding::Text)).arrow_type()
        else {
            panic!("une liste doit produire un DataType::List");
        };
        assert_eq!(champ.name(), "item");
        assert!(champ.is_nullable(), "un élément de tableau peut être NULL");
    }

    #[test]
    fn money_reste_opaque_faute_de_connaitre_son_unite() {
        // Le rendre en Int64 inviterait à sommer des montants de devises
        // différentes sans jamais afficher laquelle.
        assert_eq!(decoding_for_oid(oid::MONEY), Some(PgDecoding::Opaque));
    }
}
