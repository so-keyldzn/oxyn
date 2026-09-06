//! Du format binaire de PostgreSQL vers un `RecordBatch` Arrow.
//!
//! C'est le chemin **par ligne et par valeur** du produit : ce qui est écrit ici
//! s'exécute une fois par cellule d'un résultat de dix millions de lignes. Une
//! allocation par valeur y est un défaut de conception, pas une optimisation à
//! remettre à plus tard ([rust.md](../../../.claude/rules/rust.md)).
//!
//! # Les trois règles que ce module tient
//!
//! **Rien ne panique.** Les octets viennent du réseau. Aucun `unwrap`, aucune
//! indexation de tranche, aucun `as` sur un entier qui peut déborder ; un tampon
//! trop court rend une erreur nommée ([I-09](../../../CLAUDE.md#i-09)). Le
//! lecteur du module `numeric` existe exactement pour cela : `bytes::Buf`
//! panique sur un tampon court.
//!
//! **Aucune valeur n'entre dans un message d'erreur.** Les erreurs nomment le
//! rang de la colonne, le type PostgreSQL — assaini — et ce qui n'allait pas.
//! Jamais la donnée ([I-03](../../../CLAUDE.md#i-03)).
//!
//! **Un type inconnu ne fait pas échouer la requête.** Il devient du texte s'il
//! est de l'UTF-8 valide, une transcription hexadécimale sinon. Les seules
//! valeurs qui font vraiment échouer une ligne sont celles qu'Arrow ne peut pas
//! représenter *sans mentir* : une date infinie, un intervalle de plus de
//! 292 ans en microsecondes, un tableau à plusieurs dimensions. Les rendre
//! `NULL` serait un mensonge silencieux sur des données réelles.
//!
//! # Le dimensionnement des lots
//!
//! [`BatchAssembler::bytes`] compte les **octets de données** vus sur le fil, pas
//! les lignes. Mille lignes portant chacune un BLOB d'un mégaoctet font un
//! gigaoctet : un lot dimensionné en nombre de lignes marche sur les tables de
//! démonstration et déclenche l'OOM sur les vraies.

use std::fmt::Write as _;
use std::sync::Arc;

use arrow::array::{
    ArrayRef, BinaryBuilder, BooleanBuilder, Date32Builder, Float32Builder, Float64Builder,
    Int16Builder, Int32Builder, Int64Builder, IntervalMonthDayNanoBuilder, ListArray,
    StringBuilder, Time64MicrosecondBuilder, TimestampMicrosecondBuilder, UInt32Builder,
};
use arrow::buffer::{NullBuffer, OffsetBuffer, ScalarBuffer};
use arrow::datatypes::{Field, FieldRef, IntervalMonthDayNano, SchemaRef};
use arrow::record_batch::{RecordBatch, RecordBatchOptions};
use sqlx::Column as _;
use sqlx::Row as _;
use sqlx::TypeInfo as _;
use sqlx::postgres::{PgRow, PgValueFormat};

use crate::numeric::{Lecteur, render_binary};
use crate::types::PgDecoding;

/// Microsecondes entre l'époque Unix et celle de PostgreSQL (2000-01-01).
const EPOCH_SHIFT_MICROS: i64 = 946_684_800_000_000;
/// Jours entre l'époque Unix et celle de PostgreSQL.
const EPOCH_SHIFT_DAYS: i32 = 10_957;
/// Octets d'un `uuid` sur le fil.
const UUID_LEN: usize = 16;
/// Version attendue en tête d'un `jsonb`.
const JSONB_VERSION: u8 = 1;

/// Octets bruts au-delà desquels un type opaque n'est plus transcrit en entier.
///
/// Quatre kilo-octets font huit mille caractères hexadécimaux, soit seize fois
/// ce que la grille affiche. Au-delà, la transcription coûterait deux fois la
/// taille du BLOB en mémoire sans rien montrer de plus ; la coupe est **dite**
/// dans la valeur rendue, jamais silencieuse.
const OPAQUE_HEX_BUDGET: usize = 4_096;

/// Ce qui peut mal tourner en décodant une réponse du serveur.
///
/// Aucune variante ne porte de valeur venue de la base : seulement un rang de
/// colonne, un nom de type assaini, et un détail `&'static str`.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DecodeError {
    /// Une valeur n'a pas pu être représentée.
    #[error("colonne {ordinal} de type `{pg_type}` : {detail}")]
    Value {
        /// Rang de la colonne dans le résultat, à partir de zéro.
        ordinal: usize,
        /// Le type PostgreSQL, assaini.
        pg_type: String,
        /// Ce qui n'allait pas, sans reprendre la valeur.
        detail: &'static str,
    },

    /// La ligne n'a pas pu être lue : rang hors bornes, métadonnées absentes.
    #[error("ligne illisible : {detail}")]
    Row {
        /// Ce qui n'allait pas.
        detail: String,
    },

    /// Arrow a refusé le lot construit. C'est un bug du driver : le schéma et
    /// les constructeurs de colonnes ont divergé.
    #[error("lot Arrow invalide : {0}")]
    Arrow(#[from] arrow::error::ArrowError),
}

