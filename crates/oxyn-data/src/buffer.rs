//! Le tampon de résultats : là où « premier affichage sous 100 ms, mémoire
//! stable sur 10 M de lignes » se tient ou se perd.
//!
//! Un [`ResultBuffer`] accumule des [`RecordBatch`] dans un budget mémoire
//! ([`BufferLimits::memory_budget`], 256 Mo par défaut). Au-delà, les lots
//! partent dans un fichier Arrow IPC temporaire et sont relus à la demande
//! (module interne `spill`). Le tampon est fait pour être partagé en
//! `Arc<ResultBuffer>` : le puits pousse, la grille lit, aucun des deux
//! n'attend l'autre plus longtemps qu'un verrou d'index.
//!
//! # Les trois décisions qui gouvernent ce fichier
//!
//! **`locate` est le chemin chaud.** La grille l'appelle une fois par cellule
//! dessinée. Il ne fait qu'un verrou de lecture et une recherche binaire sur les
//! décalages cumulés — pas d'allocation, pas d'accès disque.
//!
//! **Aucune écriture disque sous le verrou d'index.** `push` décide sous un
//! verrou de lecture, écrit sur le disque **sans verrou**, puis publie sous un
//! verrou d'écriture tenu quelques microsecondes. Autrement une écriture de lot
//! de 30 ms figerait le défilement, en violation du budget de trame de 8 ms
//! ([PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-dinteraction),
//! [I-05](../../../CLAUDE.md#i-05)).
//!
//! **Le budget est strict, y compris pour le premier lot.** Garder « au moins un
//! lot » en mémoire quoi qu'il arrive serait plus commode à l'affichage, mais
//! promettrait un plafond qu'on ne tient pas : un unique lot de 2 Go tiendrait
//! dans un budget de 256 Mo.

use std::sync::Arc;

use arrow::datatypes::SchemaRef;
use arrow::record_batch::RecordBatch;
use oxyn_core::ExecStats;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};

use crate::error::{DataError, Result};
use crate::spill::{SpillCache, SpillFile, SpillRef};
use oxyn_core::CancelToken;

/// Budget mémoire par défaut d'un résultat, en octets.
///
/// 256 Mo, décidé par [ADR-0002](../../../docs/adr/0002-arrow-result-model.md).
pub const DEFAULT_MEMORY_BUDGET: usize = 256 * 1024 * 1024;

/// Position d'un lot dans un [`ResultBuffer`].
///
/// Un entier nu se confondrait avec un numéro de ligne, et les deux se croisent
/// dans chaque signature de ce module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct BatchIndex(usize);

impl BatchIndex {
    /// Construit une position.
    #[must_use]
    pub const fn new(position: usize) -> Self {
        Self(position)
    }

    /// La position, telle qu'elle indexe la suite des lots.
    #[must_use]
    pub const fn get(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for BatchIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Les bornes d'un tampon de résultats.
///
/// Toutes sont des protections, pas des réglages de confort : chacune correspond
/// à un mode de panne décrit dans
/// [PERFORMANCE](../../../docs/PERFORMANCE.md#budgets-de-mémoire).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct BufferLimits {
    /// Retention budget shared between initial batches and rehydrated pages.
    pub memory_budget: usize,
    /// Lignes au-delà desquelles le résultat est tronqué, et déclaré tel.
    ///
    /// Miroir d'[`ExecLimits::max_rows`](oxyn_core::ExecLimits) : le driver est
    /// censé l'appliquer, le tampon ne le suppose pas.
    pub max_rows: Option<usize>,
    /// Octets de débordement au-delà desquels le tampon refuse d'écrire.
    ///
    /// Sans plafond, un `SELECT *` sur 500 Go remplit le disque de l'utilisateur
    /// — panne plus désagréable que la troncature qu'il évite.
    pub max_spill_bytes: Option<u64>,
    /// Le débordement disque est-il autorisé ?
    ///
    /// `false` transforme le dépassement du budget en contre-pression franche :
    /// c'est ce que veut un export en flux, qui n'a rien à garder.
    pub allow_spill: bool,
}

impl Default for BufferLimits {
    fn default() -> Self {
        Self {
            memory_budget: DEFAULT_MEMORY_BUDGET,
            max_rows: None,
            max_spill_bytes: None,
            allow_spill: true,
        }
    }
}

impl BufferLimits {
    fn cache_budget(&self) -> usize {
        if self.allow_spill {
            self.memory_budget / 4
        } else {
            0
        }
    }
    fn resident_budget(&self) -> usize {
        self.memory_budget.saturating_sub(self.cache_budget())
    }

    /// Bornes par défaut avec un budget mémoire choisi.
    #[must_use]
    pub fn with_memory_budget(mut self, octets: usize) -> Self {
        self.memory_budget = octets;
        self
    }

    /// Borne le nombre de lignes.
    #[must_use]
    pub fn with_max_rows(mut self, lignes: impl Into<Option<usize>>) -> Self {
        self.max_rows = lignes.into();
        self
    }

    /// Borne le débordement disque.
    #[must_use]
    pub fn with_max_spill_bytes(mut self, octets: impl Into<Option<u64>>) -> Self {
        self.max_spill_bytes = octets.into();
        self
    }

