//! Les tampons de résultats d'Oxyn.
//!
//! « Premier affichage sous 100 ms, mémoire stable sur 10 M de lignes » se tient
//! ou se perd ici. Cette crate est la mise en œuvre d'[ADR-0002] : un driver
//! produit des [`RecordBatch`](arrow::record_batch::RecordBatch) Arrow, et plus
//! rien ne les reconvertit jusqu'à l'écran ou l'export.
//!
//! | Module | Sujet | Autorité |
//! |---|---|---|
//! | [`buffer`] | accumulation bornée, débordement disque, `locate` en O(log n) | ADR-0002, PERFORMANCE |
//! | [`cell`] | rendu d'une cellule pour la grille | UX-SPEC |
//! | [`sink`] | contre-pression entre un curseur et un tampon | ARCHITECTURE §9 |
//! | [`mod@export`] | CSV, TSV, JSON, JSON par lignes, Arrow IPC | I-11 |
//! | [`error`] | la frontière d'erreurs de la couche | rust.md |
//!
//! # Le chemin complet
//!
//! ```no_run
//! use std::sync::Arc;
//! use oxyn_core::CancelToken;
//! use oxyn_data::{BatchSink, BatchSource, ResultBuffer, format_cell, FormatOptions};
//!
//! # async fn exemple(mut curseur: Box<dyn BatchSource>) -> Result<(), Box<dyn std::error::Error>> {
//! // Le tampon est créé dès que le schéma est connu : la grille dessine ses
//! // colonnes avant qu'une seule ligne n'arrive.
//! let tampon = Arc::new(ResultBuffer::new(curseur.schema(), 256 * 1024 * 1024));
//! let puits = BatchSink::new(Arc::clone(&tampon));
//! let annulation = CancelToken::new();
//!
//! // La tâche draine ; l'interface lit le même `Arc` sans jamais l'attendre.
//! let issue = puits.drain(curseur.as_mut(), &annulation).await?;
//!
//! let options = FormatOptions::default();
//! if let Some((position, decalage)) = tampon.locate(0) {
//!     if let Some(lot) = tampon.batch(position)? {
//!         let cellule = format_cell(&lot, decalage, 0, &options);
//!         println!("{}", cellule.display_with(&options));
//!     }
//! }
//! # let _ = issue;
//! # Ok(())
//! # }
//! ```
//!
//! # Local page storage
//!
//! [ADR-0012] specifies positioned reads of autonomous Arrow IPC streams,
//! off the UI thread. `cached_batch` never performs I/O or waits for a lock;
//! `load_page` populates the byte-bounded cache with cooperative cancellation.
//! Initial batches and decoded pages share one retention budget. Decoder
//! temporaries and clones held by readers are separate from cache retention.
//!
//! [ADR-0012]: ../../../docs/adr/0012-lecture-pages-resultats.md
//! [ADR-0002]: ../../../docs/adr/0002-arrow-result-model.md

pub mod buffer;
pub mod cell;
pub mod error;
pub mod export;
pub mod sink;
mod spill;
pub mod value_page;

pub use buffer::{BatchIndex, BufferLimits, DEFAULT_MEMORY_BUDGET, Pressure, ResultBuffer};
pub use cell::{
    BinaryDisplay, CellValue, DEFAULT_MAX_LEN, FormatOptions, GROUP_SEPARATOR, NumberGrouping,
    format_cell, format_value,
};
pub use error::{DataError, Result};
pub use export::{ExportOptions, ExportSummary, export, is_supported};
pub use sink::{BatchProgress, BatchSink, BatchSource, SinkOutcome};