/// Rend un nom de type montrable dans un message d'erreur.
///
/// Un nom de type vient du serveur — donc d'une entrée non fiable : rien
/// n'interdit à un type utilisateur de s'appeler avec des séquences de contrôle
/// de terminal. Ce qui sort d'ici est borné et restreint à un alphabet
/// inoffensif.
fn sanitize_type(name: &str) -> String {
    const MAX: usize = 64;
    let acceptable = !name.is_empty()
        && name.len() <= MAX
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '[' | ']' | '"'));
    if acceptable {
        name.to_owned()
    } else {
        "<type non représentable>".to_owned()
    }
}

/// Accumule des lignes PostgreSQL en lots Arrow.
///
/// Réutilisable : [`BatchAssembler::finish`] vide les constructeurs et rend le
/// lot, l'assembleur repart pour le suivant. C'est ce qui permet à un flux de ne
/// jamais matérialiser plus d'un lot à la fois
/// ([I-06](../../../CLAUDE.md#i-06)).
#[derive(Debug)]
pub struct BatchAssembler {
    schema: SchemaRef,
    columns: Vec<ColumnBuilder>,
    rows: usize,
    bytes: usize,
}

impl BatchAssembler {
    /// Prépare un assembleur pour ce schéma et ce plan de décodage.
    ///
    /// Les deux viennent de [`crate::types::schema_for`] et sont alignés par
    /// construction ; une divergence serait un bug, et [`Self::finish`] la
    /// signalerait comme telle.
    #[must_use]
    pub fn new(schema: SchemaRef, decodings: &[PgDecoding]) -> Self {
        let columns = decodings.iter().map(ColumnBuilder::for_decoding).collect();
        Self {
            schema,
            columns,
            rows: 0,
            bytes: 0,
        }
    }

    /// Lignes accumulées depuis le dernier lot.
    #[must_use]
    pub const fn rows(&self) -> usize {
        self.rows
    }

    /// Octets de données accumulés depuis le dernier lot.
    ///
    /// C'est la mesure qui doit décider de la coupe d'un lot — pas [`Self::rows`].
    #[must_use]
    pub const fn bytes(&self) -> usize {
        self.bytes
    }

    /// Rien n'a été accumulé depuis le dernier lot.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.rows == 0
    }

    /// Ajoute une ligne.
    ///
    /// # Erreurs
    /// [`DecodeError::Row`] si la ligne n'a pas le nombre de colonnes attendu,
    /// [`DecodeError::Value`] si une valeur n'est pas représentable.
    pub fn push(&mut self, row: &PgRow) -> Result<(), DecodeError> {
        if row.columns().len() != self.columns.len() {
            return Err(DecodeError::Row {
                detail: format!(
                    "{} colonnes reçues, {} annoncées par le schéma",
                    row.columns().len(),
                    self.columns.len()
                ),
            });
        }

        for (ordinal, colonne) in self.columns.iter_mut().enumerate() {
            let brute = row.try_get_raw(ordinal).map_err(|err| DecodeError::Row {
                detail: err.to_string(),
            })?;
            let format = brute.format();
            // `as_bytes` rend une erreur sur une valeur absente ; c'est ainsi
            // que `sqlx` distingue NULL, et non par une variante.
            let octets = brute.as_bytes().ok();
            self.bytes = self.bytes.saturating_add(octets.map_or(0, <[u8]>::len));

            // Le nom du type n'est composé que sur le chemin d'erreur : ici, une
            // allocation par cellule coûterait le budget d'un résultat entier.
            if let Err(detail) = colonne.append(format, octets) {
                return Err(DecodeError::Value {
                    ordinal,
                    pg_type: sanitize_type(
                        row.columns()
                            .get(ordinal)
                            .map_or("unknown", |c| c.type_info().name()),
                    ),
                    detail,
                });
            }
        }

        self.rows = self.rows.saturating_add(1);
        Ok(())
    }

    /// Clôt le lot courant et repart à vide.
    ///
    /// # Erreurs
    /// [`DecodeError::Arrow`] si le lot construit ne correspond pas au schéma —
    /// ce qui serait un bug du driver, pas une donnée fautive.
    pub fn finish(&mut self) -> Result<RecordBatch, DecodeError> {
        let mut tableaux = Vec::with_capacity(self.columns.len());
        for colonne in &mut self.columns {
            tableaux.push(colonne.finish()?);
        }
        let lignes = self.rows;
        self.rows = 0;
        self.bytes = 0;

        // Un résultat sans colonne — un `INSERT`, un `SET` — a tout de même un
        // nombre de lignes, que `try_new` ne peut pas déduire de zéro colonne.
        if tableaux.is_empty() {
            let options = RecordBatchOptions::new().with_row_count(Some(lignes));
            return Ok(RecordBatch::try_new_with_options(
                Arc::clone(&self.schema),
                tableaux,
                &options,
            )?);
        }
        Ok(RecordBatch::try_new(Arc::clone(&self.schema), tableaux)?)
    }
}

/// Un constructeur de colonne Arrow, avec ce qu'il faut savoir pour y verser du
/// binaire PostgreSQL.
#[derive(Debug)]
enum ColumnBuilder {
    Bool(BooleanBuilder),
    Int16(Int16Builder),
    Int32(Int32Builder),
    Int64(Int64Builder),
    UInt32(UInt32Builder),
    Float32(Float32Builder),
    Float64(Float64Builder),
    /// Toutes les colonnes `Utf8`, distinguées par la façon d'obtenir le texte.
    Text(StringBuilder, TextSource),
    Binary(BinaryBuilder),
    Date(Date32Builder),
    Time(Time64MicrosecondBuilder),
    Timestamp(TimestampMicrosecondBuilder),
    Interval(IntervalMonthDayNanoBuilder),
    List(Box<ListColumn>),
}

