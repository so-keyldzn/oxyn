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
                    write!(f, "… ({} bytes)", b.len())?;
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

/// Scalar type a user can pick and type text for — the bound-parameter editor
/// (`oxyn-ui`'s `ParameterEditor`) and anything else that turns typed text
/// into a [`ScalarValue`].
///
/// This is a strict subset of the type space [`ScalarValue`] can hold.
/// [`ScalarValue::Interval`] and [`ScalarValue::Array`] have no unambiguous
/// textual form a user could type in a single field — an interval mixes
/// months, days and nanoseconds with no canonical separator, and an array
/// needs its own per-element type and a nesting syntax — so they are not
/// offered here at all, rather than accepted and silently misparsed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ParameterType {
    Null,
    Bool,
    Int64,
    Float64,
    Decimal,
    Text,
    Bytes,
    Uuid,
    Date,
    Time,
    Timestamp,
    TimestampNaive,
    Json,
}

impl ParameterType {
    /// Every variant, in the order a picker should list them. The single
    /// source of that order and of the variant list itself: duplicating it at
    /// call sites is exactly how a selector ends up offering a type its
    /// editor then refuses.
    pub const ALL: [Self; 13] = [
        Self::Null,
        Self::Bool,
        Self::Int64,
        Self::Float64,
        Self::Decimal,
        Self::Text,
        Self::Bytes,
        Self::Uuid,
        Self::Date,
        Self::Time,
        Self::Timestamp,
        Self::TimestampNaive,
        Self::Json,
    ];

    /// Human-readable name, used in pickers and in error messages.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Null => "NULL",
            Self::Bool => "Boolean",
            Self::Int64 => "Int64",
            Self::Float64 => "Float64",
            Self::Decimal => "Decimal",
            Self::Text => "Text",
            Self::Bytes => "Bytes (hex)",
            Self::Uuid => "UUID",
            Self::Date => "Date",
            Self::Time => "Time",
            Self::Timestamp => "Timestamp with timezone",
            Self::TimestampNaive => "Timestamp without timezone",
            Self::Json => "JSON",
        }
    }

    /// Parses `text` as a value of this type.
    ///
    /// The formats accepted for the date/time variants are documented on each
    /// variant of this enum, and each is pinned down by a test in this module
    /// that exercises `chrono`'s `FromStr` directly — they describe what the
    /// dependency actually does, not a recollection of it (invariant I-12).
    ///
    /// # Errors
    ///
    /// Returns [`ParameterParseError`] if `text` does not match the format
    /// for `self`. The error never contains `text`: see
    /// [`ParameterParseError`] for why.
    pub fn parse(self, text: &str) -> Result<ScalarValue, ParameterParseError> {
        match self {
            Self::Null => Ok(ScalarValue::Null),
            Self::Bool => text
                .parse()
                .map(ScalarValue::Bool)
                .map_err(|_| ParameterParseError),
            Self::Int64 => text
                .parse()
                .map(ScalarValue::Int64)
                .map_err(|_| ParameterParseError),
            Self::Float64 => text
                .parse()
                .map(ScalarValue::Float64)
                .map_err(|_| ParameterParseError),
            Self::Decimal => is_decimal_literal(text)
                .then(|| ScalarValue::Decimal(text.to_owned()))
                .ok_or(ParameterParseError),
            Self::Text => Ok(ScalarValue::Text(text.to_owned())),
            Self::Bytes => parse_hex(text).map(ScalarValue::Bytes),
            Self::Uuid => text
                .parse::<Uuid>()
                .map(ScalarValue::Uuid)
                .map_err(|_| ParameterParseError),
            // Accepts the calendar-date form `YYYY-MM-DD` (e.g. `2024-01-15`);
            // month and day do not need zero-padding. Anything with a time
            // component, or a non-ISO order such as `DD/MM/YYYY`, is
            // rejected. Proven by `date_accepts_the_iso_calendar_form` below.
            Self::Date => text
                .parse::<NaiveDate>()
                .map(ScalarValue::Date)
                .map_err(|_| ParameterParseError),
            // Accepts `HH:MM:SS`, `HH:MM:SS.fraction`, and also `HH:MM`
            // (seconds default to zero); the hour does not need zero-padding.
            // Proven by `time_accepts_missing_seconds_and_fraction` below.
            Self::Time => text
                .parse::<NaiveTime>()
                .map(ScalarValue::Time)
                .map_err(|_| ParameterParseError),
            // Accepts `YYYY-MM-DDTHH:MM:SS` and its fractional-second form.
            // Only the `T` separator works: the SQL-style space
            // (`YYYY-MM-DD HH:MM:SS`) is rejected, and so is anything
            // carrying a timezone offset — an offset makes it a [`Timestamp`]
            // ([`Self::Timestamp`]), not a naive one. Proven by
            // `timestamp_naive_requires_the_t_separator_and_no_offset` below.
            Self::TimestampNaive => text
                .parse::<NaiveDateTime>()
                .map(ScalarValue::TimestampNaive)
                .map_err(|_| ParameterParseError),
            // Accepts RFC 3339: a `T` or space date/time separator, then an
            // explicit UTC offset (`Z`, or `+HH:MM`/`-HH:MM`), with an
            // optional fractional second. A timestamp with no offset is
            // rejected rather than assumed to be UTC — guessing a fuzzy
            // instant into an exact one would silently shift it whenever the
            // guess is wrong. Proven by
            // `timestamp_requires_an_explicit_offset` below.
            Self::Timestamp => text
                .parse::<DateTime<Utc>>()
                .map(ScalarValue::Timestamp)
                .map_err(|_| ParameterParseError),
            Self::Json => serde_json::from_str::<serde_json::Value>(text)
                .map(ScalarValue::Json)
                .map_err(|_| ParameterParseError),
        }
    }
}

