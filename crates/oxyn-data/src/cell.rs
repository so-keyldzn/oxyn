//! Rendu d'une cellule, appelé une fois par cellule visible et par trame.
//!
//! # Ce qui gouverne ce fichier
//!
//! **`NULL` n'est pas une chaîne.** [`CellValue::Null`] est une variante, jamais
//! le texte `"NULL"` : une colonne texte contenant réellement la chaîne `NULL`
//! doit se distinguer d'une absence de valeur. C'est l'interface qui décide de
//! l'italique et du gris ([UX-SPEC](../../../docs/UX-SPEC.md)).
//!
//! **Un défaut de rendu ne se déguise pas en `NULL`.** Un type qu'on ne sait pas
//! afficher rend [`CellValue::Unrenderable`], pas une cellule vide : la seconde
//! est un mensonge silencieux sur des données réelles.
//!
//! **Le texte se prête, il ne se copie pas.** Une colonne `Utf8` — le cas le
//! plus fréquent de très loin — rend un [`Cow::Borrowed`] sur le tampon Arrow.
//! Aucune allocation entre le `RecordBatch` et l'écran.
//!
//! # Ce qui est délégué à Arrow, et pourquoi
//!
//! Les dates, heures, horodatages (fuseau compris), listes, structures,
//! dictionnaires et flottants passent par [`arrow::util::display::ArrayFormatter`].
//! Deux raisons, aucune n'est la paresse : la conversion d'un horodatage avec
//! fuseau est un travail de calendrier que `chrono` fait correctement et qui
//! n'est pas au contrat de dépendances de cette crate ; et surtout **ce que la
//! grille affiche doit être exactement ce que l'export CSV écrit**, or l'export
//! passe par ces mêmes formateurs. Deux implémentations divergeraient en
//! quelques mois, et l'utilisateur ne le découvrirait qu'en comparant un fichier
//! exporté à son écran.

use std::borrow::Cow;
use std::fmt::Write as _;

use arrow::array::{Array, AsArray};
use arrow::datatypes::{
    DataType, Decimal128Type, Int8Type, Int16Type, Int32Type, Int64Type, UInt8Type, UInt16Type,
    UInt32Type, UInt64Type,
};
use arrow::record_batch::RecordBatch;
use arrow::util::display::{ArrayFormatter, FormatOptions as ArrowFormatOptions};
use serde::{Deserialize, Serialize};

/// Longueur au-delà de laquelle une cellule est coupée à l'affichage.
///
/// Une cellule de grille fait quelques dizaines de caractères ; 512 laisse de la
/// marge pour une infobulle sans jamais rendre un document JSON de 4 Mo.
pub const DEFAULT_MAX_LEN: usize = 512;

/// Ce qu'une cellule donne à afficher.
///
/// Emprunte au `RecordBatch` quand c'est possible, d'où la durée de vie.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum CellValue<'a> {
    /// Absence de valeur. **Pas** la chaîne `"NULL"` : l'interface la dessine à
    /// sa façon, et une colonne texte peut contenir littéralement `NULL`.
    Null,
    /// La valeur, en entier.
    Text(Cow<'a, str>),
    /// La valeur, coupée pour l'affichage.
    Truncated {
        /// Le début de la valeur, coupé sur une frontière de caractère.
        text: Cow<'a, str>,
        /// Taille de la valeur complète, **en octets**.
        ///
        /// En octets et non en caractères parce que c'est une information en
        /// O(1) : compter les caractères d'un document de 10 Mo une fois par
        /// cellule et par trame coûterait le budget d'affichage entier.
        full_bytes: usize,
    },
    /// Le type de la colonne n'a pas pu être rendu.
    ///
    /// L'interface doit le montrer comme tel — un badge, un point
    /// d'interrogation — jamais comme une cellule vide.
    Unrenderable {
        /// Ce qui a échoué, en une ligne, sans valeur de la base.
        reason: Cow<'static, str>,
    },
}

