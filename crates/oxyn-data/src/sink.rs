//! Le pont entre un curseur de driver et un [`ResultBuffer`], avec
//! contre-pression et annulation.
//!
//! C'est le mécanisme qui empêche un `SELECT *` sur 500 Go de faire exploser la
//! mémoire ([I-06](../../../CLAUDE.md#i-06)). Il tient en une phrase : **on ne
//! demande le lot suivant au serveur que si le tampon a de la place**. La boucle
//! naïve — tout lire, puis tout ranger — met le débit du réseau en concurrence
//! directe avec la RAM disponible, et le réseau gagne.
//!
//! # Ce que ce puits ne fait pas
//!
//! **Il n'applique pas de délai.** `ExecLimits::timeout` est appliqué par
//! `oxyn-exec`, qui possède le runtime, le journal et le droit d'émettre
//! l'annulation côté serveur ([ARCHITECTURE](../../../docs/ARCHITECTURE.md#9-modèle-dexécution-et-de-threads)).
//! Un délai posé ici n'annulerait que le futur, laissant la requête tourner et
//! le verrou posé côté base — exactement le défaut que le contrat de driver
//! interdit.
//!
//! **Il ne reprend rien après une annulation.** Un futur `next_batch` abandonné
//! peut l'avoir été **après** avoir consommé des octets du flux : le décodeur du
//! driver est alors désynchronisé. Le puits le sait et refuse de reprendre — la
//! source doit être détruite. Un point de reprise se conçoit, il ne s'improvise
//! pas ([rust.md](../../../.claude/rules/rust.md)).

use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use futures::future::{BoxFuture, Either, select};
use oxyn_core::{CancelToken, ExecStats, OxynError};

use crate::buffer::{BatchIndex, Pressure, ResultBuffer};
use crate::error::DataError;

/// Ce qui produit des lots : un curseur de driver, un décodeur de fichier, un
/// générateur de test.
///
/// # Pour `oxyn-driver`
///
/// `Cursor` est écrit avec `#[async_trait]`, qui désucre `async fn next_batch`
/// en exactement la signature ci-dessous. L'adaptation tient donc en :
///
/// ```ignore
/// impl BatchSource for Box<dyn Cursor> {
///     fn schema(&self) -> SchemaRef { (**self).schema() }
///     fn next_batch(&mut self) -> BoxFuture<'_, oxyn_core::Result<Option<RecordBatch>>> {
///         Box::pin((**self).next_batch())
///     }
///     fn stats(&self) -> ExecStats { (**self).stats() }
/// }
/// ```
///
/// Ce trait vit ici plutôt que dans `oxyn-driver` parce que `oxyn-data` ne
/// dépend pas de `oxyn-driver` — c'est l'inverse — et parce qu'un puits doit
/// pouvoir se tester sans driver.
///
/// # Annulation
///
/// [`next_batch`](Self::next_batch) doit être abandonnable. Après abandon, la
/// source est considérée comme inutilisable : voir la note de module.
pub trait BatchSource: Send {
    /// Le schéma des lots. Connu avant le premier lot, c'est ce qui permet à la
    /// grille de dessiner ses colonnes pendant que les lignes arrivent.
    fn schema(&self) -> SchemaRef;

    /// Le lot suivant, ou `None` quand le flux est épuisé.
    ///
    /// Le type de retour est le désucrage d'`async fn` : il garde le trait
    /// compatible avec `dyn`, ce qui est une contrainte dure du workspace
    /// ([ARCHITECTURE §4.1](../../../docs/ARCHITECTURE.md#41-les-traits)).
    fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>>;

    /// Ce que la source sait de l'exécution : temps serveur, lignes, octets.
    ///
    /// Interrogée à la fin du flux, pour clore le tampon.
    fn stats(&self) -> ExecStats {
        ExecStats::default()
    }
}

/// Pourquoi le puits s'est arrêté.
///
/// Seul [`Exhausted`](Self::Exhausted) décrit un résultat entier ; tous les
/// autres cas produisent un résultat **tronqué**, marqué comme tel dans
/// [`ExecStats`] et donc à l'écran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum SinkOutcome {
    /// La source est épuisée : le résultat est complet.
    Exhausted,
    /// [`BufferLimits::max_rows`](crate::BufferLimits) est atteint.
    RowLimit,
    /// Le tampon est saturé : budget mémoire atteint, débordement interdit ou
    /// plafonné.
    Saturated,
    /// Le [`CancelToken`] a été déclenché. **La source ne doit plus servir.**
    Cancelled,
}

