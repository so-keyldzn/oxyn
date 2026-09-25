//! Export d'un [`ResultBuffer`] vers un fichier.
//!
//! **En flux, jamais en bloc.** L'export relit les lots un par un — y compris
//! ceux qui ont débordé sur le disque — et les écrit au fil de l'eau. Exporter
//! 40 millions de lignes ne demande pas plus de mémoire qu'en exporter mille
//! ([I-06](../../../CLAUDE.md#i-06)).
//!
//! **Ce qui est écrit se relit sans Oxyn.** CSV, JSON et Arrow IPC sont des
//! formats publics, produits par les écrivains d'`arrow-rs` — les mêmes que ceux
//! qui formatent la grille, donc le fichier et l'écran ne divergent pas
//! ([I-11](../../../CLAUDE.md#i-11)).
//!
//! **Un export partiel est refusé par défaut.** Un fichier tronqué qui ressemble
//! à un fichier complet est une perte de données silencieuse ; l'appelant qui
//! l'accepte le déclare avec [`ExportOptions::allow_incomplete`].
//!
//! **A truncated result is always refused.** A buffer closed by a row limit,
//! saturation, a cancellation or a timeout has finished loading: nothing on
//! screen tells it from a whole result, which is why no option allows it
//! ([UX-SPEC](../../../docs/UX-SPEC.md#ce-qui-est-exporté-est-ce-qui-est-affiché)).

use std::borrow::Cow;
use std::io::Write;

use arrow::csv::WriterBuilder as CsvWriterBuilder;
use arrow::ipc::writer::FileWriter as IpcFileWriter;
use arrow::json::{ArrayWriter, LineDelimitedWriter};
use arrow::record_batch::RecordBatch;
use oxyn_core::{CancelToken, ExportFormat};
use serde::{Deserialize, Serialize};

use crate::buffer::{BatchIndex, ResultBuffer};
use crate::error::{DataError, Result};

/// Réglages d'un export.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct ExportOptions {
    /// Écrire une ligne d'en-tête. Ne concerne que CSV et TSV.
    pub header: bool,
    /// Ce qu'écrire à la place d'une valeur absente, pour CSV et TSV.
    ///
    /// Vide par défaut, qui est la convention du CSV : `a,,c`. Y mettre `NULL`
    /// rend le fichier ambigu dès qu'une colonne texte contient ce mot.
    pub null_text: Cow<'static, str>,
    /// Motif de formatage des horodatages, au sens de `chrono`.
    ///
    /// `None` = RFC 3339, qui est ce que relisent les tableurs et les bases.
    pub timestamp_format: Option<Cow<'static, str>>,
    /// Accepter d'exporter un résultat encore en cours de réception.
    ///
    /// L'export prend alors un instantané des lots reçus au moment de l'appel.
    pub allow_incomplete: bool,
}

impl Default for ExportOptions {
    fn default() -> Self {
        Self {
            header: true,
            null_text: Cow::Borrowed(""),
            timestamp_format: None,
            allow_incomplete: false,
        }
    }
}

impl ExportOptions {
    /// Écrire ou non la ligne d'en-tête.
    #[must_use]
    pub fn with_header(mut self, header: bool) -> Self {
        self.header = header;
        self
    }

    /// Change le texte des valeurs absentes.
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

    /// Autorise l'export d'un résultat incomplet.
    #[must_use]
    pub fn allowing_incomplete(mut self) -> Self {
        self.allow_incomplete = true;
        self
    }
}

/// Ce qu'un export a produit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct ExportSummary {
    /// Lignes écrites.
    pub rows: usize,
    /// Lots relus.
    pub batches: usize,
    /// Octets écrits.
    pub bytes: u64,
}

/// Ce format est-il réellement écrit par [`export`] ?
///
/// Existe pour que l'interface n'offre pas un format qu'elle ne peut pas
/// produire : sans cela, le geste part, le sélecteur de fichier s'ouvre,
/// l'utilisateur nomme sa destination — et l'échec n'arrive qu'après, en
/// laissant un fichier vide sur son disque. Ce qui n'est pas disponible
/// s'annonce avant le clic.
///
/// Reste aligné sur le `match` d'[`export`] : le test
/// `chaque_format_declare_ecrivable_secrit_vraiment` échoue sinon.
#[must_use]
pub const fn is_supported(format: ExportFormat) -> bool {
    matches!(
        format,
        ExportFormat::Csv
            | ExportFormat::Tsv
            | ExportFormat::Json
            | ExportFormat::JsonLines
            | ExportFormat::ArrowIpc
    )
}