impl CellValue<'_> {
    /// La cellule est-elle nulle ?
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// La valeur affichée est-elle coupée ?
    #[must_use]
    pub const fn is_truncated(&self) -> bool {
        matches!(self, Self::Truncated { .. })
    }

    /// Le texte à dessiner, s'il y en a un.
    #[must_use]
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Text(texte) | Self::Truncated { text: texte, .. } => Some(texte),
            Self::Null | Self::Unrenderable { .. } => None,
        }
    }

    /// Le texte à dessiner, avec le remplacement configuré pour `NULL`.
    #[must_use]
    pub fn display_with<'b>(&'b self, opts: &'b FormatOptions) -> &'b str {
        match self {
            Self::Text(texte) | Self::Truncated { text: texte, .. } => texte,
            Self::Null => &opts.null_text,
            Self::Unrenderable { reason } => reason,
        }
    }

    /// Détache la valeur du `RecordBatch` dont elle est issue.
    ///
    /// Alloue si la valeur était empruntée : à réserver aux cas où la cellule
    /// survit au lot — presse-papiers, contexte d'agent — jamais au rendu.
    #[must_use]
    pub fn into_owned(self) -> CellValue<'static> {
        match self {
            Self::Null => CellValue::Null,
            Self::Text(texte) => CellValue::Text(Cow::Owned(texte.into_owned())),
            Self::Truncated { text, full_bytes } => CellValue::Truncated {
                text: Cow::Owned(text.into_owned()),
                full_bytes,
            },
            Self::Unrenderable { reason } => CellValue::Unrenderable { reason },
        }
    }
}

/// Comment rendre une colonne binaire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[non_exhaustive]
pub enum BinaryDisplay {
    /// Hexadécimal minuscule, sans séparateur : `48656c6c6f`.
    #[default]
    Hex,
    /// Base64 standard avec remplissage : `SGVsbG8=`.
    Base64,
    /// Seulement la taille : `<5 B>`, `<1.2 MiB>`.
    ///
    /// C'est le bon défaut pour une colonne de photos : afficher 4 Mo
    /// d'hexadécimal ne renseigne personne et coûte une trame.
    Size,
}

/// Réglages de rendu d'une cellule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct FormatOptions {
    /// Caractères au-delà desquels la valeur est coupée. `0` = pas de coupe.
    pub max_len: usize,
    /// Ce que l'interface écrit à la place d'une valeur absente, quand elle
    /// choisit d'écrire quelque chose.
    pub null_text: Cow<'static, str>,
    /// Motif de formatage des horodatages, au sens de `chrono`.
    ///
    /// `None` = RFC 3339, qui est aussi ce qu'écrit l'export : garder le défaut
    /// est ce qui rend l'écran et le fichier comparables.
    pub timestamp_format: Option<Cow<'static, str>>,
    /// Rendu des colonnes binaires.
    pub binary_display: BinaryDisplay,
}

impl Default for FormatOptions {
    fn default() -> Self {
        Self {
            max_len: DEFAULT_MAX_LEN,
            null_text: Cow::Borrowed("NULL"),
            timestamp_format: None,
            binary_display: BinaryDisplay::default(),
        }
    }
}

impl FormatOptions {
    /// Change la longueur de coupe.
    #[must_use]
    pub fn with_max_len(mut self, max_len: usize) -> Self {
        self.max_len = max_len;
        self
    }

    /// Change le texte de remplacement des valeurs absentes.
    #[must_use]
    pub fn with_null_text(mut self, texte: impl Into<Cow<'static, str>>) -> Self {
        self.null_text = texte.into();
        self
    }

    /// Change le motif d'horodatage.
    #[must_use]
    pub fn with_timestamp_format(mut self, motif: impl Into<Option<Cow<'static, str>>>) -> Self {
        self.timestamp_format = motif.into();
        self
    }

    /// Change le rendu des colonnes binaires.
    #[must_use]
    pub fn with_binary_display(mut self, mode: BinaryDisplay) -> Self {
        self.binary_display = mode;
        self
    }