impl SinkOutcome {
    /// Le résultat contient-il toutes les lignes de la requête ?
    #[must_use]
    pub const fn is_complete(self) -> bool {
        matches!(self, Self::Exhausted)
    }

    /// Des lignes manquent-elles ?
    #[must_use]
    pub const fn is_truncated(self) -> bool {
        !self.is_complete()
    }
}

/// Ce que le puits vient de ranger, pour l'appelant qui veut rendre la main à
/// l'interface entre deux lots.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct BatchProgress {
    /// Position du lot dans le tampon.
    pub index: BatchIndex,
    /// Lignes de ce lot.
    pub rows: usize,
    /// Lignes disponibles au total après ce lot.
    pub total_rows: usize,
}

/// Draine une [`BatchSource`] dans un [`ResultBuffer`].
#[derive(Debug)]
pub struct BatchSink {
    buffer: Arc<ResultBuffer>,
    /// Un `next_batch` a-t-il été abandonné en vol ?
    ///
    /// Reprendre après cela lirait un flux désynchronisé : le puits refuse.
    aborted: AtomicBool,
}

impl BatchSink {
    /// Nouveau puits alimentant `buffer`.
    #[must_use]
    pub fn new(buffer: Arc<ResultBuffer>) -> Self {
        Self {
            buffer,
            aborted: AtomicBool::new(false),
        }
    }

    /// Le tampon alimenté. Partageable en lecture pendant le drainage.
    #[must_use]
    pub fn buffer(&self) -> &Arc<ResultBuffer> {
        &self.buffer
    }

    /// Draine la source jusqu'à épuisement, saturation ou annulation.
    ///
    /// # Erreurs
    ///
    /// Remonte l'erreur de la source, ou celle du tampon si l'écriture du
    /// fichier de débordement échoue. Une saturation ou une annulation ne sont
    /// **pas** des erreurs : ce sont des [`SinkOutcome`].
    pub async fn drain(
        &self,
        source: &mut dyn BatchSource,
        ct: &CancelToken,
    ) -> Result<SinkOutcome, OxynError> {
        self.drain_with(source, ct, |_| {}).await
    }