/// D'où vient le texte d'une colonne `Utf8`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TextSource {
    /// Les octets **sont** le texte.
    Raw,
    /// Un octet de version, puis le texte.
    Jsonb,
    /// Le format binaire `numeric`, rendu en décimal exact.
    Numeric,
    /// Seize octets, rendus sous forme canonique.
    Uuid,
    /// Heure et décalage, rendus en ISO 8601.
    TimeTz,
    /// Type inconnu : texte si c'est de l'UTF-8 valide, hexadécimal sinon.
    Opaque,
}

/// Une colonne de listes, construite à la main.
///
/// `ListBuilder` d'Arrow exigerait de passer par `Box<dyn ArrayBuilder>` et de
/// redescendre par `downcast_mut` à chaque élément ; assembler les décalages
/// soi-même évite ce détour et garde le type de l'enfant.
#[derive(Debug)]
struct ListColumn {
    field: FieldRef,
    child: ColumnBuilder,
    /// Décalages cumulés, commençant par zéro.
    offsets: Vec<i32>,
    /// Une entrée par ligne : la liste elle-même est-elle non nulle ?
    validity: Vec<bool>,
    /// Éléments versés dans l'enfant depuis le dernier lot.
    elements: i32,
}

impl ColumnBuilder {
    /// Le constructeur correspondant à un plan de décodage.
    fn for_decoding(decoding: &PgDecoding) -> Self {
        match decoding {
            PgDecoding::Bool => Self::Bool(BooleanBuilder::new()),
            PgDecoding::Int16 => Self::Int16(Int16Builder::new()),
            PgDecoding::Int32 => Self::Int32(Int32Builder::new()),
            PgDecoding::Int64 => Self::Int64(Int64Builder::new()),
            PgDecoding::UInt32 => Self::UInt32(UInt32Builder::new()),
            PgDecoding::Float32 => Self::Float32(Float32Builder::new()),
            PgDecoding::Float64 => Self::Float64(Float64Builder::new()),
            PgDecoding::Text => Self::Text(StringBuilder::new(), TextSource::Raw),
            PgDecoding::Jsonb => Self::Text(StringBuilder::new(), TextSource::Jsonb),
            PgDecoding::Numeric => Self::Text(StringBuilder::new(), TextSource::Numeric),
            PgDecoding::Uuid => Self::Text(StringBuilder::new(), TextSource::Uuid),
            PgDecoding::TimeTz => Self::Text(StringBuilder::new(), TextSource::TimeTz),
            PgDecoding::Opaque => Self::Text(StringBuilder::new(), TextSource::Opaque),
            PgDecoding::Bytes => Self::Binary(BinaryBuilder::new()),
            PgDecoding::Date => Self::Date(Date32Builder::new()),
            PgDecoding::Time => Self::Time(Time64MicrosecondBuilder::new()),
            PgDecoding::Timestamp => Self::Timestamp(TimestampMicrosecondBuilder::new()),
            PgDecoding::TimestampTz => {
                Self::Timestamp(TimestampMicrosecondBuilder::new().with_timezone("UTC"))
            }
            PgDecoding::Interval => Self::Interval(IntervalMonthDayNanoBuilder::new()),
            PgDecoding::List(element) => Self::List(Box::new(ListColumn {
                field: Arc::new(Field::new("item", element.arrow_type(), true)),
                child: Self::for_decoding(element),
                offsets: vec![0],
                validity: Vec::new(),
                elements: 0,
            })),
        }
    }

    /// Ajoute une valeur, ou `NULL` quand `bytes` est absent.
    fn append(&mut self, format: PgValueFormat, bytes: Option<&[u8]>) -> Result<(), &'static str> {
        let Some(bytes) = bytes else {
            self.append_null();
            return Ok(());
        };

        // Le driver n'emploie que le protocole étendu, où le serveur répond
        // toujours en binaire. Le format texte ne peut venir que d'un chemin
        // qui n'existe pas encore ; les colonnes textuelles le supportent
        // gratuitement, les autres le refusent plutôt que de le deviner.
        if format == PgValueFormat::Text {
            return match self {
                Self::Text(builder, _) => {
                    let texte =
                        std::str::from_utf8(bytes).map_err(|_| "texte non UTF-8 du serveur")?;
                    builder.append_value(texte);
                    Ok(())
                }
                _ => Err("le format texte du protocole simple n'est pas décodé"),
            };
        }