/// Écrit le contenu de `buffer` dans `writer`.
///
/// Relit les lots un par un, y compris depuis le fichier de débordement, et
/// vérifie l'annulation entre chaque lot. Ne matérialise jamais plus d'un lot à
/// la fois.
///
/// Un résultat sans aucune ligne produit un fichier **vide**, sans même la ligne
/// d'en-tête : c'est le comportement de l'écrivain CSV d'Arrow, qui n'écrit
/// l'en-tête qu'avec le premier lot. L'appelant qui veut un fichier à en-tête
/// seul doit le composer lui-même.
///
/// # Erreurs
///
/// * [`DataError::IncompleteResult`] si le résultat coule encore et que
///   [`ExportOptions::allow_incomplete`] est faux ;
/// * [`DataError::TruncatedResult`] when rows of the result are missing;
/// * [`DataError::UnsupportedFormat`] pour Parquet, SQL et Markdown ;
/// * [`DataError::Cancelled`] si le jeton est déclenché — le fichier
///   partiellement écrit reste à la charge de l'appelant, qui seul sait s'il
///   faut l'effacer ;
/// * [`DataError::Io`], [`DataError::Arrow`] ou [`DataError::Spill`] sinon.
pub fn export<W: Write>(
    buffer: &ResultBuffer,
    format: ExportFormat,
    writer: W,
    opts: &ExportOptions,
    ct: &CancelToken,
) -> Result<ExportSummary> {
    ensure_exportable(buffer, opts)?;

    // Instantané : le nombre de lots est relu une seule fois, pour que l'export
    // d'un résultat encore en cours ait une fin définie.
    let source = Source {
        buffer,
        ct,
        lots: buffer.batch_count(),
    };
    let mut compteur = CountingWriter::new(writer);

    let mut resume = match format {
        ExportFormat::Csv => ecrire_delimite(&source, &mut compteur, opts, b',')?,
        ExportFormat::Tsv => ecrire_delimite(&source, &mut compteur, opts, b'\t')?,
        ExportFormat::JsonLines => {
            let mut sortie = LineDelimitedWriter::new(&mut compteur);
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            sortie.finish()?;
            resume
        }
        ExportFormat::Json => {
            let mut sortie = ArrayWriter::new(&mut compteur);
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            // Sans `finish`, le tableau JSON n'est jamais refermé : le fichier
            // est illisible et rien ne l'a signalé.
            sortie.finish()?;
            resume
        }
        ExportFormat::ArrowIpc => {
            let mut sortie = IpcFileWriter::try_new(&mut compteur, buffer.schema().as_ref())?;
            let resume =
                source.pour_chaque_lot(|lot| sortie.write(lot).map_err(DataError::from))?;
            // Le pied de page porte l'index des blocs : sans lui, le fichier
            // n'est pas un fichier Arrow.
            sortie.finish()?;
            resume
        }
        // TODO(phase 1, ouvert le 2026-09-05) : Parquet attend l'ajout de la
        // crate `parquet` au manifeste du workspace ; SQL attend la citation
        // d'identifiants de `oxyn-query` (I-10) ; Markdown attend la mise en
        // forme décidée par l'interface. Aucun des trois n'est un manque de code
        // ici : chacun attend une dépendance qui n'existe pas encore.
        autre => {
            return Err(DataError::UnsupportedFormat {
                format: autre.extension(),
            });
        }
    };

    compteur.flush()?;
    resume.bytes = compteur.bytes;
    Ok(resume)
}

/// Refuses what [`export`] would refuse, without writing anything.
///
/// For the caller that prepares a destination before writing: the refusal
/// then comes before any file is created.
///
/// # Errors
///
/// [`DataError::TruncatedResult`] or [`DataError::IncompleteResult`].
pub fn ensure_exportable(buffer: &ResultBuffer, opts: &ExportOptions) -> Result<()> {
    // `truncated` first: it is never cleared, and a truncated buffer still
    // open will not become whole by waiting.
    if buffer.stats().truncated {
        return Err(DataError::TruncatedResult);
    }
    if !buffer.is_complete() && !opts.allow_incomplete {
        return Err(DataError::IncompleteResult);
    }
    Ok(())
}