    /// Comme [`drain`](Self::drain), en appelant `on_batch` après chaque lot
    /// rangé.
    ///
    /// C'est le crochet dont `oxyn-exec` a besoin pour émettre
    /// [`Event::BatchReady`](oxyn_core::Event) sans attendre la fin du flux —
    /// « la grille s'affiche dès le premier `RecordBatch` ».
    ///
    /// `on_batch` s'exécute **dans** la boucle de drainage : ce qu'on y met
    /// retarde le lot suivant. Y envoyer sur un canal, oui ; y dessiner, non.
    ///
    /// # Erreurs
    ///
    /// Celles de [`drain`](Self::drain), plus [`OxynError::Internal`] si la
    /// source a déjà été abandonnée en vol lors d'un appel précédent.
    pub async fn drain_with<F>(
        &self,
        source: &mut dyn BatchSource,
        ct: &CancelToken,
        mut on_batch: F,
    ) -> Result<SinkOutcome, OxynError>
    where
        F: FnMut(BatchProgress),
    {
        if self.aborted.load(Ordering::SeqCst) {
            return Err(OxynError::Internal(
                "batch source was abandoned mid-flight and cannot be resumed".to_owned(),
            ));
        }

        loop {
            if ct.is_cancelled() {
                self.seal(source, true);
                return Ok(SinkOutcome::Cancelled);
            }

            // Contre-pression : la question au serveur n'est posée que si la
            // réponse a où aller.
            match self.buffer.pressure() {
                Pressure::Ready => {}
                Pressure::RowLimit => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::RowLimit);
                }
                Pressure::Saturated => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::Saturated);
                }
                Pressure::Complete => return Ok(SinkOutcome::Exhausted),
            }

            // Le bloc borne l'emprunt mutable de `source` par le futur de
            // lecture : sans lui, plus rien ne pourrait toucher à la source
            // dans les branches ci-dessous.
            let recu = {
                let lot = pin!(source.next_batch());
                let annulation = pin!(ct.cancelled());
                match select(lot, annulation).await {
                    Either::Left((recu, _)) => Some(recu),
                    Either::Right(((), _)) => None,
                }
            };

            let Some(recu) = recu else {
                // Le futur de lecture vient d'être abandonné, peut-être après
                // avoir consommé des octets du flux : la source est brûlée.
                self.aborted.store(true, Ordering::SeqCst);
                self.seal(source, true);
                return Ok(SinkOutcome::Cancelled);
            };

            let lot = match recu {
                Ok(Some(lot)) => lot,
                Ok(None) => {
                    self.seal(source, false);
                    return Ok(SinkOutcome::Exhausted);
                }
                Err(erreur) if erreur.is_cancelled() => {
                    self.seal(source, true);
                    return Ok(SinkOutcome::Cancelled);
                }
                Err(erreur) => {
                    // Les lignes déjà reçues restent lisibles ; le tampon est
                    // clos pour que l'interface cesse d'attendre la suite.
                    self.seal(source, true);
                    return Err(erreur);
                }
            };

            match self.buffer.push(lot) {
                // Lot vide : la source a le droit d'en produire, il n'y a rien
                // à signaler.
                Ok(None) => {}
                Ok(Some(index)) => {
                    // Relu depuis le tampon, jamais depuis le lot poussé :
                    // c'est le tampon qui décide combien de lignes il garde.
                    let rows = self.buffer.batch_rows(index).unwrap_or(0);
                    let total_rows = self.buffer.row_count();
                    on_batch(BatchProgress {
                        index,
                        rows,
                        total_rows,
                    });
                }
                Err(DataError::Full { .. }) => {
                    let issue = match self.buffer.pressure() {
                        Pressure::RowLimit => SinkOutcome::RowLimit,
                        _ => SinkOutcome::Saturated,
                    };
                    self.seal(source, true);
                    return Ok(issue);
                }
                Err(autre) => {
                    self.seal(source, true);
                    return Err(autre.into());
                }
            }
        }
    }

    /// Clôt le tampon avec les mesures de la source.
    fn seal(&self, source: &dyn BatchSource, truncated: bool) {
        if truncated {
            self.buffer.mark_truncated();
        }
        self.buffer.mark_complete(source.stats());
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};
    use futures::executor::block_on;

    use super::*;
    use crate::buffer::BufferLimits;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]))
    }

    fn lot(lignes: usize) -> RecordBatch {
        let valeurs: Vec<i32> = (0..lignes)
            .map(|i| i32::try_from(i).unwrap_or(i32::MAX))
            .collect();
        RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(valeurs))])
            .expect("la colonne correspond au schéma construit juste au-dessus")
    }

    /// Source scriptée qui compte combien de fois on lui a demandé un lot :
    /// c'est **ce compteur** qui prouve la contre-pression, pas le contenu du
    /// tampon.
    #[derive(Debug)]
    struct SourceScriptee {
        restants: Vec<RecordBatch>,
        appels: usize,
        erreur: Option<OxynError>,
    }

    impl SourceScriptee {
        fn new(lots: Vec<RecordBatch>) -> Self {
            let mut restants = lots;
            restants.reverse();
            Self {
                restants,
                appels: 0,
                erreur: None,
            }
        }

        fn qui_echoue(erreur: OxynError) -> Self {
            Self {
                restants: Vec::new(),
                appels: 0,
                erreur: Some(erreur),
            }
        }
    }

    impl BatchSource for SourceScriptee {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>> {
            self.appels = self.appels.saturating_add(1);
            if let Some(erreur) = self.erreur.take() {
                return Box::pin(async move { Err(erreur) });
            }
            let suivant = self.restants.pop();
            Box::pin(async move { Ok(suivant) })
        }

        fn stats(&self) -> ExecStats {
            ExecStats {
                batches: u64::try_from(self.appels).unwrap_or(u64::MAX),
                ..ExecStats::default()
            }
        }
    }

    #[test]
    fn une_source_epuisee_clot_le_resultat() {
        let tampon = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::new(vec![lot(10), lot(10), lot(5)]);
        let ct = CancelToken::new();

        let issue = block_on(puits.drain(&mut source, &ct)).expect("drainage sans erreur");

        assert_eq!(issue, SinkOutcome::Exhausted);
        assert!(issue.is_complete());
        assert_eq!(tampon.row_count(), 25);
        assert!(tampon.is_complete());
        assert!(!tampon.stats().truncated);
    }

    #[test]
    fn chaque_lot_est_signale_des_son_arrivee() {
        let tampon = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::new(vec![lot(3), lot(4)]);
        let ct = CancelToken::new();

        let mut vus = Vec::new();
        block_on(puits.drain_with(&mut source, &ct, |progres| vus.push(progres)))
            .expect("drainage sans erreur");

        assert_eq!(vus.len(), 2);
        assert_eq!(vus.first().map(|p| (p.rows, p.total_rows)), Some((3, 3)));
        assert_eq!(vus.get(1).map(|p| (p.rows, p.total_rows)), Some((4, 7)));
    }

    /// Le test qui porte la promesse : quand le tampon refuse, **on ne demande
    /// pas** le lot suivant au serveur.
    #[test]
    fn un_tampon_sature_arrete_de_demander_des_lots() {
        let tampon = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .without_spill(),
        ));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::new(vec![lot(10); 50]);
        let ct = CancelToken::new();

        let issue = block_on(puits.drain(&mut source, &ct)).expect("drainage sans erreur");

        assert_eq!(issue, SinkOutcome::Saturated);
        assert!(issue.is_truncated());
        assert_eq!(
            source.appels, 1,
            "un seul lot demandé : le refus doit remonter avant la demande suivante"
        );
        assert!(tampon.stats().truncated);
        assert!(tampon.is_complete(), "l'interface doit cesser d'attendre");
    }

    #[test]
    fn la_limite_de_lignes_arrete_le_drainage() {
        let tampon = Arc::new(ResultBuffer::with_limits(
            schema(),
            BufferLimits::default().with_max_rows(15_usize),
        ));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::new(vec![lot(10); 20]);
        let ct = CancelToken::new();

        let issue = block_on(puits.drain(&mut source, &ct)).expect("drainage sans erreur");

        assert_eq!(issue, SinkOutcome::RowLimit);
        assert_eq!(tampon.row_count(), 15);
        assert!(tampon.stats().truncated);
        assert_eq!(
            source.appels, 2,
            "deux lots suffisent à atteindre 15 lignes ; le troisième ne doit pas être demandé"
        );
    }

    #[test]
    fn une_annulation_prealable_ne_demande_aucun_lot() {
        let tampon = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::new(vec![lot(10)]);
        let ct = CancelToken::new();
        ct.cancel();

        let issue = block_on(puits.drain(&mut source, &ct)).expect("drainage sans erreur");

        assert_eq!(issue, SinkOutcome::Cancelled);
        assert_eq!(source.appels, 0);
        assert!(tampon.is_empty());
        assert!(tampon.stats().truncated);
    }

    /// Source qui ne répond jamais, et qui annule au premier sondage : c'est le
    /// serveur qui ne rend pas la main pendant que l'utilisateur tape `Échap`.
    #[derive(Debug)]
    struct SourceMuette {
        ct: CancelToken,
    }

    impl BatchSource for SourceMuette {
        fn schema(&self) -> SchemaRef {
            schema()
        }

        fn next_batch(&mut self) -> BoxFuture<'_, Result<Option<RecordBatch>, OxynError>> {
            let ct = self.ct.clone();
            Box::pin(async move {
                ct.cancel();
                std::future::pending::<Result<Option<RecordBatch>, OxynError>>().await
            })
        }
    }

    #[test]
    fn une_annulation_en_vol_brule_la_source() {
        let tampon = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let ct = CancelToken::new();
        let mut source = SourceMuette { ct: ct.clone() };

        let issue = block_on(puits.drain(&mut source, &ct)).expect("drainage sans erreur");
        assert_eq!(issue, SinkOutcome::Cancelled);

        // Reprendre lirait un flux dont le décodeur est peut-être désynchronisé.
        let ct2 = CancelToken::new();
        let reprise = block_on(puits.drain(&mut source, &ct2));
        assert!(reprise.is_err(), "la reprise doit être refusée");
    }

    #[test]
    fn une_erreur_de_source_remonte_mais_clot_le_tampon() {
        let tampon = Arc::new(ResultBuffer::new(schema(), 1 << 20));
        let puits = BatchSink::new(Arc::clone(&tampon));
        let mut source = SourceScriptee::qui_echoue(OxynError::Query("boom".to_owned()));
        let ct = CancelToken::new();

        let erreur = block_on(puits.drain(&mut source, &ct)).expect_err("l'erreur doit remonter");
        assert!(matches!(erreur, OxynError::Query(_)));
        assert!(
            tampon.is_complete(),
            "sans clôture, l'interface attend un lot qui ne viendra pas"
        );
        assert!(tampon.stats().truncated);
    }
}