        match self {
            Self::Bool(builder) => {
                let octet = bytes.first().ok_or("booléen vide")?;
                builder.append_value(*octet != 0);
            }
            Self::Int16(builder) => builder.append_value(read_i16(bytes)?),
            Self::Int32(builder) => builder.append_value(read_i32(bytes)?),
            Self::Int64(builder) => builder.append_value(read_i64(bytes)?),
            Self::UInt32(builder) => builder.append_value(read_u32(bytes)?),
            Self::Float32(builder) => {
                let octets: [u8; 4] = bytes.try_into().map_err(|_| "float4 mal dimensionné")?;
                builder.append_value(f32::from_be_bytes(octets));
            }
            Self::Float64(builder) => {
                let octets: [u8; 8] = bytes.try_into().map_err(|_| "float8 mal dimensionné")?;
                builder.append_value(f64::from_be_bytes(octets));
            }
            Self::Text(builder, source) => {
                render_text(builder, *source, bytes)?;
            }
            Self::Binary(builder) => builder.append_value(bytes),
            Self::Date(builder) => builder.append_value(read_date(bytes)?),
            Self::Time(builder) => builder.append_value(read_time(bytes)?),
            Self::Timestamp(builder) => builder.append_value(read_timestamp(bytes)?),
            Self::Interval(builder) => builder.append_value(read_interval(bytes)?),
            Self::List(colonne) => colonne.append(bytes)?,
        }
        Ok(())
    }

    /// Ajoute une valeur absente.
    fn append_null(&mut self) {
        match self {
            Self::Bool(builder) => builder.append_null(),
            Self::Int16(builder) => builder.append_null(),
            Self::Int32(builder) => builder.append_null(),
            Self::Int64(builder) => builder.append_null(),
            Self::UInt32(builder) => builder.append_null(),
            Self::Float32(builder) => builder.append_null(),
            Self::Float64(builder) => builder.append_null(),
            Self::Text(builder, _) => builder.append_null(),
            Self::Binary(builder) => builder.append_null(),
            Self::Date(builder) => builder.append_null(),
            Self::Time(builder) => builder.append_null(),
            Self::Timestamp(builder) => builder.append_null(),
            Self::Interval(builder) => builder.append_null(),
            Self::List(colonne) => colonne.append_null(),
        }
    }

    /// Clôt la colonne et repart à vide.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let tableau: ArrayRef = match self {
            Self::Bool(builder) => Arc::new(builder.finish()),
            Self::Int16(builder) => Arc::new(builder.finish()),
            Self::Int32(builder) => Arc::new(builder.finish()),
            Self::Int64(builder) => Arc::new(builder.finish()),
            Self::UInt32(builder) => Arc::new(builder.finish()),
            Self::Float32(builder) => Arc::new(builder.finish()),
            Self::Float64(builder) => Arc::new(builder.finish()),
            Self::Text(builder, _) => Arc::new(builder.finish()),
            Self::Binary(builder) => Arc::new(builder.finish()),
            Self::Date(builder) => Arc::new(builder.finish()),
            Self::Time(builder) => Arc::new(builder.finish()),
            Self::Timestamp(builder) => Arc::new(builder.finish()),
            Self::Interval(builder) => Arc::new(builder.finish()),
            Self::List(colonne) => colonne.finish()?,
        };
        Ok(tableau)
    }
}

impl ListColumn {
    /// Ajoute un tableau PostgreSQL à une dimension.
    fn append(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        let elements = split_array(bytes)?;
        for element in elements {
            self.child.append(PgValueFormat::Binary, element)?;
            self.elements = self
                .elements
                .checked_add(1)
                .ok_or("plus de 2 milliards d'éléments dans un lot")?;
        }
        self.offsets.push(self.elements);
        self.validity.push(true);
        Ok(())
    }

    /// Ajoute une liste absente. `NULL` et `{}` ne se confondent pas : la
    /// première n'a pas d'éléments, la seconde en a zéro.
    fn append_null(&mut self) {
        self.offsets.push(self.elements);
        self.validity.push(false);
    }

    /// Clôt la colonne et repart à vide.
    fn finish(&mut self) -> Result<ArrayRef, DecodeError> {
        let valeurs = self.child.finish()?;
        let decalages = std::mem::replace(&mut self.offsets, vec![0]);
        let validite = std::mem::take(&mut self.validity);
        self.elements = 0;

        let tableau = ListArray::try_new(
            Arc::clone(&self.field),
            OffsetBuffer::new(ScalarBuffer::from(decalages)),
            valeurs,
            Some(NullBuffer::from(validite)),
        )?;
        Ok(Arc::new(tableau))
    }
}

/// Écrit la représentation textuelle d'une valeur dans le constructeur.
fn render_text(
    builder: &mut StringBuilder,
    source: TextSource,
    bytes: &[u8],
) -> Result<(), &'static str> {
    match source {
        TextSource::Raw => {
            let texte = std::str::from_utf8(bytes).map_err(|_| "texte non UTF-8 du serveur")?;
            builder.append_value(texte);
        }
        TextSource::Jsonb => {
            let (version, suite) = bytes.split_first().ok_or("jsonb vide")?;
            if *version != JSONB_VERSION {
                return Err("version de jsonb inconnue");
            }
            let texte = std::str::from_utf8(suite).map_err(|_| "jsonb non UTF-8")?;
            builder.append_value(texte);
        }
        TextSource::Numeric => {
            let rendu = render_binary(bytes).ok_or("numeric illisible")?;
            builder.append_value(&rendu);
        }
        TextSource::Uuid => {
            let octets: [u8; UUID_LEN] = bytes.try_into().map_err(|_| "uuid mal dimensionné")?;
            let identifiant = uuid::Uuid::from_bytes(octets);
            let mut tampon = uuid::Uuid::encode_buffer();
            let canonique: &str = identifiant.hyphenated().encode_lower(&mut tampon);
            builder.append_value(canonique);
        }
        TextSource::TimeTz => {
            builder.append_value(&render_timetz(bytes)?);
        }
        TextSource::Opaque => {
            match std::str::from_utf8(bytes) {
                // Couvre les `enum`, `xml`, `citext`, `ltree` et la plupart des
                // types d'extension : leur représentation binaire *est* leur
                // texte.
                Ok(texte) => builder.append_value(texte),
                Err(_) => builder.append_value(render_hex(bytes)),
            }
        }
    }
    Ok(())
}