    /// Interdit le débordement disque.
    #[must_use]
    pub fn without_spill(mut self) -> Self {
        self.allow_spill = false;
        self
    }
}

/// Pourquoi le tampon accepte, ou n'accepte plus, un lot de plus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Pressure {
    /// Le tampon prend un lot de plus.
    Ready,
    /// [`BufferLimits::max_rows`] est atteint : le résultat sera tronqué.
    RowLimit,
    /// Le budget mémoire est atteint et le débordement est interdit ou plafonné.
    Saturated,
    /// Le résultat est clos ; plus rien ne s'y ajoute.
    Complete,
}

impl Pressure {
    /// Le tampon prend-il un lot de plus ?
    #[must_use]
    pub const fn accepts(self) -> bool {
        matches!(self, Self::Ready)
    }
}

/// Où vit un lot.
#[derive(Debug)]
enum Slot {
    /// En mémoire. Le clone est un clone d'`Arc`, pas de données.
    Resident(RecordBatch),
    /// Dans le fichier de débordement.
    Spilled(SpillRef),
}

/// L'état mutable du tampon, protégé par un `RwLock`.
///
/// Tout ce qui est ici se lit en O(1) ou O(log n) et ne touche pas au disque :
/// c'est la condition pour que le thread d'interface prenne ce verrou.
#[derive(Debug)]
struct BufferIndex {
    /// `starts[i]` = numéro de la première ligne du lot `i`. Strictement
    /// croissant : les lots vides ne sont jamais enregistrés.
    starts: Vec<usize>,
    slots: Vec<Slot>,
    rows: usize,
    resident_bytes: usize,
    spilled_bytes: u64,
    spilled_batches: usize,
    complete: bool,
    stats: ExecStats,
}

/// Ce que `push` a décidé avant de toucher au disque.
#[derive(Debug, Clone, Copy)]
struct PushPlan {
    /// Lignes à conserver ; `None` = le lot entier.
    keep_rows: Option<usize>,
    /// Le lot a-t-il été raboté par la limite de lignes ?
    truncated: bool,
    /// Faut-il écrire ce lot sur le disque ?
    spill: bool,
}

impl BufferIndex {
    fn new() -> Self {
        Self {
            starts: Vec::new(),
            slots: Vec::new(),
            rows: 0,
            resident_bytes: 0,
            spilled_bytes: 0,
            spilled_batches: 0,
            complete: false,
            stats: ExecStats::default(),
        }
    }

    /// Décide du sort d'un lot entrant, sans rien modifier.
    fn plan(&self, limits: &BufferLimits, rows: usize, bytes: usize) -> Result<PushPlan> {
        if self.complete {
            return Err(DataError::AlreadyComplete);
        }

        let (keep_rows, truncated) = match limits.max_rows {
            Some(max) => {
                let reste = max.saturating_sub(self.rows);
                if reste == 0 {
                    return Err(DataError::Full {
                        reason: "row limit reached",
                    });
                }
                if rows > reste {
                    (Some(reste), true)
                } else {
                    (None, false)
                }
            }
            None => (None, false),
        };

        let spill = self.resident_bytes.saturating_add(bytes) > limits.memory_budget;
        if spill {
            if !limits.allow_spill {
                return Err(DataError::Full {
                    reason: "memory budget reached and spilling is disabled",
                });
            }
            if let Some(quota) = limits.max_spill_bytes {
                let projete = self
                    .spilled_bytes
                    .saturating_add(u64::try_from(bytes).unwrap_or(u64::MAX));
                if projete > quota {
                    return Err(DataError::Full {
                        reason: "spill quota reached",
                    });
                }
            }
        }

        Ok(PushPlan {
            keep_rows,
            truncated,
            spill,
        })
    }

    /// Publie un lot déjà placé (en mémoire ou sur le disque).
    fn append(&mut self, slot: Slot, rows: usize, bytes: usize) -> BatchIndex {
        let position = self.slots.len();
        self.starts.push(self.rows);
        match &slot {
            Slot::Resident(_) => self.resident_bytes = self.resident_bytes.saturating_add(bytes),
            Slot::Spilled(reference) => {
                self.spilled_bytes = self.spilled_bytes.saturating_add(reference.byte_len());
                self.spilled_batches = self.spilled_batches.saturating_add(1);
            }
        }
        self.slots.push(slot);
        self.rows = self.rows.saturating_add(rows);
        BatchIndex(position)
    }
}

/// Les lots d'un résultat, bornés en mémoire, débordant sur disque, partageables
/// en lecture.
///
/// Toutes les méthodes prennent `&self` : le tampon vit derrière un
/// `Arc<ResultBuffer>`, un producteur unique appelle [`push`](Self::push), et un
/// nombre quelconque de lecteurs appellent [`locate`](Self::locate) et
/// [`batch`](Self::batch) en parallèle.
///
/// # Un seul producteur
///
/// [`push`](Self::push) est sûr à appeler depuis plusieurs threads — l'index
/// reste cohérent —, mais l'ordre des lignes suit alors l'ordre d'arrivée, qui
/// n'est plus celui du curseur. Le pipeline d'Oxyn n'a qu'un producteur par
/// résultat ([`BatchSink`](crate::BatchSink)) ; ce n'est pas une supposition
/// d'implémentation, c'est la définition d'un résultat ordonné.
pub struct ResultBuffer {
    schema: SchemaRef,
    limits: BufferLimits,
    index: RwLock<BufferIndex>,
    /// Créé paresseusement : la grande majorité des résultats ne déborde jamais,
    /// et un fichier temporaire par requête serait un coût pur.
    spill: Mutex<Option<Arc<SpillFile>>>,
    cache: Mutex<SpillCache>,
}