    /// Traduit ces réglages pour les formateurs d'Arrow.
    ///
    /// `with_display_error(false)` : on veut qu'une erreur de formatage remonte
    /// pour devenir [`CellValue::Unrenderable`], plutôt que d'être écrite dans la
    /// cellule là où l'utilisateur attend une valeur.
    ///
    /// `null_text` ne concerne ici que les valeurs absentes **imbriquées** —
    /// celles d'une liste ou d'une structure. Une cellule nulle, elle, n'atteint
    /// jamais ce chemin : elle rend [`CellValue::Null`].
    fn arrow(&self) -> ArrowFormatOptions<'_> {
        let motif = self.timestamp_format.as_deref();
        ArrowFormatOptions::new()
            .with_display_error(false)
            .with_null(&self.null_text)
            .with_timestamp_format(motif)
            .with_timestamp_tz_format(motif)
    }
}

/// Rend la cellule `(row, col)` de `batch`.
///
/// Ne panique jamais : un indice hors borne, un type inattendu ou un échec de
/// formatage rendent [`CellValue::Unrenderable`]
/// ([I-09](../../../CLAUDE.md#i-09)). Une valeur absente rend
/// [`CellValue::Null`], jamais du texte.
///
/// N'alloue pas pour les colonnes `Utf8`, `LargeUtf8` et `Utf8View`, qui sont
/// l'essentiel de ce qu'une grille affiche.
#[must_use]
pub fn format_cell<'a>(
    batch: &'a RecordBatch,
    row: usize,
    col: usize,
    opts: &FormatOptions,
) -> CellValue<'a> {
    let Some(colonne) = batch.columns().get(col) else {
        return unrenderable("column index out of range");
    };
    if row >= colonne.len() {
        return unrenderable("row index out of range");
    }
    if colonne.is_null(row) {
        return CellValue::Null;
    }
    format_value(&**colonne, row, opts)
}