/// L'instantané des lots à écrire.
///
/// Le nombre de lots est relevé **une fois**, à la construction : sans cela, un
/// export lancé sur un résultat encore en cours n'aurait pas de fin définie.
#[derive(Debug, Clone, Copy)]
struct Source<'a> {
    buffer: &'a ResultBuffer,
    ct: &'a CancelToken,
    lots: usize,
}

impl Source<'_> {
    /// Relit les lots de l'instantané et les passe à `ecrire`, en vérifiant
    /// l'annulation entre chacun.
    fn pour_chaque_lot<F>(&self, mut ecrire: F) -> Result<ExportSummary>
    where
        F: FnMut(&RecordBatch) -> Result<()>,
    {
        let mut resume = ExportSummary::default();
        for position in 0..self.lots {
            // Entre deux lots, pas au milieu : un fichier coupé en plein
            // encodage n'est pas récupérable, alors qu'un fichier coupé sur une
            // frontière de lot est un préfixe valide.
            if self.ct.is_cancelled() {
                return Err(DataError::Cancelled);
            }
            let Some(lot) = self.buffer.batch(BatchIndex::new(position))? else {
                // Le nombre de lots a été relevé avant la boucle ; un trou
                // signale un invariant rompu, pas une course normale.
                return Err(DataError::Spill(std::io::Error::other(
                    "a batch vanished from the buffer during export",
                )));
            };
            ecrire(&lot)?;
            resume.rows = resume.rows.saturating_add(lot.num_rows());
            resume.batches = resume.batches.saturating_add(1);
        }
        Ok(resume)
    }
}

/// CSV et TSV : même écrivain, un séparateur près.
///
/// `arrow::csv::Writer` vide son tampon interne à chaque lot écrit : une sortie
/// abandonnée en cours de route reste un préfixe valide.
fn ecrire_delimite<W: Write>(
    source: &Source<'_>,
    sortie: &mut CountingWriter<W>,
    opts: &ExportOptions,
    delimiteur: u8,
) -> Result<ExportSummary> {
    let mut constructeur = CsvWriterBuilder::new()
        .with_header(opts.header)
        .with_delimiter(delimiteur)
        .with_null(opts.null_text.as_ref().to_owned());
    if let Some(motif) = opts.timestamp_format.as_deref() {
        constructeur = constructeur
            .with_timestamp_format(motif.to_owned())
            .with_timestamp_tz_format(motif.to_owned());
    }

    let mut ecrivain = constructeur.build(sortie);
    source.pour_chaque_lot(|lot| ecrivain.write(lot).map_err(DataError::from))
}

/// Écrivain qui compte ce qui le traverse.
///
/// Le compte sert au retour d'[`ExportSummary`] et à l'affichage de progression :
/// sans lui, un export de 4 Go n'a aucun repère à montrer.
#[derive(Debug)]
struct CountingWriter<W> {
    inner: W,
    bytes: u64,
}

impl<W: Write> CountingWriter<W> {
    const fn new(inner: W) -> Self {
        Self { inner, bytes: 0 }
    }
}