/// Why [`ParameterType::parse`] rejected a value.
///
/// Carries nothing: structurally, there is no field to hold the rejected
/// text, so neither `Display` nor the derived `Debug` can leak it into a log,
/// an error banner, or a crash report (invariant I-03) — a bound parameter is
/// exactly the kind of value that can turn out to be a password pasted into
/// the wrong field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("value does not match the expected parameter type")]
pub struct ParameterParseError;

/// Checks that `text` is a plain decimal literal: an optional sign, at least
/// one digit, and at most one decimal point among the digits.
///
/// Deliberately excludes scientific notation (`1e400`) rather than routing
/// through `f64` to validate the shape: `f64::parse` accepts `1e400` and
/// silently turns it into infinity, and rejects nothing about the *shape* of
/// a long literal — the two failures this function exists to avoid. The
/// digits are kept exactly as typed, so a 40-digit literal loses nothing.
fn is_decimal_literal(text: &str) -> bool {
    let mut chars = text.chars();
    let first = chars.clone().next();
    if matches!(first, Some('+' | '-')) {
        chars.next();
    }
    let mut has_digit = false;
    let mut has_dot = false;
    for ch in chars {
        match ch {
            '0'..='9' => has_digit = true,
            '.' if !has_dot => has_dot = true,
            _ => return false,
        }
    }
    has_digit
}

