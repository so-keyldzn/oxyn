//! Valeurs scalaires du domaine.
//!
//! [`ScalarValue`] n'est **pas** le modèle de résultat : les résultats sont des
//! `arrow::RecordBatch` (ADR-0002) et ne passent jamais par ce type. `ScalarValue`
//! sert aux valeurs isolées : paramètres liés d'une requête, cellule éditée,
//! valeur affichée dans un inspecteur.
//!
//! Le traitement du temps suit
//! [`DRIVER-CONTRACT` §7](../../../docs/DRIVER-CONTRACT.md) : un instant avec
//! fuseau se transporte en UTC ([`Timestamp`](ScalarValue::Timestamp)), un
//! horodatage sans fuseau se transporte **sans en inventer un**
//! ([`TimestampNaive`](ScalarValue::TimestampNaive)). Les deux ne se confondent
//! pas : c'est cette confusion qui décale une donnée de deux heures en base, de
//! façon invisible et permanente.
//!
//! Les décimales voyagent en texte ([`Decimal`](ScalarValue::Decimal)) : aucun
//! type flottant ne représente `0.1` exactement, et une valeur monétaire arrondie
//! au passage est une corruption silencieuse.

use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Une valeur scalaire, telle qu'elle traverse la frontière d'un driver.
///
/// L'énumération est **fermée**, contrairement à la convention du dépôt sur les
/// énumérations publiques. La raison est le contrat de driver : chaque driver
/// tient une table de correspondance de types **dans les deux sens**, et cette
/// table est un `match`. Ajouter un type scalaire doit faire échouer la
/// compilation de chaque driver, pour que la question « et celui-là, je le rends
/// comment ? » soit posée — plutôt que d'être absorbée par un `_ =>` qui
/// produirait une conversion silencieusement fausse.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum ScalarValue {
    /// Absence de valeur. Se distingue toujours de la chaîne vide et de zéro.
    Null,
    /// Booléen.
    Bool(bool),
    /// Entier signé 64 bits.
    Int64(i64),
    /// Flottant double précision.
    Float64(f64),
    /// Décimal exact, conservé en texte pour ne rien perdre.
    Decimal(String),
    /// Texte. L'encodage est déjà validé : ce qui arrive du serveur et n'est pas
    /// de l'UTF-8 valide devient [`Bytes`](Self::Bytes), pas un texte mutilé.
    Text(String),
    /// Suite d'octets opaque.
    Bytes(Vec<u8>),
    /// UUID.
    Uuid(Uuid),
    /// Date sans heure ni fuseau.
    Date(NaiveDate),
    /// Heure sans date ni fuseau.
    Time(NaiveTime),
    /// Instant absolu, transporté en UTC.
    Timestamp(DateTime<Utc>),
    /// Horodatage sans fuseau. Aucun fuseau ne lui est attribué à la lecture.
    TimestampNaive(NaiveDateTime),
    /// Intervalle, décomposé comme PostgreSQL le fait : les mois n'ont pas de
    /// durée fixe, les jours non plus dès qu'il y a un changement d'heure. Les
    /// aplatir en une seule durée serait faux.
    Interval {
        /// Nombre de mois.
        months: i32,
        /// Nombre de jours.
        days: i32,
        /// Reste, en nanosecondes.
        nanos: i64,
    },
    /// Document JSON.
    Json(serde_json::Value),
    /// Tableau homogène ou non, selon ce qu'accepte la source.
    Array(Vec<ScalarValue>),
}

impl ScalarValue {
    /// Nombre maximal d'octets rendus par [`fmt::Display`] pour
    /// [`Bytes`](Self::Bytes) avant troncature.
    const APERCU_OCTETS: usize = 32;