/// Rend une valeur non nulle d'un tableau Arrow quelconque.
///
/// Séparée de [`format_cell`] pour que l'export et les tests puissent formater
/// une colonne sans construire un `RecordBatch`.
///
/// # Préconditions
///
/// `row < array.len()` et la valeur n'est pas nulle ; les deux sont vérifiés par
/// [`format_cell`]. Appelée directement, elle rend `Unrenderable` plutôt que de
/// paniquer.
#[must_use]
pub fn format_value<'a>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a> {
    if row >= array.len() {
        return unrenderable("row index out of range");
    }
    if array.is_null(row) {
        return CellValue::Null;
    }

    match array.data_type() {
        DataType::Null => CellValue::Null,

        DataType::Boolean => match array.as_boolean_opt() {
            Some(valeurs) => CellValue::Text(Cow::Borrowed(if valeurs.value(row) {
                "true"
            } else {
                "false"
            })),
            None => delegate(array, row, opts),
        },

        // Les entiers se formatent à la main : `Display` de Rust et le
        // formateur d'Arrow produisent la même chaîne, et on évite une
        // construction de formateur par cellule.
        DataType::Int8 => integer::<Int8Type>(array, row, opts),
        DataType::Int16 => integer::<Int16Type>(array, row, opts),
        DataType::Int32 => integer::<Int32Type>(array, row, opts),
        DataType::Int64 => integer::<Int64Type>(array, row, opts),
        DataType::UInt8 => integer::<UInt8Type>(array, row, opts),
        DataType::UInt16 => integer::<UInt16Type>(array, row, opts),
        DataType::UInt32 => integer::<UInt32Type>(array, row, opts),
        DataType::UInt64 => integer::<UInt64Type>(array, row, opts),

        DataType::Utf8 => match array.as_string_opt::<i32>() {
            Some(valeurs) => finish(Cow::Borrowed(valeurs.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },
        DataType::LargeUtf8 => match array.as_string_opt::<i64>() {
            Some(valeurs) => finish(Cow::Borrowed(valeurs.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },
        DataType::Utf8View => match array.as_string_view_opt() {
            Some(valeurs) => finish(Cow::Borrowed(valeurs.value(row)), opts.max_len),
            None => delegate(array, row, opts),
        },

        DataType::Binary => match array.as_binary_opt::<i32>() {
            Some(valeurs) => binary(valeurs.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::LargeBinary => match array.as_binary_opt::<i64>() {
            Some(valeurs) => binary(valeurs.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::BinaryView => match array.as_binary_view_opt() {
            Some(valeurs) => binary(valeurs.value(row), opts),
            None => delegate(array, row, opts),
        },
        DataType::FixedSizeBinary(_) => match array
            .as_any()
            .downcast_ref::<arrow::array::FixedSizeBinaryArray>()
        {
            Some(valeurs) => binary(valeurs.value(row), opts),
            None => delegate(array, row, opts),
        },

        // `value_as_string` place la virgule décimale d'après l'échelle de la
        // colonne. Formater l'entier sous-jacent afficherait 12345 pour 123,45.
        DataType::Decimal128(_, _) => match array.as_primitive_opt::<Decimal128Type>() {
            Some(valeurs) => finish(Cow::Owned(valeurs.value_as_string(row)), opts.max_len),
            None => delegate(array, row, opts),
        },

        // Dates, heures, horodatages, durées, intervalles, listes, structures,
        // cartes, dictionnaires, flottants : voir la note en tête de module.
        _ => delegate(array, row, opts),
    }
}

/// Rend un entier de largeur quelconque.
fn integer<'a, T>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a>
where
    T: arrow::datatypes::ArrowPrimitiveType,
    T::Native: std::fmt::Display,
{
    match array.as_primitive_opt::<T>() {
        Some(valeurs) => {
            let mut texte = String::new();
            if write!(texte, "{}", valeurs.value(row)).is_err() {
                return unrenderable("integer formatting failed");
            }
            finish(Cow::Owned(texte), opts.max_len)
        }
        None => delegate(array, row, opts),
    }
}

/// Passe la main aux formateurs d'Arrow.
fn delegate<'a>(array: &'a dyn Array, row: usize, opts: &FormatOptions) -> CellValue<'a> {
    let options = opts.arrow();
    let Ok(formateur) = ArrayFormatter::try_new(array, &options) else {
        return unrenderable("unsupported column type");
    };
    match formateur.value(row).try_to_string() {
        Ok(texte) => finish(Cow::Owned(texte), opts.max_len),
        Err(_) => unrenderable("value could not be formatted"),
    }
}

/// Rend une valeur binaire selon [`BinaryDisplay`].
///
/// N'encode que les octets qui seront montrés : convertir 4 Mo de BLOB en
/// hexadécimal pour en afficher 512 caractères coûterait la trame entière.
fn binary<'a>(octets: &[u8], opts: &FormatOptions) -> CellValue<'a> {
    if matches!(opts.binary_display, BinaryDisplay::Size) {
        return CellValue::Text(Cow::Owned(human_size(octets.len())));
    }

    // Un peu plus que la limite, pour que `finish` **constate** le dépassement
    // au lieu de le supposer.
    let utiles = match opts.binary_display {
        BinaryDisplay::Base64 if opts.max_len > 0 => {
            // Trois octets sources donnent quatre caractères ; rester sur un
            // multiple de trois évite d'émettre un remplissage `=` au milieu
            // d'une valeur qui continue.
            octets.len().min((opts.max_len / 4 + 1) * 3)
        }
        BinaryDisplay::Hex if opts.max_len > 0 => octets.len().min(opts.max_len / 2 + 1),
        _ => octets.len(),
    };

    let Some(debut) = octets.get(..utiles) else {
        return unrenderable("binary slice out of range");
    };

    let mut texte = String::with_capacity(utiles.saturating_mul(2));
    match opts.binary_display {
        BinaryDisplay::Base64 => push_base64(&mut texte, debut),
        _ => {
            for octet in debut {
                push_hex(&mut texte, *octet);
            }
        }
    }

    // `full_bytes` compte les octets de la valeur, pas les caractères de son
    // encodage : c'est la taille du BLOB que l'utilisateur veut connaître.
    let complet = octets.len();
    match finish(Cow::Owned(texte), opts.max_len) {
        CellValue::Truncated { text, .. } => CellValue::Truncated {
            text,
            full_bytes: complet,
        },
        CellValue::Text(text) if utiles < complet => CellValue::Truncated {
            text,
            full_bytes: complet,
        },
        autre => autre,
    }
}

const HEX: &[u8; 16] = b"0123456789abcdef";

fn push_hex(sortie: &mut String, octet: u8) {
    let haut = usize::from(octet >> 4);
    let bas = usize::from(octet & 0x0f);
    if let (Some(a), Some(b)) = (HEX.get(haut), HEX.get(bas)) {
        sortie.push(char::from(*a));
        sortie.push(char::from(*b));
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Base64 standard (RFC 4648), avec remplissage.
///
/// Écrite à la main : la seule alternative serait une dépendance directe pour
/// vingt lignes, ce que la politique de dépendances décourage
/// ([SECURITY](../../../docs/SECURITY.md#dépendances)).
fn push_base64(sortie: &mut String, octets: &[u8]) {
    for morceau in octets.chunks(3) {
        let a = morceau.first().copied().unwrap_or(0);
        let b = morceau.get(1).copied().unwrap_or(0);
        let c = morceau.get(2).copied().unwrap_or(0);
        let bloc = (u32::from(a) << 16) | (u32::from(b) << 8) | u32::from(c);

        // 1 octet source → 2 caractères puis « == » ; 2 octets → 3 puis « = ».
        let significatifs = morceau.len().saturating_add(1).min(4);
        for rang in 0..4_usize {
            if rang < significatifs {
                let index = usize::try_from((bloc >> (18 - rang * 6)) & 0x3f).unwrap_or(0);
                if let Some(caractere) = BASE64.get(index) {
                    sortie.push(char::from(*caractere));
                }
            } else {
                sortie.push('=');
            }
        }
    }
}

/// Taille lisible : `<5 B>`, `<1.2 KiB>`, `<3.3 MiB>`.
///
/// Calculée en entiers : `usize as f64` perd de la précision au-delà de 2^53 et
/// la règle du dépôt proscrit les conversions `as` silencieuses
/// ([rust.md](../../../.claude/rules/rust.md)).
fn human_size(octets: usize) -> String {
    const UNITES: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut valeur = octets;
    let mut reste = 0_usize;
    let mut rang = 0_usize;
    while valeur >= 1024 && rang + 1 < UNITES.len() {
        reste = valeur % 1024;
        valeur /= 1024;
        rang += 1;
    }
    let unite = UNITES.get(rang).copied().unwrap_or("B");
    if rang == 0 {
        format!("<{valeur} {unite}>")
    } else {
        let dixiemes = reste.saturating_mul(10) / 1024;
        format!("<{valeur}.{dixiemes} {unite}>")
    }
}

/// Coupe la valeur si elle dépasse `max_len` caractères.
fn finish(texte: Cow<'_, str>, max_len: usize) -> CellValue<'_> {
    // Chemin rapide en O(1) : moins d'octets que de caractères autorisés, donc
    // à plus forte raison moins de caractères.
    if max_len == 0 || texte.len() <= max_len {
        return CellValue::Text(texte);
    }
    let Some((coupe, _)) = texte.char_indices().nth(max_len) else {
        return CellValue::Text(texte);
    };
    let full_bytes = texte.len();
    match texte {
        Cow::Borrowed(valeur) => match valeur.get(..coupe) {
            Some(debut) => CellValue::Truncated {
                text: Cow::Borrowed(debut),
                full_bytes,
            },
            None => unrenderable("truncation landed inside a character"),
        },
        Cow::Owned(mut valeur) => {
            valeur.truncate(coupe);
            CellValue::Truncated {
                text: Cow::Owned(valeur),
                full_bytes,
            }
        }
    }
}

const fn unrenderable<'a>(raison: &'static str) -> CellValue<'a> {
    CellValue::Unrenderable {
        reason: Cow::Borrowed(raison),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{
        BinaryArray, BooleanArray, Date32Array, Decimal128Array, Float64Array, Int32Array,
        Int64Array, ListArray, StringArray, StructArray, Time64MicrosecondArray,
        TimestampMillisecondArray, UInt8Array,
    };
    use arrow::datatypes::{Field, Fields, Schema, TimeUnit};

    use super::*;

    fn rendu(array: Arc<dyn Array>, opts: &FormatOptions) -> CellValue<'static> {
        // `into_owned` détache la valeur du tableau, qui meurt à la fin de
        // l'appel : c'est le prix d'un helper de test, pas du chemin de rendu.
        format_value(array.as_ref(), 0, opts).into_owned()
    }

    fn texte(array: Arc<dyn Array>) -> String {
        let opts = FormatOptions::default();
        rendu(array, &opts)
            .text()
            .map(str::to_owned)
            .unwrap_or_else(|| "<non rendu>".to_owned())
    }

    #[test]
    fn les_booleens_ne_sont_pas_alloues() {
        let array = BooleanArray::from(vec![Some(true), Some(false)]);
        let opts = FormatOptions::default();
        assert_eq!(
            format_value(&array, 0, &opts),
            CellValue::Text(Cow::Borrowed("true"))
        );
        assert_eq!(
            format_value(&array, 1, &opts),
            CellValue::Text(Cow::Borrowed("false"))
        );
    }

    #[test]
    fn les_entiers_de_toutes_largeurs_se_rendent() {
        assert_eq!(texte(Arc::new(Int32Array::from(vec![-42]))), "-42");
        assert_eq!(
            texte(Arc::new(Int64Array::from(vec![i64::MIN]))),
            i64::MIN.to_string()
        );
        assert_eq!(texte(Arc::new(UInt8Array::from(vec![255_u8]))), "255");
    }

    #[test]
    fn le_texte_est_emprunte_et_non_copie() {
        let array = StringArray::from(vec![Some("bonjour")]);
        let opts = FormatOptions::default();
        match format_value(&array, 0, &opts) {
            CellValue::Text(Cow::Borrowed(valeur)) => assert_eq!(valeur, "bonjour"),
            autre => panic!("attendu un emprunt, obtenu {autre:?}"),
        }
    }

    #[test]
    fn une_absence_de_valeur_est_null_pas_du_texte() {
        let array = StringArray::from(vec![None::<&str>]);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 0, &opts), CellValue::Null);
    }

    /// Le piège : une colonne texte qui contient réellement `NULL`.
    #[test]
    fn la_chaine_null_n_est_pas_une_absence_de_valeur() {
        let array = StringArray::from(vec![Some("NULL")]);
        let opts = FormatOptions::default();
        let valeur = format_value(&array, 0, &opts);
        assert!(!valeur.is_null());
        assert_eq!(valeur.text(), Some("NULL"));
    }

    #[test]
    fn une_valeur_trop_longue_est_coupee_sur_une_frontiere_de_caractere() {
        // 10 caractères, 20 octets : la coupe naïve à l'octet 5 casserait l'UTF-8.
        let array = StringArray::from(vec![Some("éàéàéàéàéà")]);
        let opts = FormatOptions::default().with_max_len(5);
        match format_value(&array, 0, &opts) {
            CellValue::Truncated { text, full_bytes } => {
                assert_eq!(text, "éàéàé");
                assert_eq!(full_bytes, 20);
            }
            autre => panic!("attendu Truncated, obtenu {autre:?}"),
        }
    }

    #[test]
    fn une_coupe_desactivee_laisse_la_valeur_entiere() {
        let long = "x".repeat(10_000);
        let array = StringArray::from(vec![Some(long.as_str())]);
        let opts = FormatOptions::default().with_max_len(0);
        let valeur = format_value(&array, 0, &opts);
        assert!(!valeur.is_truncated());
        assert_eq!(valeur.text().map(str::len), Some(10_000));
    }

    #[test]
    fn le_binaire_se_rend_en_hexadecimal() {
        let array = BinaryArray::from(vec![Some(b"Hello".as_slice())]);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 0, &opts).text(), Some("48656c6c6f"));
    }

    #[test]
    fn le_binaire_se_rend_en_base64() {
        let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Base64);
        // Les trois longueurs de reste, qui sont les trois cas de remplissage.
        for (entree, attendu) in [
            (b"Hello".as_slice(), "SGVsbG8="),
            (b"Hell".as_slice(), "SGVsbA=="),
            (b"Hel".as_slice(), "SGVs"),
        ] {
            let array = BinaryArray::from(vec![Some(entree)]);
            assert_eq!(
                format_value(&array, 0, &opts).text(),
                Some(attendu),
                "entrée de {} octets",
                entree.len()
            );
        }
    }

    #[test]
    fn un_gros_binaire_se_rend_en_taille() {
        let gros = vec![0_u8; 3_500_000];
        let array = BinaryArray::from(vec![Some(gros.as_slice())]);
        let opts = FormatOptions::default().with_binary_display(BinaryDisplay::Size);
        assert_eq!(format_value(&array, 0, &opts).text(), Some("<3.3 MiB>"));
    }

    /// Le point qui coûte une trame s'il est raté : on ne convertit pas
    /// 4 Mo de BLOB pour en afficher 32 caractères.
    #[test]
    fn un_gros_binaire_hexadecimal_n_encode_que_ce_qui_est_montre() {
        let gros = vec![0xab_u8; 1_000_000];
        let array = BinaryArray::from(vec![Some(gros.as_slice())]);
        let opts = FormatOptions::default().with_max_len(32);
        match format_value(&array, 0, &opts) {
            CellValue::Truncated { text, full_bytes } => {
                assert!(text.len() <= 40, "{} caractères produits", text.len());
                assert_eq!(full_bytes, 1_000_000);
            }
            autre => panic!("attendu Truncated, obtenu {autre:?}"),
        }
    }

    #[test]
    fn un_decimal_porte_sa_virgule() {
        let array = Decimal128Array::from(vec![Some(12_345_i128)])
            .with_precision_and_scale(10, 2)
            .expect("précision et échelle valides pour 12345");
        assert_eq!(texte(Arc::new(array)), "123.45");
    }

    #[test]
    fn les_flottants_suivent_la_convention_arrow() {
        // `ryu`, comme l'export : 1.0 et non 1.
        assert_eq!(texte(Arc::new(Float64Array::from(vec![1.0_f64]))), "1.0");
        assert_eq!(texte(Arc::new(Float64Array::from(vec![0.5_f64]))), "0.5");
    }

    #[test]
    fn les_dates_et_heures_se_rendent() {
        // 2021-01-01 = 18 628 jours après l'époque.
        assert_eq!(
            texte(Arc::new(Date32Array::from(vec![18_628]))),
            "2021-01-01"
        );
        let heure = Time64MicrosecondArray::from(vec![3_661_000_000_i64]);
        let rendu = texte(Arc::new(heure));
        assert!(rendu.starts_with("01:01:01"), "{rendu}");
    }

    /// Le fuseau n'est pas décoratif : la même valeur affichée sans décalage
    /// donne une heure fausse d'autant, et personne ne s'en aperçoit.
    #[test]
    fn un_horodatage_avec_fuseau_porte_son_decalage() {
        let array =
            TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]).with_timezone("+02:00");
        let rendu = texte(Arc::new(array));
        assert!(rendu.starts_with("2021-01-01T02:00:00"), "{rendu}");
        assert!(rendu.contains("+02:00"), "{rendu}");
    }

    #[test]
    fn un_motif_d_horodatage_personnalise_est_respecte() {
        let array = TimestampMillisecondArray::from(vec![1_609_459_200_000_i64]);
        let opts = FormatOptions::default().with_timestamp_format(Some(Cow::Borrowed("%Y/%m/%d")));
        assert_eq!(format_value(&array, 0, &opts).text(), Some("2021/01/01"));
    }

    #[test]
    fn une_liste_se_rend() {
        let array = ListArray::from_iter_primitive::<arrow::datatypes::Int32Type, _, _>(vec![
            Some(vec![Some(1), Some(2), None]),
        ]);
        // La forme exacte appartient à Arrow ; ce qui est vérifié ici, c'est que
        // la colonne imbriquée est rendue plutôt que déclarée irrécupérable.
        let rendu = texte(Arc::new(array));
        assert!(rendu.starts_with('['), "{rendu}");
        assert!(rendu.contains('1') && rendu.contains('2'), "{rendu}");
        assert!(rendu.ends_with(']'), "{rendu}");
    }

    #[test]
    fn une_structure_se_rend() {
        let champs = Fields::from(vec![
            Field::new("a", DataType::Int32, false),
            Field::new("b", DataType::Utf8, false),
        ]);
        let array = StructArray::new(
            champs,
            vec![
                Arc::new(Int32Array::from(vec![7])),
                Arc::new(StringArray::from(vec!["sept"])),
            ],
            None,
        );
        let rendu = texte(Arc::new(array));
        assert!(rendu.contains('7') && rendu.contains("sept"), "{rendu}");
    }

    #[test]
    fn une_colonne_entierement_nulle_se_rend_null() {
        let array = arrow::array::NullArray::new(3);
        let opts = FormatOptions::default();
        assert_eq!(format_value(&array, 1, &opts), CellValue::Null);
    }

    /// Un indice hors borne est un bug d'appelant, jamais une panique
    /// ([I-09](../../../CLAUDE.md#i-09)).
    #[test]
    fn un_indice_hors_borne_ne_panique_pas() {
        let schema = Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]));
        let batch = RecordBatch::try_new(schema, vec![Arc::new(Int32Array::from(vec![1, 2]))])
            .expect("lot construit pour le test");
        let opts = FormatOptions::default();

        assert!(matches!(
            format_cell(&batch, 99, 0, &opts),
            CellValue::Unrenderable { .. }
        ));
        assert!(matches!(
            format_cell(&batch, 0, 99, &opts),
            CellValue::Unrenderable { .. }
        ));
    }

    #[test]
    fn format_cell_traverse_le_lot() {
        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("nom", DataType::Utf8, true),
            Field::new(
                "quand",
                DataType::Timestamp(TimeUnit::Millisecond, None),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(Int32Array::from(vec![1, 2])),
                Arc::new(StringArray::from(vec![Some("un"), None])),
                Arc::new(TimestampMillisecondArray::from(vec![Some(0), None])),
            ],
        )
        .expect("lot construit pour le test");
        let opts = FormatOptions::default();

        assert_eq!(format_cell(&batch, 0, 0, &opts).text(), Some("1"));
        assert_eq!(format_cell(&batch, 0, 1, &opts).text(), Some("un"));
        assert!(format_cell(&batch, 1, 1, &opts).is_null());
        assert!(format_cell(&batch, 1, 2, &opts).is_null());
        assert_eq!(
            format_cell(&batch, 0, 2, &opts).text(),
            Some("1970-01-01T00:00:00")
        );
    }

    #[test]
    fn le_texte_de_remplacement_des_nulls_est_configurable() {
        let opts = FormatOptions::default().with_null_text("(vide)");
        assert_eq!(CellValue::Null.display_with(&opts), "(vide)");
    }
}