/// Transcrit des octets en hexadécimal, à la façon de `bytea` : `\x0badc0de`.
///
/// Au-delà de [`OPAQUE_HEX_BUDGET`] octets, la transcription est coupée et **le
/// dit** : la valeur rendue nomme la taille réelle, pour qu'un tronçon ne passe
/// jamais pour la valeur entière.
fn render_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let montres = bytes.len().min(OPAQUE_HEX_BUDGET);
    let mut sortie = String::with_capacity(montres.saturating_mul(2).saturating_add(32));
    sortie.push_str("\\x");
    for octet in bytes.iter().take(montres) {
        let haut = usize::from(octet >> 4);
        let bas = usize::from(octet & 0x0f);
        if let (Some(a), Some(b)) = (HEX.get(haut), HEX.get(bas)) {
            sortie.push(char::from(*a));
            sortie.push(char::from(*b));
        }
    }
    if montres < bytes.len() {
        let _ = write!(sortie, "… ({} octets)", bytes.len());
    }
    sortie
}

/// Rend un `timetz` en ISO 8601 : `14:30:00.250000+02:00`.
///
/// PostgreSQL transmet le décalage en **secondes à l'ouest** de UTC ; la
/// notation ISO le compte à l'est. Le signe s'inverse, et c'est exactement le
/// genre d'inversion qui décale une donnée de deux heures sans qu'on le voie
/// ([DRIVER-CONTRACT §7](../../../docs/DRIVER-CONTRACT.md)).
fn render_timetz(bytes: &[u8]) -> Result<String, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let micros = lecteur.i64().ok_or("timetz tronqué")?;
    let ouest = lecteur.i32().ok_or("timetz sans décalage")?;

    if !(0..=86_400_000_000).contains(&micros) {
        return Err("timetz hors des bornes d'une journée");
    }
    let secondes = micros / 1_000_000;
    let reste = micros % 1_000_000;
    let heures = secondes / 3_600;
    let minutes = (secondes % 3_600) / 60;
    let sec = secondes % 60;

    let est = ouest.checked_neg().ok_or("décalage de timetz absurde")?;
    let signe = if est < 0 { '-' } else { '+' };
    let absolu = est.unsigned_abs();
    let heures_offset = absolu / 3_600;
    let minutes_offset = (absolu % 3_600) / 60;

    let mut sortie = String::with_capacity(24);
    let _ = write!(sortie, "{heures:02}:{minutes:02}:{sec:02}");
    if reste != 0 {
        let _ = write!(sortie, ".{reste:06}");
    }
    let _ = write!(sortie, "{signe}{heures_offset:02}:{minutes_offset:02}");
    Ok(sortie)
}

/// Un `int2` gros-boutiste.
fn read_i16(bytes: &[u8]) -> Result<i16, &'static str> {
    let octets: [u8; 2] = bytes.try_into().map_err(|_| "int2 mal dimensionné")?;
    Ok(i16::from_be_bytes(octets))
}

/// Un `int4` gros-boutiste.
fn read_i32(bytes: &[u8]) -> Result<i32, &'static str> {
    let octets: [u8; 4] = bytes.try_into().map_err(|_| "int4 mal dimensionné")?;
    Ok(i32::from_be_bytes(octets))
}

/// Un `int8` gros-boutiste.
fn read_i64(bytes: &[u8]) -> Result<i64, &'static str> {
    let octets: [u8; 8] = bytes.try_into().map_err(|_| "int8 mal dimensionné")?;
    Ok(i64::from_be_bytes(octets))
}

/// Un `oid` gros-boutiste.
fn read_u32(bytes: &[u8]) -> Result<u32, &'static str> {
    let octets: [u8; 4] = bytes.try_into().map_err(|_| "oid mal dimensionné")?;
    Ok(u32::from_be_bytes(octets))
}

/// Une `date` : jours depuis 2000-01-01, ramenés à l'époque Unix.
fn read_date(bytes: &[u8]) -> Result<i32, &'static str> {
    let jours = read_i32(bytes)?;
    if jours == i32::MIN || jours == i32::MAX {
        // `infinity` et `-infinity` sont des dates légales en PostgreSQL. Aucune
        // valeur de `Date32` ne les représente ; les rendre comme la date
        // extrême serait faux, et `NULL` serait un mensonge.
        return Err("date infinie, non représentable en Date32 — la convertir en texte");
    }
    jours
        .checked_add(EPOCH_SHIFT_DAYS)
        .ok_or("date hors des bornes de Date32")
}

/// Un `time` : microsecondes depuis minuit.
fn read_time(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if !(0..=86_400_000_000).contains(&micros) {
        return Err("time hors des bornes d'une journée");
    }
    Ok(micros)
}