    /// Nom stable du type, utilisable dans un message ou une table de
    /// correspondance de types.
    ///
    /// Ces noms font partie de l'API : ils apparaissent dans les messages
    /// d'erreur de conversion et dans les tables de correspondance des drivers.
    #[must_use]
    pub const fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "bool",
            Self::Int64(_) => "int64",
            Self::Float64(_) => "float64",
            Self::Decimal(_) => "decimal",
            Self::Text(_) => "text",
            Self::Bytes(_) => "bytes",
            Self::Uuid(_) => "uuid",
            Self::Date(_) => "date",
            Self::Time(_) => "time",
            Self::Timestamp(_) => "timestamptz",
            Self::TimestampNaive(_) => "timestamp",
            Self::Interval { .. } => "interval",
            Self::Json(_) => "json",
            Self::Array(_) => "array",
        }
    }

    /// La valeur est-elle absente ?
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
}

impl fmt::Display for ScalarValue {
    /// Rendu lisible, destiné à l'affichage et aux tests.
    ///
    /// Ce n'est **pas** un littéral SQL : `Text` n'est pas mis entre
    /// apostrophes et rien n'est échappé. Composer du SQL à partir de ce rendu
    /// serait exactement l'erreur que
    /// [`DRIVER-CONTRACT` §6](../../../docs/DRIVER-CONTRACT.md) interdit ;
    /// les valeurs se **lient**, elles ne se concatènent pas.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => f.write_str("NULL"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int64(i) => write!(f, "{i}"),
            Self::Float64(x) => write!(f, "{x}"),
            Self::Decimal(d) => f.write_str(d),
            Self::Text(t) => f.write_str(t),
            Self::Bytes(b) => {
                f.write_str("\\x")?;
                for octet in b.iter().take(Self::APERCU_OCTETS) {
                    write!(f, "{octet:02x}")?;
                }
                if b.len() > Self::APERCU_OCTETS {
                    write!(f, "… ({} octets)", b.len())?;
                }
                Ok(())
            }
            Self::Uuid(u) => write!(f, "{u}"),
            Self::Date(d) => write!(f, "{d}"),
            Self::Time(t) => write!(f, "{t}"),
            // RFC 3339 en UTC : jamais converti dans le fuseau du poste.
            Self::Timestamp(ts) => write!(f, "{}", ts.to_rfc3339()),
            Self::TimestampNaive(ts) => write!(f, "{}", ts.format("%Y-%m-%dT%H:%M:%S%.f")),
            Self::Interval {
                months,
                days,
                nanos,
            } => {
                // Forme ISO 8601 : les trois composantes restent distinctes.
                let secondes = nanos / 1_000_000_000;
                let reste = (nanos % 1_000_000_000).unsigned_abs();
                write!(f, "P{months}M{days}DT{secondes}.{reste:09}S")
            }
            Self::Json(v) => write!(f, "{v}"),
            Self::Array(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{item}")?;
                }
                f.write_str("]")
            }
        }
    }
}

impl From<bool> for ScalarValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<i64> for ScalarValue {
    fn from(v: i64) -> Self {
        Self::Int64(v)
    }
}

impl From<f64> for ScalarValue {
    fn from(v: f64) -> Self {
        Self::Float64(v)
    }
}

impl From<String> for ScalarValue {
    fn from(v: String) -> Self {
        Self::Text(v)
    }
}

impl From<&str> for ScalarValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
}

impl From<Uuid> for ScalarValue {
    fn from(v: Uuid) -> Self {
        Self::Uuid(v)
    }
}