impl ResultBuffer {
    /// Nouveau tampon pour `schema`, avec un budget mémoire en octets.
    ///
    /// Pour régler autre chose que le budget, voir
    /// [`with_limits`](Self::with_limits).
    #[must_use]
    pub fn new(schema: SchemaRef, budget: usize) -> Self {
        Self::with_limits(schema, BufferLimits::default().with_memory_budget(budget))
    }

    /// Nouveau tampon avec toutes ses bornes.
    #[must_use]
    pub fn with_limits(schema: SchemaRef, limits: BufferLimits) -> Self {
        Self {
            schema,
            limits,
            index: RwLock::new(BufferIndex::new()),
            spill: Mutex::new(None),
            cache: Mutex::new(SpillCache::new(limits.cache_budget())),
        }
    }

    /// Le schéma des lots. Fixé à la construction, il ne change jamais.
    #[must_use]
    pub fn schema(&self) -> &SchemaRef {
        &self.schema
    }

    /// Les bornes appliquées.
    #[must_use]
    pub fn limits(&self) -> &BufferLimits {
        &self.limits
    }

    /// Lignes disponibles à cet instant.
    ///
    /// Croît tant que le résultat n'est pas [complet](Self::is_complete) : la
    /// grille doit relire cette valeur, pas la mémoriser.
    #[must_use]
    pub fn row_count(&self) -> usize {
        self.index.read().rows
    }

    /// Lots enregistrés à cet instant.
    #[must_use]
    pub fn batch_count(&self) -> usize {
        self.index.read().slots.len()
    }