/// Un `timestamp` ou `timestamptz` : microsecondes depuis 2000-01-01, ramenées à
/// l'époque Unix.
fn read_timestamp(bytes: &[u8]) -> Result<i64, &'static str> {
    let micros = read_i64(bytes)?;
    if micros == i64::MIN || micros == i64::MAX {
        return Err("horodatage infini, non représentable — le convertir en texte");
    }
    micros
        .checked_add(EPOCH_SHIFT_MICROS)
        .ok_or("horodatage hors des bornes")
}

/// Un `interval` : microsecondes, jours, mois — dans cet ordre sur le fil.
fn read_interval(bytes: &[u8]) -> Result<IntervalMonthDayNano, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let micros = lecteur.i64().ok_or("interval tronqué")?;
    let jours = lecteur.i32().ok_or("interval sans jours")?;
    let mois = lecteur.i32().ok_or("interval sans mois")?;
    let nanos = micros
        .checked_mul(1_000)
        .ok_or("interval de plus de 292 ans en microsecondes, non représentable")?;
    Ok(IntervalMonthDayNano::new(mois, jours, nanos))
}

/// Découpe un tableau PostgreSQL binaire en éléments.
///
/// Rend une tranche par élément, `None` pour un élément absent. Le type
/// d'élément annoncé dans l'en-tête est **ignoré** : le plan de décodage vient
/// du `RowDescription`, qui fait foi, et suivre l'en-tête permettrait à un
/// serveur de faire décoder n'importe quoi comme n'importe quoi.
fn split_array(bytes: &[u8]) -> Result<Vec<Option<&[u8]>>, &'static str> {
    let mut lecteur = Lecteur::new(bytes);
    let dimensions = lecteur.i32().ok_or("en-tête de tableau tronqué")?;
    let _drapeaux = lecteur.i32().ok_or("en-tête de tableau tronqué")?;
    let _type_element = lecteur.u32().ok_or("en-tête de tableau tronqué")?;

    if dimensions == 0 {
        return Ok(Vec::new());
    }
    if dimensions != 1 {
        // Une liste Arrow est à une dimension. Aplatir perdrait la forme sans
        // le dire ; refuser la nomme.
        return Err("tableau à plusieurs dimensions, non représentable en liste Arrow");
    }

    let longueur = lecteur.i32().ok_or("dimension de tableau tronquée")?;
    let _borne_basse = lecteur.i32().ok_or("dimension de tableau tronquée")?;
    let longueur = usize::try_from(longueur).map_err(|_| "longueur de tableau négative")?;

    // Chaque élément coûte au moins ses quatre octets de longueur : une valeur
    // plus grande que ce que le tampon peut contenir est hostile, et
    // `with_capacity` sur une telle valeur épuiserait la mémoire.
    if longueur > lecteur.reste().len() / 4 + 1 {
        return Err("longueur de tableau incohérente avec le tampon reçu");
    }

    let mut elements = Vec::with_capacity(longueur);
    for _ in 0..longueur {
        let taille = lecteur.i32().ok_or("élément de tableau tronqué")?;
        if taille == -1 {
            elements.push(None);
            continue;
        }
        let taille = usize::try_from(taille).map_err(|_| "taille d'élément négative")?;
        elements.push(Some(
            lecteur
                .prendre(taille)
                .ok_or("élément de tableau tronqué")?,
        ));
    }
    Ok(elements)
}

#[cfg(test)]
mod tests {
    use arrow::array::{Array, AsArray, StringArray};
    use arrow::datatypes::{Int32Type, Schema};

    use super::*;

    /// Construit un tableau PostgreSQL binaire à une dimension.
    fn tableau(elements: &[Option<&[u8]>]) -> Vec<u8> {
        let mut octets = Vec::new();
        octets.extend_from_slice(&1_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        octets.extend_from_slice(
            &i32::try_from(elements.len())
                .expect("cas de test court")
                .to_be_bytes(),
        );
        octets.extend_from_slice(&1_i32.to_be_bytes());
        for element in elements {
            match element {
                Some(valeur) => {
                    octets.extend_from_slice(
                        &i32::try_from(valeur.len())
                            .expect("cas de test court")
                            .to_be_bytes(),
                    );
                    octets.extend_from_slice(valeur);
                }
                None => octets.extend_from_slice(&(-1_i32).to_be_bytes()),
            }
        }
        octets
    }

    fn colonne(decoding: &PgDecoding) -> ColumnBuilder {
        ColumnBuilder::for_decoding(decoding)
    }

    fn texte_rendu(source: TextSource, bytes: &[u8]) -> Result<String, &'static str> {
        let mut builder = StringBuilder::new();
        render_text(&mut builder, source, bytes)?;
        let tableau: StringArray = builder.finish();
        Ok(tableau.value(0).to_owned())
    }

    #[test]
    fn les_entiers_se_relisent_gros_boutistes() {
        assert_eq!(read_i16(&1234_i16.to_be_bytes()), Ok(1234));
        assert_eq!(read_i32(&(-7_i32).to_be_bytes()), Ok(-7));
        assert_eq!(read_i64(&i64::MAX.to_be_bytes()), Ok(i64::MAX));
        assert_eq!(
            read_u32(&4_000_000_000_u32.to_be_bytes()),
            Ok(4_000_000_000)
        );
    }

    #[test]
    fn un_tampon_mal_dimensionne_rend_une_erreur_pas_une_panique() {
        // Les octets viennent du réseau : un serveur peut mentir sur les tailles.
        assert!(read_i32(&[0, 1]).is_err());
        assert!(read_i64(&[]).is_err());
        assert!(read_i16(&[0, 0, 0]).is_err());
    }