/// Ce qu'on importe d'un coup quand on travaille avec des résultats.
pub mod prelude {
    pub use crate::buffer::{BatchIndex, BufferLimits, Pressure, ResultBuffer};
    pub use crate::cell::{BinaryDisplay, CellValue, FormatOptions, NumberGrouping, format_cell};
    pub use crate::error::{DataError, Result};
    pub use crate::export::{ExportOptions, ExportSummary, export, is_supported};
    pub use crate::sink::{BatchProgress, BatchSink, BatchSource, SinkOutcome};
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::{Int32Array, StringArray};
    use arrow::datatypes::{DataType, Field, Schema, SchemaRef};
    use arrow::record_batch::RecordBatch;
    use futures::executor::block_on;
    use futures::future::BoxFuture;
    use oxyn_core::{CancelToken, ExecStats, ExportFormat, OxynError};

    use crate::prelude::*;

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
        let noms: Vec<Option<String>> = ids.iter().map(|i| Some(format!("n{i}"))).collect();
        RecordBatch::try_new(
            schema(),
            vec![
                Arc::new(Int32Array::from(ids)),
                Arc::new(StringArray::from(noms)),
            ],
        )
        .expect("les colonnes correspondent au schéma construit juste au-dessus")
    }

    /// Curseur simulé : beaucoup de lots, aucun ne tenant dans le budget.
    #[derive(Debug)]
    struct Curseur {
        restants: usize,
    }

    impl BatchSource for Curseur {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(
            &mut self,
        ) -> BoxFuture<'_, std::result::Result<Option<RecordBatch>, OxynError>> {
            let rang = self.restants;
            self.restants = self.restants.saturating_sub(1);
            Box::pin(async move {
                if rang == 0 {
                    return Ok(None);
                }
                let depart = i32::try_from(rang).unwrap_or(i32::MAX).saturating_mul(100);
                Ok(Some(lot(depart, 50)))
            })
        }

        fn stats(&self) -> ExecStats {
            ExecStats::default()
        }
    }

    /// Le trajet complet de la crate, avec un budget assez petit pour forcer le
    /// débordement : drainage, localisation, rendu, export.
    ///
    /// C'est la version réduite du critère de sortie de la phase 0 : la mémoire
    /// reste bornée, rien n'est perdu, et ce qui s'affiche est ce qui s'exporte.
    #[test]
    fn le_trajet_complet_tient_dans_un_budget_minuscule() {
        let tampon = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_memory_budget(2_048),
        ));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut curseur = Curseur { restants: 40 };
        let annulation = CancelToken::new();

        let issue = block_on(puits.drain(&mut curseur, &annulation)).expect("drainage");
        assert_eq!(issue, SinkOutcome::Exhausted);
        assert!(issue.is_complete());
        assert_eq!(tampon.row_count(), 40 * 50);
        assert!(
            tampon.spilled_batches() > 0,
            "le budget doit avoir été dépassé"
        );
        // L'invariant de mémoire, en une ligne : ce qui reste résident ne
        // dépasse jamais le budget, quel que soit le volume traversé.
        assert!(
            tampon.resident_bytes() <= 2_048,
            "{} octets résidents pour un budget de 2 048",
            tampon.resident_bytes()
        );

        // Une ligne prise loin dans le résultat se relit sans réexécution.
        let options = FormatOptions::default();
        let (position, decalage) = tampon.locate(1_999).expect("la ligne existe");
        let lot = tampon.batch(position).expect("relecture").expect("le lot");
        let cellule = format_cell(&lot, decalage, 1, &options);
        assert!(
            cellule.text().is_some_and(|t| t.starts_with('n')),
            "{cellule:?}"
        );

        // Et ce qui s'affiche est ce qui s'exporte.
        let mut sortie: Vec<u8> = Vec::new();
        let resume = export(
            &tampon,
            ExportFormat::Csv,
            &mut sortie,
            &ExportOptions::default(),
            &annulation,
        )
        .expect("export");
        assert_eq!(resume.rows, 2_000);

        let texte = String::from_utf8(sortie).expect("CSV en UTF-8");
        let attendu = cellule.text().unwrap_or_default();
        assert!(texte.contains(attendu), "{attendu} absent de l'export");
    }
}