    /// Aucune ligne reçue.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.row_count() == 0
    }

    /// Le flux est-il terminé ?
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.index.read().complete
    }

    /// Volumétrie et temps de l'exécution.
    ///
    /// Tant que le résultat n'est pas complet, seuls les compteurs alimentés par
    /// [`push`](Self::push) sont renseignés ; les temps arrivent avec
    /// [`mark_complete`](Self::mark_complete).
    #[must_use]
    pub fn stats(&self) -> ExecStats {
        self.index.read().stats
    }

    /// Octets de lots gardés en mémoire.
    #[must_use]
    pub fn resident_bytes(&self) -> usize {
        self.index.read().resident_bytes
    }

    /// Octets écrits dans le fichier de débordement.
    #[must_use]
    pub fn spilled_bytes(&self) -> u64 {
        self.index.read().spilled_bytes
    }

    /// Lots partis sur le disque.
    #[must_use]
    pub fn spilled_batches(&self) -> usize {
        self.index.read().spilled_batches
    }

    /// Le tampon accepterait-il un lot de plus, et sinon pourquoi ?
    ///
    /// C'est la question que pose [`BatchSink`](crate::BatchSink) **avant** de
    /// demander le lot suivant au curseur : la contre-pression consiste
    /// exactement à ne pas poser la question au serveur quand la réponse n'a
    /// nulle part où aller.
    ///
    /// La raison compte autant que la réponse : « limite de lignes atteinte »
    /// est un résultat tronqué normal, « budget saturé » est une condition que
    /// l'utilisateur peut lever en réglant le tampon.
    #[must_use]
    pub fn pressure(&self) -> Pressure {
        let index = self.index.read();
        if index.complete {
            return Pressure::Complete;
        }
        if let Some(max) = self.limits.max_rows
            && index.rows >= max
        {
            return Pressure::RowLimit;
        }
        if index.resident_bytes >= self.limits.resident_budget() {
            if !self.limits.allow_spill {
                return Pressure::Saturated;
            }
            if let Some(quota) = self.limits.max_spill_bytes
                && index.spilled_bytes >= quota
            {
                return Pressure::Saturated;
            }
        }
        Pressure::Ready
    }

    /// Le tampon accepterait-il un lot de plus ?
    ///
    /// Raccourci sur [`pressure`](Self::pressure) pour les appelants qui n'ont
    /// pas besoin de la raison.
    #[must_use]
    pub fn has_capacity(&self) -> bool {
        matches!(self.pressure(), Pressure::Ready)
    }

    /// Lignes encore acceptées avant la limite, `None` s'il n'y en a pas.
    #[must_use]
    pub fn remaining_rows(&self) -> Option<usize> {
        let max = self.limits.max_rows?;
        Some(max.saturating_sub(self.index.read().rows))
    }

    /// Ajoute un lot.
    ///
    /// Rend `None` pour un lot vide, qui n'est pas une erreur : un curseur peut
    /// en produire en fin de flux, et l'enregistrer casserait la stricte
    /// croissance des décalages sur laquelle repose [`locate`](Self::locate).
    ///
    /// Le lot est raboté si [`BufferLimits::max_rows`] l'exige, et le résultat
    /// est alors marqué tronqué. Il part sur le disque si le budget mémoire est
    /// dépassé.
    ///
    /// **Bloque** le temps d'une écriture disque en cas de débordement : à
    /// n'appeler que depuis une tâche, jamais depuis le thread d'interface
    /// ([I-05](../../../CLAUDE.md#i-05)).
    ///
    /// # Erreurs
    ///
    /// [`DataError::SchemaMismatch`] si le lot ne porte pas le schéma du tampon,
    /// [`DataError::AlreadyComplete`] après [`mark_complete`](Self::mark_complete),
    /// [`DataError::Full`] si une borne est atteinte, [`DataError::Spill`] si le
    /// fichier temporaire refuse l'écriture.
    pub fn push(&self, batch: RecordBatch) -> Result<Option<BatchIndex>> {
        if batch.num_rows() == 0 {
            return Ok(None);
        }
        self.check_schema(&batch)?;

        // Phase 1 — décider. Verrou de lecture, relâché immédiatement.
        let plan = {
            let index = self.index.read();
            index.plan(
                &BufferLimits {
                    memory_budget: self.limits.resident_budget(),
                    ..self.limits
                },
                batch.num_rows(),
                batch.get_array_memory_size(),
            )?
        };

        let batch = match plan.keep_rows {
            // Une tranche Arrow partage les tampons de son parent : elle ne
            // libère pas de mémoire, elle ne fait que borner ce qu'on publie.
            Some(lignes) => batch.slice(0, lignes),
            None => batch,
        };
        let rows = batch.num_rows();
        let bytes = batch.get_array_memory_size();

        // Phase 2 — placer. Hors de tout verrou d'index : c'est ici que se joue
        // le budget de trame du défilement.
        let slot = if plan.spill {
            let fichier = self.spill_file()?;
            Slot::Spilled(fichier.append(&self.schema, &batch)?)
        } else {
            Slot::Resident(batch)
        };

        // Phase 3 — publier. Quelques microsecondes de verrou d'écriture.
        let mut index = self.index.write();
        if index.complete {
            return Err(DataError::AlreadyComplete);
        }
        let position = index.append(slot, rows, bytes);
        index.stats.record_batch(
            u64::try_from(rows).unwrap_or(u64::MAX),
            u64::try_from(bytes).unwrap_or(u64::MAX),
        );
        if plan.truncated {
            index.stats.mark_truncated();
        }
        Ok(Some(position))
    }

    /// Déclare le flux terminé et enregistre les mesures de l'exécution.
    ///
    /// Les mesures de l'appelant l'emportent — lui seul connaît le temps
    /// serveur, le temps total et, pour une écriture, le nombre de lignes
    /// *affectées*, qui n'a rien à voir avec le nombre de lignes reçues. Les
    /// compteurs accumulés par [`push`](Self::push) ne comblent que ce que
    /// l'appelant a laissé à zéro.
    ///
    /// Une exception : `truncated` **ne se retire jamais**. Un driver qui ignore
    /// avoir tronqué ne doit pas pouvoir effacer une troncature constatée ici ;
    /// l'indicateur remonte jusqu'à l'écran, et un résultat tronqué qui a l'air
    /// complet conduit à des conclusions fausses sur des données réelles.
    ///
    /// Idempotent : un second appel remplace les mesures sans rien casser.
    pub fn mark_complete(&self, stats: ExecStats) {
        let mut index = self.index.write();
        let compte = index.stats;
        index.stats = stats;
        if index.stats.rows == 0 {
            index.stats.rows = compte.rows;
        }
        if index.stats.bytes == 0 {
            index.stats.bytes = compte.bytes;
        }
        if index.stats.batches == 0 {
            index.stats.batches = compte.batches;
        }
        if compte.truncated {
            index.stats.mark_truncated();
        }
        index.complete = true;
    }

    /// Marque le résultat comme tronqué, sans le clore.
    ///
    /// Appelé par [`BatchSink`](crate::BatchSink) quand il s'arrête avant la fin
    /// du curseur.
    pub fn mark_truncated(&self) {
        self.index.write().stats.mark_truncated();
    }

    /// Trouve le lot et le décalage local d'une ligne globale.
    ///
    /// **C'est le chemin chaud du produit** : la grille l'appelle pour chaque
    /// cellule dessinée. Un verrou de lecture, une recherche binaire sur les
    /// décalages cumulés, aucune allocation, aucun accès disque.
    ///
    /// Rend `None` si `row` dépasse ce qui est reçu à cet instant — ce qui
    /// arrive normalement pendant un flux, et ne doit pas être traité comme une
    /// erreur.
    #[must_use]
    pub fn locate(&self, row: usize) -> Option<(BatchIndex, usize)> {
        let index = self.index.read();
        if row >= index.rows {
            return None;
        }
        // `starts` commence à 0 et croît strictement : `partition_point` rend
        // donc au moins 1 dès que `row >= 0`, et le `checked_sub` ne peut pas
        // échouer — il est là pour que l'invariant soit vérifié plutôt que
        // supposé ([I-09](../../../CLAUDE.md#i-09)).
        let position = index
            .starts
            .partition_point(|debut| *debut <= row)
            .checked_sub(1)?;
        let debut = *index.starts.get(position)?;
        Some((BatchIndex(position), row.saturating_sub(debut)))
    }

    /// Le lot à cette position.
    ///
    /// Rend `None` si la position n'existe pas encore. **Peut lire le disque**
    /// si le lot a débordé : voir [`is_resident`](Self::is_resident) avant de
    /// l'appeler depuis un chemin qui ne peut pas se permettre d'attendre.
    ///
    /// # Erreurs
    ///
    /// [`DataError::Spill`] ou [`DataError::Arrow`] si la relecture échoue.
    pub fn batch(&self, position: BatchIndex) -> Result<Option<RecordBatch>> {
        self.batch_cancellable(position, &CancelToken::new())
    }

    fn batch_cancellable(
        &self,
        position: BatchIndex,
        cancel: &CancelToken,
    ) -> Result<Option<RecordBatch>> {
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        let reference = {
            let index = self.index.read();
            match index.slots.get(position.get()) {
                None => return Ok(None),
                Some(Slot::Resident(lot)) => return Ok(Some(lot.clone())),
                Some(Slot::Spilled(reference)) => *reference,
            }
        };

        if let Some(lot) = self.cache.lock().get(position.get()) {
            return Ok(Some(lot));
        }

        let fichier = self.spill.lock().clone();
        let Some(fichier) = fichier else {
            // Le fichier n'existe pas alors qu'un lot s'y dit rangé : invariant
            // interne rompu, jamais une entrée du serveur.
            return Err(DataError::Spill(std::io::Error::other(
                "spill file is missing while a batch claims to live in it",
            )));
        };

        let lot = fichier.read_cancellable(reference, cancel)?;
        self.check_schema(&lot)?;
        let mut cache = self.cache.lock();
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        cache.insert(position.get(), lot.clone());
        Ok(Some(lot))
    }

    /// Returns an already resident or cached batch without I/O or waiting for a lock.
    #[must_use]
    pub fn cached_batch(&self, position: BatchIndex) -> Option<RecordBatch> {
        {
            let index = self.index.try_read()?;
            match index.slots.get(position.get())? {
                Slot::Resident(batch) => return Some(batch.clone()),
                Slot::Spilled(_) => {}
            }
        }
        self.cache.try_lock()?.get(position.get())
    }

    /// Loads a page into the bounded cache. Blocking; never call on the UI thread.
    ///
    /// Returns false for an absent page. Oversized pages fail before decoding when
    /// their recorded size already exceeds the cache budget. Cancellation is checked
    /// between disk chunks and before publication; it never contacts a server.
    pub fn load_page(&self, position: BatchIndex, cancel: &CancelToken) -> Result<bool> {
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        {
            let index = self.index.read();
            match index.slots.get(position.get()) {
                None => return Ok(false),
                Some(Slot::Resident(_)) => return Ok(true),
                Some(Slot::Spilled(reference))
                    if reference.retained_bytes() > self.limits.cache_budget() =>
                {
                    return Err(DataError::Full {
                        reason: "result page exceeds the display cache budget; exporting remains available",
                    });
                }
                Some(Slot::Spilled(_)) => {}
            }
        }
        let Some(batch) = self.batch_cancellable(position, cancel)? else {
            return Ok(false);
        };
        if cancel.is_cancelled() {
            return Err(DataError::Cancelled);
        }
        if crate::spill::retained_size(&batch) > self.cache.lock().capacity_bytes() {
            return Err(DataError::Full {
                reason: "decoded result page exceeds the display cache budget; exporting remains available",
            });
        }
        Ok(true)
    }

    /// Bytes retained by the decoded-page cache; excludes transient reader clones.
    #[must_use]
    pub fn cached_bytes(&self) -> usize {
        self.cache.lock().retained_bytes()
    }

    /// Lignes du lot à cette position, sans le charger.
    ///
    /// Se lit dans l'index seul : un lot débordé répond sans toucher au disque,
    /// ce qui permet à la grille de calculer sa hauteur totale sans réhydrater
    /// dix mille lots.
    #[must_use]
    pub fn batch_rows(&self, position: BatchIndex) -> Option<usize> {
        let index = self.index.read();
        let debut = *index.starts.get(position.get())?;
        let fin = position
            .get()
            .checked_add(1)
            .and_then(|suivant| index.starts.get(suivant).copied())
            .unwrap_or(index.rows);
        Some(fin.saturating_sub(debut))
    }

    /// Numéro de la première ligne du lot à cette position.
    #[must_use]
    pub fn batch_start(&self, position: BatchIndex) -> Option<usize> {
        self.index.read().starts.get(position.get()).copied()
    }

    /// Le lot est-il en mémoire ?
    ///
    /// Permet au rendu de dessiner un remplacement plutôt que de bloquer sur une
    /// lecture disque, et de déclencher la réhydratation en tâche de fond.
    #[must_use]
    pub fn is_resident(&self, position: BatchIndex) -> bool {
        matches!(
            self.index.read().slots.get(position.get()),
            Some(Slot::Resident(_))
        )
    }

    /// Le lot contenant `row`, et le décalage de `row` dans ce lot.
    ///
    /// Composition de [`locate`](Self::locate) et [`batch`](Self::batch), pour
    /// les appelants qui ne veulent pas manipuler de position de lot — un export
    /// ou un agent, typiquement.
    ///
    /// # Erreurs
    ///
    /// Celles de [`batch`](Self::batch).
    pub fn row(&self, row: usize) -> Result<Option<(RecordBatch, usize)>> {
        self.read_row(row, &CancelToken::new())
    }

    /// Reads one existing row's batch with cooperative disk cancellation. May block.
    pub fn read_row(
        &self,
        row: usize,
        cancel: &CancelToken,
    ) -> Result<Option<(RecordBatch, usize)>> {
        let Some((position, decalage)) = self.locate(row) else {
            return Ok(None);
        };
        Ok(self
            .batch_cancellable(position, cancel)?
            .map(|lot| (lot, decalage)))
    }

    /// Refuse un lot dont le schéma n'est pas celui du tampon.
    ///
    /// Compare les champs, pas les métadonnées : un driver a le droit d'attacher
    /// des métadonnées différentes d'un lot à l'autre, il n'a pas le droit de
    /// changer les colonnes.
    fn check_schema(&self, batch: &RecordBatch) -> Result<()> {
        if batch.schema_ref().fields() == self.schema.fields() {
            return Ok(());
        }
        Err(DataError::SchemaMismatch {
            expected: format!("{:?}", self.schema.fields()),
            found: format!("{:?}", batch.schema_ref().fields()),
        })
    }

    /// Rend le fichier de débordement, en le créant au premier besoin.
    fn spill_file(&self) -> Result<Arc<SpillFile>> {
        let mut emplacement = self.spill.lock();
        if let Some(fichier) = emplacement.as_ref() {
            return Ok(Arc::clone(fichier));
        }
        let fichier = Arc::new(SpillFile::create()?);
        *emplacement = Some(Arc::clone(&fichier));
        Ok(fichier)
    }
}