impl<T> From<Option<T>> for ScalarValue
where
    T: Into<ScalarValue>,
{
    fn from(v: Option<T>) -> Self {
        v.map_or(Self::Null, Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_se_rend_en_majuscules_et_se_distingue_du_vide() {
        assert_eq!(ScalarValue::Null.to_string(), "NULL");
        assert_eq!(ScalarValue::Text(String::new()).to_string(), "");
        // Le piège, énoncé comme tel : `Display` ne distingue pas `NULL` de la
        // chaîne « NULL ». C'est délibéré — `Display` rend une valeur, il ne
        // porte pas de typographie — et c'est pour cette raison que
        // `oxyn_ui::data_grid` rend `CellValue::Null` en italique et dans une
        // couleur propre. Quiconque écrirait une comparaison de valeurs sur ce
        // rendu prendrait l'un pour l'autre.
        assert_eq!(
            ScalarValue::Null.to_string(),
            ScalarValue::Text("NULL".into()).to_string(),
            "si ces deux rendus divergent un jour, la grille peut cesser de les \
             distinguer visuellement : c'est elle qui porte la distinction"
        );
    }

    #[test]
    fn les_noms_de_types_sont_stables() {
        assert_eq!(ScalarValue::Null.type_name(), "null");
        assert_eq!(ScalarValue::Int64(1).type_name(), "int64");
        assert_eq!(
            ScalarValue::Timestamp(DateTime::<Utc>::UNIX_EPOCH).type_name(),
            "timestamptz"
        );
        assert_eq!(
            ScalarValue::TimestampNaive(DateTime::<Utc>::UNIX_EPOCH.naive_utc()).type_name(),
            "timestamp",
            "un horodatage sans fuseau ne porte pas le même nom qu'un instant"
        );
    }

    #[test]
    fn un_instant_se_rend_en_utc() {
        let ts = DateTime::<Utc>::UNIX_EPOCH;
        let rendu = ScalarValue::Timestamp(ts).to_string();
        assert!(rendu.ends_with("+00:00"), "rendu inattendu : {rendu}");
        assert!(rendu.starts_with("1970-01-01T00:00:00"));
    }

    #[test]
    fn un_horodatage_sans_fuseau_n_en_invente_pas_un() {
        let naive = DateTime::<Utc>::UNIX_EPOCH.naive_utc();
        let rendu = ScalarValue::TimestampNaive(naive).to_string();
        assert!(
            !rendu.contains('+') && !rendu.ends_with('Z'),
            "un fuseau a été inventé : {rendu}"
        );
    }

    #[test]
    fn les_octets_sont_tronques_a_l_affichage() {
        let court = ScalarValue::Bytes(vec![0x00, 0x0a, 0xff]);
        assert_eq!(court.to_string(), "\\x000aff");

        let long = ScalarValue::Bytes(vec![0xab; 1024]);
        let rendu = long.to_string();
        assert!(rendu.contains("1024 octets"), "rendu : {rendu}");
        assert!(
            rendu.len() < 128,
            "un BLOB ne doit pas être rendu en entier"
        );
    }

    #[test]
    fn les_decimales_ne_passent_pas_par_un_flottant() {
        let d = ScalarValue::Decimal("0.10".into());
        assert_eq!(d.to_string(), "0.10", "les zéros de queue sont signifiants");
    }

    #[test]
    fn un_intervalle_garde_ses_trois_composantes() {
        let i = ScalarValue::Interval {
            months: 1,
            days: 2,
            nanos: 3_000_000_004,
        };
        assert_eq!(i.to_string(), "P1M2DT3.000000004S");
    }

    #[test]
    fn un_tableau_se_rend_imbrique() {
        let a = ScalarValue::Array(vec![
            ScalarValue::Int64(1),
            ScalarValue::Null,
            ScalarValue::Array(vec![ScalarValue::Text("x".into())]),
        ]);
        assert_eq!(a.to_string(), "[1, NULL, [x]]");
    }

    #[test]
    fn conversion_depuis_option() {
        let absent: ScalarValue = Option::<i64>::None.into();
        assert!(absent.is_null());
        let present: ScalarValue = Some(7_i64).into();
        assert_eq!(present, ScalarValue::Int64(7));
    }

    #[test]
    fn aller_retour_json() {
        let cas = [
            ScalarValue::Null,
            ScalarValue::Bool(true),
            ScalarValue::Int64(-42),
            ScalarValue::Decimal("1.000".into()),
            ScalarValue::Text("café".into()),
            ScalarValue::Bytes(vec![1, 2, 3]),
            ScalarValue::Uuid(Uuid::nil()),
            ScalarValue::Interval {
                months: -1,
                days: 0,
                nanos: 1,
            },
            ScalarValue::Array(vec![ScalarValue::Int64(1)]),
        ];
        for valeur in cas {
            let json = serde_json::to_string(&valeur).expect("sérialisation");
            let relu: ScalarValue = serde_json::from_str(&json).expect("désérialisation");
            assert_eq!(
                valeur,
                relu,
                "aller-retour raté pour {}",
                valeur.type_name()
            );
        }
    }
}