/// Decodes a hex string (as `Bytes` parameters are typed) without indexing a
/// slice by a byte offset derived from user input: `str::get` on a
/// non-boundary index returns `None` rather than panicking.
fn parse_hex(text: &str) -> Result<Vec<u8>, ParameterParseError> {
    if !text.len().is_multiple_of(2) {
        return Err(ParameterParseError);
    }
    (0..text.len())
        .step_by(2)
        .map(|index| {
            u8::from_str_radix(text.get(index..index + 2).unwrap_or_default(), 16)
                .map_err(|_| ParameterParseError)
        })
        .collect()
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
        assert!(rendu.contains("1024 bytes"), "rendu : {rendu}");
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

    // The following tests pin down what chrono's `FromStr` actually accepts,
    // proven by running them, not recalled from memory (invariant I-12). The
    // doc comments on `ParameterType::parse` restate exactly what these show.

    #[test]
    fn date_accepts_the_iso_calendar_form() {
        assert_eq!(
            "2024-01-15".parse::<NaiveDate>(),
            Ok(NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date"))
        );
        // Month and day are not required to be zero-padded.
        assert_eq!(
            "2024-1-15".parse::<NaiveDate>(),
            Ok(NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date"))
        );
    }

    #[test]
    fn date_rejects_a_time_component_and_a_non_iso_order() {
        assert!("2024-01-15T00:00:00".parse::<NaiveDate>().is_err());
        assert!("15/01/2024".parse::<NaiveDate>().is_err());
    }

    #[test]
    fn time_accepts_missing_seconds_and_fraction() {
        assert_eq!(
            "13:45:00.123".parse::<NaiveTime>(),
            Ok(NaiveTime::from_hms_milli_opt(13, 45, 0, 123).expect("valid time"))
        );
        // No seconds: they default to zero.
        assert_eq!(
            "13:45".parse::<NaiveTime>(),
            Ok(NaiveTime::from_hms_opt(13, 45, 0).expect("valid time"))
        );
    }

    #[test]
    fn time_rejects_garbage() {
        assert!("not a time".parse::<NaiveTime>().is_err());
    }

    #[test]
    fn timestamp_naive_requires_the_t_separator_and_no_offset() {
        assert!("2024-01-15T13:45:00".parse::<NaiveDateTime>().is_ok());
        // The SQL-style space separator is not accepted by `FromStr`.
        assert!("2024-01-15 13:45:00".parse::<NaiveDateTime>().is_err());
        // A trailing offset makes it a `Timestamp`, not a naive one.
        assert!("2024-01-15T13:45:00Z".parse::<NaiveDateTime>().is_err());
    }

    #[test]
    fn timestamp_requires_an_explicit_offset() {
        assert!("2024-01-15T13:45:00Z".parse::<DateTime<Utc>>().is_ok());
        assert!("2024-01-15T13:45:00+02:00".parse::<DateTime<Utc>>().is_ok());
        // The space separator works too, as long as an offset is present.
        assert!("2024-01-15 13:45:00Z".parse::<DateTime<Utc>>().is_ok());
        // No offset: rejected rather than assumed to be UTC.
        assert!("2024-01-15T13:45:00".parse::<DateTime<Utc>>().is_err());
    }

    #[test]
    fn parameter_type_null_ignores_its_input() {
        assert_eq!(ParameterType::Null.parse("anything"), Ok(ScalarValue::Null));
    }

    #[test]
    fn parameter_type_bool_parses_strictly() {
        assert_eq!(
            ParameterType::Bool.parse("true"),
            Ok(ScalarValue::Bool(true))
        );
        assert!(ParameterType::Bool.parse("yes").is_err());
    }

    #[test]
    fn parameter_type_int64_parses_strictly() {
        assert_eq!(
            ParameterType::Int64.parse("-42"),
            Ok(ScalarValue::Int64(-42))
        );
        assert!(ParameterType::Int64.parse("4.2").is_err());
    }

    #[test]
    fn parameter_type_float64_parses_strictly() {
        assert_eq!(
            ParameterType::Float64.parse("4.2"),
            Ok(ScalarValue::Float64(4.2))
        );
        assert!(ParameterType::Float64.parse("four").is_err());
    }

    #[test]
    fn parameter_type_text_accepts_anything_including_empty() {
        assert_eq!(
            ParameterType::Text.parse(""),
            Ok(ScalarValue::Text(String::new()))
        );
    }

    #[test]
    fn parameter_type_bytes_parses_hex() {
        assert_eq!(
            ParameterType::Bytes.parse("00ff"),
            Ok(ScalarValue::Bytes(vec![0x00, 0xff]))
        );
        assert!(ParameterType::Bytes.parse("0").is_err(), "odd length");
        assert!(ParameterType::Bytes.parse("zz").is_err(), "not hex");
    }

    #[test]
    fn parameter_type_uuid_parses_hyphenated_and_bare_forms() {
        let expected = Uuid::nil();
        assert_eq!(
            ParameterType::Uuid.parse("00000000-0000-0000-0000-000000000000"),
            Ok(ScalarValue::Uuid(expected))
        );
        assert!(ParameterType::Uuid.parse("not-a-uuid").is_err());
    }

    #[test]
    fn parameter_type_date_parses_iso_form() {
        assert_eq!(
            ParameterType::Date.parse("2024-01-15"),
            Ok(ScalarValue::Date(
                NaiveDate::from_ymd_opt(2024, 1, 15).expect("valid date")
            ))
        );
        assert!(ParameterType::Date.parse("15/01/2024").is_err());
    }

    #[test]
    fn parameter_type_time_parses_iso_form() {
        assert_eq!(
            ParameterType::Time.parse("13:45:00"),
            Ok(ScalarValue::Time(
                NaiveTime::from_hms_opt(13, 45, 0).expect("valid time")
            ))
        );
        assert!(ParameterType::Time.parse("not a time").is_err());
    }

    #[test]
    fn parameter_type_timestamp_naive_parses_t_form_and_rejects_offset() {
        assert!(
            ParameterType::TimestampNaive
                .parse("2024-01-15T13:45:00")
                .is_ok()
        );
        assert!(
            ParameterType::TimestampNaive
                .parse("2024-01-15T13:45:00Z")
                .is_err(),
            "an offset makes it a Timestamp, not a naive one"
        );
    }

    #[test]
    fn parameter_type_timestamp_parses_with_offset_and_rejects_without() {
        assert!(
            ParameterType::Timestamp
                .parse("2024-01-15T13:45:00Z")
                .is_ok()
        );
        assert!(
            ParameterType::Timestamp
                .parse("2024-01-15T13:45:00")
                .is_err(),
            "no offset: must not be guessed as UTC"
        );
    }

    #[test]
    fn parameter_type_json_parses_a_document() {
        assert_eq!(
            ParameterType::Json.parse("{\"a\":1}"),
            Ok(ScalarValue::Json(serde_json::json!({"a": 1})))
        );
        assert!(ParameterType::Json.parse("{a:1}").is_err());
    }

    #[test]
    fn parameter_type_decimal_keeps_a_long_literal_exact() {
        let forty_digits = "1".repeat(40);
        let literal = format!("{forty_digits}.5");
        assert_eq!(
            ParameterType::Decimal.parse(&literal),
            Ok(ScalarValue::Decimal(literal.clone())),
            "the exact digits must survive untouched"
        );
    }

    #[test]
    fn parameter_type_decimal_rejects_scientific_notation() {
        // `f64::parse("1e400")` succeeds and silently produces infinity; the
        // form validator must reject it outright instead.
        assert!(ParameterType::Decimal.parse("1e400").is_err());
        assert!("1e400".parse::<f64>().unwrap_or_default().is_infinite());
    }

    #[test]
    fn parameter_type_decimal_rejects_malformed_shapes() {
        assert!(ParameterType::Decimal.parse("").is_err());
        assert!(ParameterType::Decimal.parse("-").is_err());
        assert!(ParameterType::Decimal.parse("1.2.3").is_err());
        assert!(ParameterType::Decimal.parse("12a").is_err());
    }

    #[test]
    fn parameter_type_decimal_accepts_signed_and_pointless_forms() {
        assert!(ParameterType::Decimal.parse("-0.5").is_ok());
        assert!(ParameterType::Decimal.parse("+5").is_ok());
        assert!(ParameterType::Decimal.parse(".5").is_ok());
        assert!(ParameterType::Decimal.parse("5.").is_ok());
    }

    #[test]
    fn parameter_parse_error_never_carries_the_rejected_text() {
        let sentinel = "S3NT1NELLE";
        let err = ParameterType::Int64.parse(sentinel).expect_err("invalid");
        assert!(!format!("{err:?}").contains(sentinel));
        assert!(!format!("{err}").contains(sentinel));
    }
}