/// `Debug` manuel : le dérivé imprimerait le contenu des lots, c'est-à-dire des
/// valeurs de la base de l'utilisateur, dans le premier `tracing::debug!` venu
/// ([I-03](../../../CLAUDE.md#i-03)).
impl std::fmt::Debug for ResultBuffer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let index = self.index.read();
        f.debug_struct("ResultBuffer")
            .field("columns", &self.schema.fields().len())
            .field("rows", &index.rows)
            .field("batches", &index.slots.len())
            .field("resident_bytes", &index.resident_bytes)
            .field("spilled_bytes", &index.spilled_bytes)
            .field("spilled_batches", &index.spilled_batches)
            .field("complete", &index.complete)
            .field("truncated", &index.stats.truncated)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use arrow::array::Int32Array;
    use arrow::datatypes::{DataType, Field, Schema};

    use super::*;

    fn schema() -> SchemaRef {
        Arc::new(Schema::new(vec![Field::new("n", DataType::Int32, false)]))
    }

    fn lot(depart: i32, lignes: usize) -> RecordBatch {
        let valeurs: Vec<i32> = (0..lignes)
            .map(|i| depart.saturating_add(i32::try_from(i).unwrap_or(i32::MAX)))
            .collect();
        RecordBatch::try_new(schema(), vec![Arc::new(Int32Array::from(valeurs))])
            .expect("la colonne correspond au schéma construit juste au-dessus")
    }

    fn valeur(lot: &RecordBatch, ligne: usize) -> i32 {
        lot.column(0)
            .as_any()
            .downcast_ref::<Int32Array>()
            .expect("la colonne 0 est un Int32Array par construction")
            .value(ligne)
    }

    #[test]
    fn un_tampon_neuf_est_vide() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        assert!(tampon.is_empty());
        assert_eq!(tampon.row_count(), 0);
        assert_eq!(tampon.batch_count(), 0);
        assert!(!tampon.is_complete());
        assert!(tampon.locate(0).is_none());
        assert!(tampon.has_capacity());
    }

    #[test]
    fn un_lot_vide_est_ignore_sans_erreur() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        assert_eq!(tampon.push(lot(0, 0)).expect("lot vide accepté"), None);
        assert_eq!(tampon.batch_count(), 0);
    }

    #[test]
    fn un_lot_au_mauvais_schema_est_refuse() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        let autre = Arc::new(Schema::new(vec![Field::new("n", DataType::Utf8, false)]));
        let mauvais = RecordBatch::try_new(
            autre,
            vec![Arc::new(arrow::array::StringArray::from(vec!["a"]))],
        )
        .expect("lot construit pour le test");

        match tampon.push(mauvais) {
            Err(DataError::SchemaMismatch { .. }) => {}
            autre => panic!("attendu SchemaMismatch, obtenu {autre:?}"),
        }
    }

    /// La frontière de lot est le cas que `locate` rate quand il est écrit à la
    /// main : dernière ligne d'un lot, première ligne du suivant.
    #[test]
    fn locate_tombe_juste_sur_les_frontieres_de_lot() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        for (depart, lignes) in [(0, 3), (100, 1), (200, 4)] {
            tampon.push(lot(depart, lignes)).expect("lot accepté");
        }
        assert_eq!(tampon.row_count(), 8);

        let attendu = [
            (0, 0, 0),
            (1, 0, 1),
            (2, 0, 2), // dernière ligne du lot 0
            (3, 1, 0), // le lot 1 ne contient qu'une ligne
            (4, 2, 0), // première ligne du lot 2
            (7, 2, 3), // dernière ligne du résultat
        ];
        for (ligne, batch, decalage) in attendu {
            assert_eq!(
                tampon.locate(ligne),
                Some((BatchIndex::new(batch), decalage)),
                "ligne {ligne}"
            );
        }
        assert_eq!(tampon.locate(8), None, "une ligne au-delà n'existe pas");
        assert_eq!(tampon.locate(usize::MAX), None);
    }

    /// Le même parcours, mais en vérifiant la valeur relue : `locate` peut être
    /// cohérent avec lui-même et pointer sur le mauvais lot.
    #[test]
    fn chaque_ligne_se_relit_a_sa_valeur() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 5)).expect("lot accepté");
        tampon.push(lot(1_000, 5)).expect("lot accepté");

        let attendu: Vec<i32> = (0..5).chain(1_000..1_005).collect();
        for (ligne, valeur_attendue) in attendu.into_iter().enumerate() {
            let (lot, decalage) = tampon
                .row(ligne)
                .expect("relecture")
                .expect("la ligne existe");
            assert_eq!(valeur(&lot, decalage), valeur_attendue, "ligne {ligne}");
        }
    }

    /// Budget minuscule : tout déborde, et tout se relit quand même.
    #[test]
    fn un_budget_minuscule_fait_tout_deborder_sans_rien_perdre() {
        let tampon = ResultBuffer::new(schema(), 1);
        for depart in [0, 100, 200, 300] {
            tampon.push(lot(depart, 10)).expect("lot accepté");
        }

        assert_eq!(tampon.spilled_batches(), 4, "aucun lot ne tient en mémoire");
        assert_eq!(tampon.resident_bytes(), 0);
        assert!(tampon.spilled_bytes() > 0);
        assert_eq!(tampon.row_count(), 40);

        // Parcours dans le désordre, pour ne pas dépendre du cache.
        for ligne in [39_usize, 0, 25, 10, 9, 30] {
            let (lot, decalage) = tampon
                .row(ligne)
                .expect("relecture depuis le disque")
                .expect("la ligne existe");
            let bloc = i32::try_from(ligne / 10).unwrap_or(0) * 100;
            let dans_le_bloc = i32::try_from(ligne % 10).unwrap_or(0);
            assert_eq!(valeur(&lot, decalage), bloc + dans_le_bloc, "ligne {ligne}");
        }
    }

    /// Le budget se remplit puis déborde : les premiers lots restent en mémoire,
    /// ce qui est ce qui garantit le premier affichage rapide.
    #[test]
    fn les_premiers_lots_restent_residents() {
        let echantillon = lot(0, 64);
        let taille = echantillon.get_array_memory_size();
        // Three quarters remain resident: enough for two batches, not three.
        let tampon = ResultBuffer::new(schema(), taille * 3);

        for depart in [0, 100, 200, 300] {
            tampon.push(lot(depart, 64)).expect("lot accepté");
        }

        assert_eq!(tampon.batch_count(), 4);
        assert_eq!(tampon.spilled_batches(), 2, "seuls les derniers débordent");
        assert!(tampon.is_resident(BatchIndex::new(0)));
        assert!(tampon.is_resident(BatchIndex::new(1)));
        assert!(!tampon.is_resident(BatchIndex::new(3)));
    }

    #[test]
    fn le_debordement_interdit_produit_de_la_contre_pression() {
        let tampon = ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .without_spill(),
        );
        match tampon.push(lot(0, 10)) {
            Err(DataError::Full { reason }) => assert!(reason.contains("spilling is disabled")),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
        assert!(tampon.has_capacity(), "rien n'a encore été accepté");
    }

    #[test]
    fn le_quota_de_debordement_est_respecte() {
        let tampon = ResultBuffer::with_limits(
            schema(),
            BufferLimits::default()
                .with_memory_budget(1)
                .with_max_spill_bytes(16_u64),
        );
        match tampon.push(lot(0, 1_000)) {
            Err(DataError::Full { reason }) => assert_eq!(reason, "spill quota reached"),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
    }

    #[test]
    fn la_limite_de_lignes_rabote_le_lot_et_declare_la_troncature() {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(12_usize));
        tampon.push(lot(0, 10)).expect("lot accepté");
        tampon.push(lot(100, 10)).expect("lot raboté");

        assert_eq!(tampon.row_count(), 12);
        assert!(tampon.stats().truncated);
        assert!(!tampon.has_capacity());
        assert_eq!(tampon.remaining_rows(), Some(0));

        match tampon.push(lot(200, 1)) {
            Err(DataError::Full { reason }) => assert_eq!(reason, "row limit reached"),
            autre => panic!("attendu Full, obtenu {autre:?}"),
        }
    }

    #[test]
    fn une_troncature_survit_aux_stats_du_driver() {
        let tampon =
            ResultBuffer::with_limits(schema(), BufferLimits::default().with_max_rows(5_usize));
        tampon.push(lot(0, 10)).expect("lot raboté");
        assert!(tampon.stats().truncated);

        // Le driver, lui, n'a rien vu.
        tampon.mark_complete(ExecStats {
            rows: 10,
            truncated: false,
            ..ExecStats::default()
        });

        assert!(
            tampon.stats().truncated,
            "un driver ne doit pas pouvoir effacer une troncature constatée"
        );
    }

    #[test]
    fn rien_ne_s_ajoute_apres_la_cloture() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 4)).expect("lot accepté");
        tampon.mark_complete(ExecStats::default());

        assert!(tampon.is_complete());
        assert!(!tampon.has_capacity());
        match tampon.push(lot(100, 4)) {
            Err(DataError::AlreadyComplete) => {}
            autre => panic!("attendu AlreadyComplete, obtenu {autre:?}"),
        }
    }

    #[test]
    fn une_position_inexistante_rend_none() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(0, 2)).expect("lot accepté");
        assert!(
            tampon
                .batch(BatchIndex::new(7))
                .expect("pas d'erreur")
                .is_none()
        );
        assert!(tampon.row(99).expect("pas d'erreur").is_none());
    }

    /// Le `Debug` ne doit jamais imprimer de valeur de la base
    /// ([I-03](../../../CLAUDE.md#i-03)).
    #[test]
    fn le_debug_ne_montre_aucune_valeur() {
        let tampon = ResultBuffer::new(schema(), DEFAULT_MEMORY_BUDGET);
        tampon.push(lot(424_242, 3)).expect("lot accepté");
        let rendu = format!("{tampon:?}");
        assert!(!rendu.contains("424242"), "{rendu}");
        assert!(rendu.contains("rows: 3"), "{rendu}");
    }

    /// La grille lit pendant que le puits pousse : le tampon doit rester
    /// cohérent, et surtout `locate` ne doit jamais rendre une position hors
    /// borne des lots publiés.
    #[test]
    fn lectures_et_ecritures_concurrentes_restent_coherentes() {
        // Budget assez petit pour que la quasi-totalité des lots déborde : le
        // test exerce ainsi la lecture disque **pendant** l'écriture disque.
        let tampon = Arc::new(ResultBuffer::new(schema(), 128));
        let ecrivain = Arc::clone(&tampon);

        let producteur = std::thread::spawn(move || {
            for i in 0..64_i32 {
                ecrivain.push(lot(i * 10, 10)).expect("lot accepté");
            }
            ecrivain.mark_complete(ExecStats::default());
        });

        while !tampon.is_complete() {
            let lignes = tampon.row_count();
            for ligne in (0..lignes).step_by(7) {
                let (position, decalage) = tampon.locate(ligne).expect("ligne annoncée reçue");
                let lot = tampon
                    .batch(position)
                    .expect("relecture")
                    .expect("le lot localisé existe");
                assert!(decalage < lot.num_rows());
            }
        }

        producteur.join().expect("le producteur ne panique pas");
        assert_eq!(tampon.row_count(), 640);
    }
}