    #[test]
    fn une_date_se_ramene_a_l_epoque_unix() {
        // 2000-01-01 vaut 0 côté PostgreSQL et 10 957 côté Arrow.
        assert_eq!(read_date(&0_i32.to_be_bytes()), Ok(EPOCH_SHIFT_DAYS));
        // 1970-01-01 : -10 957 jours avant l'époque PostgreSQL.
        assert_eq!(read_date(&(-EPOCH_SHIFT_DAYS).to_be_bytes()), Ok(0));
    }

    #[test]
    fn une_date_infinie_est_refusee_plutot_que_rendue_fausse() {
        // PostgreSQL admet `infinity` ; Date32 non. La rendre comme la date
        // extrême ou comme NULL serait un mensonge sur une donnée réelle.
        let erreur = read_date(&i32::MAX.to_be_bytes()).expect_err("refus attendu");
        assert!(erreur.contains("infinie"), "{erreur}");
        assert!(read_date(&i32::MIN.to_be_bytes()).is_err());
    }

    #[test]
    fn un_horodatage_se_ramene_a_l_epoque_unix() {
        assert_eq!(read_timestamp(&0_i64.to_be_bytes()), Ok(EPOCH_SHIFT_MICROS));
        assert!(read_timestamp(&i64::MAX.to_be_bytes()).is_err());
    }

    #[test]
    fn une_heure_hors_des_bornes_d_une_journee_est_refusee() {
        assert_eq!(read_time(&0_i64.to_be_bytes()), Ok(0));
        assert_eq!(
            read_time(&86_400_000_000_i64.to_be_bytes()),
            Ok(86_400_000_000)
        );
        assert!(read_time(&(-1_i64).to_be_bytes()).is_err());
        assert!(read_time(&86_400_000_001_i64.to_be_bytes()).is_err());
    }

    #[test]
    fn un_intervalle_garde_ses_trois_composantes_distinctes() {
        // Un mois n'est pas 30 jours et un jour n'est pas 24 heures : les
        // fusionner fausserait tout calcul de calendrier.
        let mut octets = Vec::new();
        octets.extend_from_slice(&3_600_000_000_i64.to_be_bytes());
        octets.extend_from_slice(&2_i32.to_be_bytes());
        octets.extend_from_slice(&14_i32.to_be_bytes());
        let intervalle = read_interval(&octets).expect("interval valide");
        assert_eq!(intervalle.months, 14);
        assert_eq!(intervalle.days, 2);
        assert_eq!(intervalle.nanoseconds, 3_600_000_000_000);
    }