impl<W: Write> Write for CountingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let ecrits = self.inner.write(buf)?;
        self.bytes = self
            .bytes
            .saturating_add(u64::try_from(ecrits).unwrap_or(u64::MAX));
        Ok(ecrits)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::ipc::reader::FileReader;
    use oxyn_core::ExecStats;

    use super::*;
    use crate::buffer::BufferLimits;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int32, false),
            Field::new("nom", DataType::Utf8, true),
        ]))
    }

    fn lot(depart: i32, lignes: usize) -> RecordBatch {
        let ids: Vec<i32> = (0..lignes)
            .map(|i| depart.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        let noms: Vec<Option<String>> = ids
            .iter()
            .map(|i| {
                if i % 2 == 0 {
                    Some(format!("n{i}"))
                } else {
                    None
                }
            })
            .collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(noms)),
            ],
        )
        .expect("les colonnes correspondent au schéma construit juste au-dessus")
    }

    /// Tampon clos, avec un budget si petit que tout a débordé sur le disque :
    /// c'est le cas qui compte, puisque l'export doit relire depuis le fichier
    /// temporaire.
    fn tampon_deborde() -> ResultBuffer {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_memory_budget(1));
        tampon.push(lot(0, 3)).expect("lot accepté");
        tampon.push(lot(100, 2)).expect("lot accepté");
        tampon.mark_complete(ExecStats::default());
        assert_eq!(tampon.spilled_batches(), 2);
        tampon
    }

    fn exporter(format: ExportFormat, opts: &ExportOptions) -> (String, ExportSummary) {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        let resume = export(&tampon, format, &mut sortie, opts, &CancelToken::new())
            .expect("export sans erreur");
        (
            String::from_utf8(sortie).expect("les formats testés sont en UTF-8"),
            resume,
        )
    }

    #[test]
    fn le_csv_porte_son_en_tete_et_toutes_ses_lignes() {
        let (texte, resume) = exporter(ExportFormat::Csv, &ExportOptions::default());
        let lignes: Vec<&str> = texte.lines().collect();

        assert_eq!(lignes.first(), Some(&"id,nom"));
        assert_eq!(lignes.len(), 6, "un en-tête et cinq lignes : {texte}");
        assert_eq!(resume.rows, 5);
        assert_eq!(resume.batches, 2);
        assert!(resume.bytes > 0);
    }

    /// Le débordement disque ne doit rien changer au contenu exporté.
    #[test]
    fn l_export_relit_les_lots_debordes() {
        let (texte, _) = exporter(ExportFormat::Csv, &ExportOptions::default());
        for attendu in ["0,n0", "1,", "2,n2", "100,n100", "101,"] {
            assert!(texte.contains(attendu), "{attendu} absent de :\n{texte}");
        }
    }

    #[test]
    fn le_tsv_utilise_la_tabulation() {
        let (texte, _) = exporter(ExportFormat::Tsv, &ExportOptions::default());
        assert!(texte.starts_with("id\tnom"), "{texte}");
    }

    #[test]
    fn l_en_tete_est_optionnel() {
        let (texte, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_header(false),
        );
        assert!(!texte.starts_with("id,nom"), "{texte}");
        assert_eq!(texte.lines().count(), 5);
    }

    #[test]
    fn le_texte_des_valeurs_absentes_est_configurable() {
        let (texte, _) = exporter(
            ExportFormat::Csv,
            &ExportOptions::default().with_null_text("\\N"),
        );
        assert!(texte.contains("1,\\N"), "{texte}");
    }

    #[test]
    fn le_json_par_lignes_produit_un_objet_par_ligne() {
        let (texte, resume) = exporter(ExportFormat::JsonLines, &ExportOptions::default());
        let lignes: Vec<&str> = texte.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lignes.len(), 5, "{texte}");
        assert!(
            lignes.first().is_some_and(|l| l.starts_with('{')),
            "{texte}"
        );
        assert_eq!(resume.rows, 5);
    }

    #[test]
    fn le_json_produit_un_tableau_referme() {
        let (texte, _) = exporter(ExportFormat::Json, &ExportOptions::default());
        assert!(texte.starts_with('['), "{texte}");
        assert!(texte.trim_end().ends_with(']'), "{texte}");
    }

    /// L'aller-retour Arrow IPC est le seul export qui ne convertit rien : ce
    /// qui sort doit être exactement ce qui est entré.
    #[test]
    fn l_arrow_ipc_fait_un_aller_retour_exact() {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        let resume = export(
            &tampon,
            ExportFormat::ArrowIpc,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export sans erreur");
        assert_eq!(resume.rows, 5);

        let lecteur = FileReader::try_new(std::io::Cursor::new(sortie), None)
            .expect("le fichier écrit doit être un fichier Arrow valide");
        assert_eq!(lecteur.schema().fields(), schema().fields());

        let relus: Vec<RecordBatch> = lecteur
            .collect::<std::result::Result<Vec<_>, _>>()
            .expect("relecture des lots");
        assert_eq!(relus.len(), 2);
        assert_eq!(relus.first(), Some(&lot(0, 3)));
        assert_eq!(relus.get(1), Some(&lot(100, 2)));
    }

    #[test]
    fn parquet_est_refuse_explicitement() {
        let tampon = tampon_deborde();
        let mut sortie: Vec<u8> = Vec::new();
        match export(
            &tampon,
            ExportFormat::Parquet,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::UnsupportedFormat { format }) => assert_eq!(format, "parquet"),
            autre => panic!("attendu UnsupportedFormat, obtenu {autre:?}"),
        }
        assert!(sortie.is_empty(), "rien ne doit être écrit");
    }

    /// Le mode de panne à éviter : un fichier partiel qui a l'air complet.
    #[test]
    fn un_resultat_en_cours_ne_s_exporte_pas_par_accident() {
        let tampon = ResultBuffer::new(schema(), 1 << 20);
        tampon.push(lot(0, 3)).expect("lot accepté");
        let mut sortie: Vec<u8> = Vec::new();

        match export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        ) {
            Err(DataError::IncompleteResult) => {}
            autre => panic!("attendu IncompleteResult, obtenu {autre:?}"),
        }

        // Déclaré explicitement, c'est autorisé.
        let resume = export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default().allowing_incomplete(),
            &CancelToken::new(),
        )
        .expect("export explicite d'un résultat partiel");
        assert_eq!(resume.rows, 3);
    }

    /// Closed but truncated: the case nothing on screen tells from a whole
    /// result. No option allows it, not even `allowing_incomplete`.
    #[test]
    fn a_truncated_result_never_exports() {
        let tampon = tampon_deborde();
        tampon.mark_truncated();

        for opts in [
            ExportOptions::default(),
            ExportOptions::default().allowing_incomplete(),
        ] {
            let mut sortie: Vec<u8> = Vec::new();
            match export(
                &tampon,
                ExportFormat::Csv,
                &mut sortie,
                &opts,
                &CancelToken::new(),
            ) {
                Err(DataError::TruncatedResult) => {}
                autre => panic!("expected TruncatedResult, got {autre:?}"),
            }
            assert!(sortie.is_empty(), "nothing may be written");
        }
    }

    #[test]
    fn une_annulation_interrompt_l_export() {
        let tampon = tampon_deborde();
        let ct = CancelToken::new();
        ct.cancel();
        let mut sortie: Vec<u8> = Vec::new();

        match export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &ct,
        ) {
            Err(DataError::Cancelled) => {}
            autre => panic!("attendu Cancelled, obtenu {autre:?}"),
        }
    }

    #[test]
    fn chaque_format_declare_ecrivable_secrit_vraiment() {
        // `is_supported` gouverne ce que l'interface propose. Une divergence
        // entre cette liste et le `match` d'`export` ne casse rien ici : elle
        // casse trois clics plus loin, après que l'utilisateur a nommé son
        // fichier, et laisse un fichier vide derrière elle.
        let tampon = tampon_deborde();
        for format in [
            ExportFormat::Csv,
            ExportFormat::Tsv,
            ExportFormat::Json,
            ExportFormat::JsonLines,
            ExportFormat::Parquet,
            ExportFormat::ArrowIpc,
            ExportFormat::Sql,
            ExportFormat::Markdown,
        ] {
            let mut sortie: Vec<u8> = Vec::new();
            let ecrit = export(
                &tampon,
                format,
                &mut sortie,
                &ExportOptions::default(),
                &CancelToken::new(),
            )
            .is_ok();
            assert_eq!(
                ecrit,
                is_supported(format),
                "{format} : `is_supported` dit {}, `export` dit {ecrit}",
                is_supported(format)
            );
        }
    }

    #[test]
    fn un_resultat_vide_produit_un_fichier_vide_et_non_une_erreur() {
        let tampon = ResultBuffer::new(schema(), 1 << 20);
        tampon.mark_complete(ExecStats::default());
        let mut sortie: Vec<u8> = Vec::new();

        let resume = export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &CancelToken::new(),
        )
        .expect("export sans erreur");

        assert_eq!(resume.rows, 0);
        assert_eq!(resume.batches, 0);
        assert!(sortie.is_empty());
    }
}