#[cfg(test)]
mod page_tests {
    use super::*;
    use arrow::array::Int64Array;
    use arrow::datatypes::{DataType, Field, Schema};

    fn batch() -> RecordBatch {
        RecordBatch::try_new(
            Arc::new(Schema::new(vec![Field::new("n", DataType::Int64, false)])),
            vec![Arc::new(Int64Array::from_iter_values(0..512))],
        )
        .expect("batch")
    }

    #[test]
    fn page_cache_shares_the_budget_and_loading_does_not_change_the_result() {
        let sample = batch();
        let buffer = ResultBuffer::new(sample.schema(), 64 * 1024);
        for _ in 0..64 {
            buffer.push(sample.clone()).expect("push");
        }
        buffer.mark_complete(ExecStats::default());
        for index in (0..64).rev() {
            let position = BatchIndex::new(index);
            assert!(
                buffer
                    .load_page(position, &CancelToken::new())
                    .expect("page load")
            );
            assert_eq!(buffer.cached_batch(position).expect("loaded page"), sample);
            assert!(
                buffer.resident_bytes() + buffer.cached_bytes() <= buffer.limits().memory_budget
            );
        }
        assert_eq!(buffer.row_count(), 64 * 512);
        assert!(!buffer.stats().truncated);
        assert!(
            buffer.cached_batch(BatchIndex::new(63)).is_none(),
            "older decoded pages are evicted"
        );
    }

    #[test]
    fn oversized_and_cancelled_pages_do_not_populate_the_cache() {
        let sample = batch();
        let buffer = ResultBuffer::new(sample.schema(), 1);
        buffer.push(sample.clone()).expect("spill");
        assert!(matches!(
            buffer.load_page(BatchIndex::new(0), &CancelToken::new()),
            Err(DataError::Full { .. })
        ));
        assert_eq!(buffer.cached_bytes(), 0);
        assert_eq!(
            buffer
                .batch(BatchIndex::new(0))
                .expect("export can still read"),
            Some(sample)
        );
        assert_eq!(buffer.cached_bytes(), 0);
        let cancel = CancelToken::new();
        cancel.cancel();
        assert!(matches!(
            buffer.load_page(BatchIndex::new(0), &cancel),
            Err(DataError::Cancelled)
        ));
        assert!(buffer.cached_batch(BatchIndex::new(0)).is_none());
    }
}