    #[test]
    fn un_intervalle_qui_deborde_en_nanosecondes_est_refuse() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&i64::MAX.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        assert!(read_interval(&octets).is_err());
    }

    #[test]
    fn le_decalage_d_un_timetz_change_de_signe() {
        // PostgreSQL compte les secondes à l'ouest ; ISO 8601 à l'est. Une
        // inversion ratée décale la donnée sans que rien ne le signale.
        let mut octets = Vec::new();
        octets.extend_from_slice(&52_200_000_000_i64.to_be_bytes()); // 14:30:00
        octets.extend_from_slice(&(-7_200_i32).to_be_bytes()); // 7200 s à l'est
        assert_eq!(render_timetz(&octets).as_deref(), Ok("14:30:00+02:00"));

        let mut ouest = Vec::new();
        ouest.extend_from_slice(&52_200_000_000_i64.to_be_bytes());
        ouest.extend_from_slice(&18_000_i32.to_be_bytes()); // 5 h à l'ouest
        assert_eq!(render_timetz(&ouest).as_deref(), Ok("14:30:00-05:00"));
    }

    #[test]
    fn un_timetz_avec_des_microsecondes_les_garde() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&52_200_250_000_i64.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        assert_eq!(
            render_timetz(&octets).as_deref(),
            Ok("14:30:00.250000+00:00")
        );
    }

    #[test]
    fn un_jsonb_perd_son_octet_de_version_et_rien_d_autre() {
        let mut octets = vec![JSONB_VERSION];
        octets.extend_from_slice(br#"{"a": 1}"#);
        assert_eq!(
            texte_rendu(TextSource::Jsonb, &octets).as_deref(),
            Ok(r#"{"a": 1}"#)
        );
        // Une version inconnue se refuse : le reste n'est plus du JSON.
        assert!(texte_rendu(TextSource::Jsonb, &[9, b'{']).is_err());
    }

    #[test]
    fn un_uuid_se_rend_sous_forme_canonique() {
        let octets: [u8; 16] = [
            0x67, 0xe5, 0x50, 0x44, 0x10, 0xb1, 0x42, 0x6f, 0x9d, 0x0c, 0x45, 0x1f, 0x8a, 0xd0,
            0x5b, 0x1a,
        ];
        assert_eq!(
            texte_rendu(TextSource::Uuid, &octets).as_deref(),
            Ok("67e55044-10b1-426f-9d0c-451f8ad05b1a")
        );
        assert!(texte_rendu(TextSource::Uuid, &octets[..8]).is_err());
    }

    #[test]
    fn un_type_inconnu_lisible_rend_son_texte() {
        // C'est le cas des `enum`, dont la représentation binaire est
        // l'étiquette : une erreur ici ferait échouer toute la requête.
        assert_eq!(
            texte_rendu(TextSource::Opaque, b"expedie").as_deref(),
            Ok("expedie")
        );
    }

    #[test]
    fn un_type_inconnu_binaire_rend_de_l_hexadecimal_pas_une_erreur() {
        let rendu = texte_rendu(TextSource::Opaque, &[0x00, 0xff, 0x10]).expect("jamais d'erreur");
        assert_eq!(rendu, "\\x00ff10");
    }

    #[test]
    fn une_transcription_coupee_annonce_la_taille_reelle() {
        // Un tronçon qui passerait pour la valeur entière serait un mensonge.
        let gros = vec![0xab_u8; OPAQUE_HEX_BUDGET + 10];
        let rendu = render_hex(&gros);
        assert!(rendu.contains("octets"), "la coupe doit se dire");
        assert!(
            rendu.contains(&(OPAQUE_HEX_BUDGET + 10).to_string()),
            "{rendu}"
        );
    }

    #[test]
    fn un_tableau_vide_et_un_tableau_absent_ne_se_confondent_pas() {
        let mut colonne = colonne(&PgDecoding::List(Box::new(PgDecoding::Int32)));
        // `{}` : zéro élément, mais la liste existe.
        let vide = tableau(&[]);
        colonne
            .append(PgValueFormat::Binary, Some(vide.as_slice()))
            .expect("tableau vide valide");
        // `NULL` : pas de liste du tout.
        colonne.append(PgValueFormat::Binary, None).expect("null");

        let tableau_arrow = colonne.finish().expect("liste construite");
        assert_eq!(tableau_arrow.len(), 2);
        assert!(!tableau_arrow.is_null(0), "{{}} n'est pas NULL");
        assert!(tableau_arrow.is_null(1));
    }

    #[test]
    fn un_tableau_d_entiers_conserve_ses_elements_et_ses_trous() {
        let mut colonne = colonne(&PgDecoding::List(Box::new(PgDecoding::Int32)));
        let un = 1_i32.to_be_bytes();
        let trois = 3_i32.to_be_bytes();
        let octets = tableau(&[Some(&un), None, Some(&trois)]);
        colonne
            .append(PgValueFormat::Binary, Some(octets.as_slice()))
            .expect("tableau valide");

        let tableau_arrow = colonne.finish().expect("liste construite");
        let listes = tableau_arrow
            .as_list_opt::<i32>()
            .expect("une colonne de listes");
        let premiere = listes.value(0);
        let entiers = premiere
            .as_primitive_opt::<Int32Type>()
            .expect("des entiers 32 bits");
        assert_eq!(entiers.len(), 3);
        assert_eq!(entiers.value(0), 1);
        assert!(entiers.is_null(1), "un élément NULL reste NULL");
        assert_eq!(entiers.value(2), 3);
    }

    #[test]
    fn un_tableau_a_plusieurs_dimensions_est_refuse_pas_aplati() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&2_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        let erreur = split_array(&octets).expect_err("refus attendu");
        assert!(erreur.contains("dimensions"), "{erreur}");
    }

    #[test]
    fn une_longueur_de_tableau_hostile_n_epuise_pas_la_memoire() {
        let mut octets = Vec::new();
        octets.extend_from_slice(&1_i32.to_be_bytes());
        octets.extend_from_slice(&0_i32.to_be_bytes());
        octets.extend_from_slice(&23_u32.to_be_bytes());
        octets.extend_from_slice(&i32::MAX.to_be_bytes());
        octets.extend_from_slice(&1_i32.to_be_bytes());
        assert!(split_array(&octets).is_err());
    }

    #[test]
    fn un_nom_de_type_hostile_ne_sort_pas_tel_quel_dans_une_erreur() {
        // Un type utilisateur peut porter des séquences de contrôle de terminal.
        assert_eq!(sanitize_type("int4"), "int4");
        assert_eq!(sanitize_type("BYTEA[]"), "BYTEA[]");
        assert_eq!(sanitize_type("\u{1b}[2J"), "<type non représentable>");
        assert_eq!(sanitize_type(""), "<type non représentable>");
        assert_eq!(sanitize_type(&"a".repeat(200)), "<type non représentable>");
    }

    #[test]
    fn un_resultat_sans_colonne_garde_son_nombre_de_lignes() {
        // Un `INSERT` n'a pas de colonne mais a des lignes affectées : sans
        // `with_row_count`, Arrow construirait un lot de zéro ligne.
        let schema = Arc::new(Schema::empty());
        let mut assembleur = BatchAssembler::new(schema, &[]);
        assembleur.rows = 3;
        let lot = assembleur.finish().expect("lot sans colonne");
        assert_eq!(lot.num_rows(), 3);
        assert_eq!(lot.num_columns(), 0);
    }

    #[test]
    fn l_assembleur_repart_a_vide_apres_chaque_lot() {
        let schema = Arc::new(Schema::empty());
        let mut assembleur = BatchAssembler::new(schema, &[]);
        assembleur.rows = 5;
        assembleur.bytes = 4_096;
        let _ = assembleur.finish().expect("lot sans colonne");
        assert!(assembleur.is_empty());
        assert_eq!(assembleur.bytes(), 0);
    }
}
