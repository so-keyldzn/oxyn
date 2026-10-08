//! What semantic ranking protects without its 800 MB model: off changes
//! nothing, a deadline costs scores and never the question, turning off
//! leaves nothing behind, and no `NaN` reaches the ordering.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use oxyn_catalog::{CatalogCache, CatalogPath, RelationKind, RelationRef};
use oxyn_embed::pinned::DIMENSIONS;
use oxyn_embed::{EmbedError, Embedding};
use parking_lot::{Mutex, RwLock};

use super::vectors::{BATCH, Embeddable, VectorCache, rank};
use super::{Ranking, failed};
use crate::backend::Backend;
use crate::ipc::semantic::ModelState;

fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("a test runtime starts")
}

/// A unit vector along one axis: two texts on the same axis score 1, on
/// different axes 0.
fn axis(index: usize) -> Embedding {
    let mut values = vec![0.0_f32; DIMENSIONS];
    if let Some(value) = values.get_mut(index % DIMENSIONS) {
        *value = 1.0;
    }
    Embedding::from_values(&values).expect("a unit vector")
}

/// An embedding by the text's first byte: `customers` and `clients` share
/// an axis, `audit` has its own.
fn by_initial(texts: &[&str]) -> Result<Vec<Embedding>, EmbedError> {
    Ok(texts
        .iter()
        .map(|text| axis(usize::from(text.bytes().next().unwrap_or(0))))
        .collect())
}

fn relation(name: &str) -> (CatalogPath, Embeddable) {
    let path = CatalogPath::for_relation(None, Some("public"), name).expect("a valid path");
    (path, Embeddable::text(name))
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(60)
}

#[test]
fn relations_are_scored_by_the_cosine_with_the_question() {
    let vectors = Mutex::new(VectorCache::default());
    let ranked = rank(
        "clients who ordered",
        vec![relation("customers"), relation("audit")],
        &vectors,
        by_initial,
        far_future(),
    )
    .expect("ranked");
    let (customers, _) = relation("customers");
    let (audit, _) = relation("audit");
    assert_eq!(ranked.scores.get(&customers), Some(1.0));
    assert_eq!(ranked.scores.get(&audit), Some(0.0));
    assert_eq!(ranked.unscored, 0);
    assert_eq!(ranked.embedded, 2, "both computed for this question");
    assert_eq!(vectors.lock().len(), 2, "both relations are kept");
}

#[test]
fn a_relation_already_embedded_is_not_embedded_again() {
    let vectors = Mutex::new(VectorCache::default());
    let calls = AtomicUsize::new(0);
    let counting = |texts: &[&str]| {
        calls.fetch_add(texts.len(), Ordering::SeqCst);
        by_initial(texts)
    };
    let relations = || vec![relation("customers"), relation("orders")];
    rank("q", relations(), &vectors, counting, far_future()).expect("first");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "the question and two relations"
    );
    rank("q", relations(), &vectors, counting, far_future()).expect("second");
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "the second question embeds only itself"
    );
}

#[test]
fn a_deadline_reached_costs_scores_never_the_question() {
    let vectors = Mutex::new(VectorCache::default());
    let relations: Vec<_> = (0..BATCH * 3)
        .map(|n| relation(&format!("table_{n:03}")))
        .collect();
    let total = relations.len();
    // Each batch takes longer than the whole budget: the first one runs,
    // the deadline is then past, and the others wait for the next question.
    // A busy wait: the closure is synchronous, as the model's call is, and
    // `std::thread::sleep` is refused by `clippy.toml` (I-05).
    let slow = |texts: &[&str]| {
        if texts.len() > 1 {
            let until = Instant::now() + Duration::from_millis(60);
            while Instant::now() < until {
                std::hint::spin_loop();
            }
        }
        by_initial(texts)
    };
    let ranked = rank(
        "q",
        relations,
        &vectors,
        slow,
        Instant::now() + Duration::from_millis(30),
    )
    .expect("a deadline is not an error");
    assert_eq!(ranked.scores.len(), BATCH, "one batch past the deadline");
    assert_eq!(ranked.unscored, total - BATCH);
    assert_eq!(vectors.lock().len(), BATCH, "what was embedded is kept");

    // Already past: nothing is embedded, not even the question.
    let ranked = rank(
        "q",
        vec![relation("orders")],
        &vectors,
        |_: &[&str]| -> Result<Vec<Embedding>, EmbedError> {
            panic!("no embedding past the deadline")
        },
        Instant::now(),
    )
    .expect("no error");
    assert!(ranked.scores.is_empty());
    assert_eq!(ranked.unscored, 1);
}

#[test]
fn no_nan_reaches_the_ordering() {
    let mut nan = vec![0.5_f32; DIMENSIONS];
    if let Some(value) = nan.first_mut() {
        *value = f32::NAN;
    }
    assert!(
        Embedding::from_values(&nan).is_none(),
        "a vector with a NaN is refused"
    );
    assert!(
        Embedding::from_values(&vec![0.0; DIMENSIONS]).is_none(),
        "a zero vector cannot be normalized"
    );
    let vectors = Mutex::new(VectorCache::default());
    let names: Vec<String> = (0..40)
        .map(|n| format!("{}_t", char::from(b'a' + n % 26)))
        .collect();
    let relations: Vec<_> = names.iter().map(|name| relation(name)).collect();
    let paths: Vec<_> = relations.iter().map(|(path, _)| path.clone()).collect();
    let ranked = rank("a question", relations, &vectors, by_initial, far_future()).expect("ranked");
    for path in &paths {
        let score = ranked.scores.get(path).expect("scored");
        assert!(score.is_finite() && (-1.0..=1.0).contains(&score));
    }
}

#[test]
fn a_failed_embedding_is_an_error_not_a_panic() {
    let vectors = Mutex::new(VectorCache::default());
    let missing =
        |_: &[&str]| -> Result<Vec<Embedding>, EmbedError> { Err(EmbedError::NotDownloaded) };
    assert!(matches!(
        rank(
            "q",
            vec![relation("orders")],
            &vectors,
            missing,
            far_future()
        ),
        Err(EmbedError::NotDownloaded)
    ));
}

#[test]
fn a_relation_is_embedded_by_its_qualified_name_and_comment() {
    let schema = CatalogPath::for_namespace(None, "public").expect("a namespace");
    let plain = RelationRef::new(schema.clone(), "orders", RelationKind::Table).expect("valid");
    let commented = RelationRef::new(schema, "orders", RelationKind::Table)
        .expect("valid")
        .with_comment("  Customer orders  ");
    assert_eq!(Embeddable::of(&plain).embedded(), "public.orders");
    assert_eq!(
        Embeddable::of(&commented).embedded(),
        "public.orders\nCustomer orders"
    );
    // The `Debug` counts bytes; it never shows the schema.
    assert!(!format!("{:?}", Embeddable::of(&commented)).contains("orders"));
}

#[test]
fn the_cache_tells_texts_apart_by_their_digest() {
    let vectors = Mutex::new(VectorCache::default());
    // Same initial, so the same fake vector: only the key tells them apart.
    let relations = vec![relation("orders"), relation("orders_archive")];
    rank("q", relations, &vectors, by_initial, far_future()).expect("ranked");
    assert_eq!(vectors.lock().len(), 2, "two texts, two keys");
    rank(
        "q",
        vec![relation("orders")],
        &vectors,
        by_initial,
        far_future(),
    )
    .expect("ranked");
    assert_eq!(vectors.lock().len(), 2, "the same text finds its key again");
}

/// No error text reaches the settings or the journal: an I/O message holds
/// a local path, a dependency's detail whatever it was given.
#[test]
fn an_error_shown_copies_no_error_text() {
    const LEAK: &str = "/Users/someone/Library/SECRET-MARK";
    let errors = [
        EmbedError::Io {
            action: "rename",
            path: LEAK.into(),
            source: std::io::Error::new(std::io::ErrorKind::StorageFull, LEAK),
        },
        EmbedError::Download {
            file: "model.safetensors",
            detail: format!("huggingface.co: {LEAK}"),
        },
        EmbedError::Corrupt {
            file: "model.bpk",
            detail: LEAK.to_owned(),
        },
        EmbedError::Conversion(LEAK.to_owned()),
        EmbedError::Tokenizer(LEAK.to_owned()),
        EmbedError::Inference(LEAK.to_owned()),
        EmbedError::NotDownloaded,
        EmbedError::Cancelled,
        EmbedError::DownloadInProgress,
    ];
    for error in &errors {
        let ModelState::Failed { message, .. } = failed(error) else {
            panic!("a failure");
        };
        assert!(!message.contains("SECRET-MARK"), "{message}");
        assert!(!message.is_empty());
    }
    let ModelState::Failed { message, retryable } = failed(&errors[0]) else {
        panic!("a failure");
    };
    assert_eq!(message, "Could not rename a model file: the disk is full.");
    assert!(!retryable);
    let ModelState::Failed { retryable, .. } = failed(&errors[1]) else {
        panic!("a failure");
    };
    assert!(retryable, "a pinned GET can be tried again");
    let ModelState::Failed { message, retryable } = failed(&EmbedError::DownloadInProgress) else {
        panic!("a failure");
    };
    assert_eq!(
        message,
        "Another Oxyn window or process is downloading the model; try again once it ends."
    );
    assert!(retryable, "waiting for the other download is the fix");
}

/// A workspace whose model lives in a directory of its own.
fn placed(runtime: &tokio::runtime::Runtime) -> (Backend, tempfile::TempDir) {
    let backend = runtime
        .block_on(async { Backend::open_temporary() })
        .expect("a temporary backend");
    let root = tempfile::tempdir().expect("a temporary directory");
    backend.inner.semantic.place(root.path());
    (backend, root)
}

fn catalog_of(names: &[&str]) -> oxyn_catalog::SharedCatalog {
    let schema = CatalogPath::for_namespace(None, "public").expect("a namespace");
    let mut cache = CatalogCache::new();
    cache
        .set_relations(
            &schema,
            names
                .iter()
                .map(|name| {
                    RelationRef::new(schema.clone(), *name, RelationKind::Table).expect("valid")
                })
                .collect(),
        )
        .expect("a namespace");
    Arc::new(RwLock::new(cache))
}

#[test]
fn off_by_default_and_off_means_no_ranking() {
    let runtime = runtime();
    let (backend, _root) = placed(&runtime);
    let snapshot = runtime.block_on(backend.semantic_state()).expect("a state");
    assert!(!snapshot.enabled, "off by default");
    assert_eq!(snapshot.model, ModelState::Absent);
    assert!(
        Ranking::new(&backend.inner, "which customers ordered?").is_none(),
        "off: the selection is the lexical one"
    );
    assert!(!backend.inner.semantic.is_resident(), "nothing is created");
}

#[test]
fn a_workspace_without_a_model_directory_offers_nothing() {
    let runtime = runtime();
    let backend = runtime
        .block_on(async { Backend::open_temporary() })
        .expect("a temporary backend");
    runtime
        .block_on(backend.save_preferences(|preferences| preferences.semantic_ranking = true))
        .expect("saved");
    assert!(Ranking::new(&backend.inner, "q").is_none());
    assert_eq!(
        backend.inner.semantic.snapshot().model,
        ModelState::Unavailable
    );
    assert!(runtime.block_on(backend.enable_semantic_ranking()).is_err());
}

#[test]
fn on_without_a_model_ranks_lexically() {
    let runtime = runtime();
    let (backend, _root) = placed(&runtime);
    runtime
        .block_on(backend.save_preferences(|preferences| preferences.semantic_ranking = true))
        .expect("saved");
    let ranking = Ranking::new(&backend.inner, "which customers ordered?").expect("on");
    let catalog = catalog_of(&["customers", "orders"]);
    let scores = runtime.block_on(ranking.scores_within(&catalog, super::SEMANTIC_DEADLINE));
    assert!(scores.is_empty(), "no model: no score, and no error");
    assert!(
        !backend.inner.semantic.is_resident(),
        "nothing is loaded for a model that is not there"
    );
}

#[test]
fn turning_off_unloads_the_model_empties_the_vectors_and_deletes_the_files() {
    let runtime = runtime();
    let (backend, root) = placed(&runtime);
    let state = &backend.inner.semantic;
    runtime
        .block_on(backend.save_preferences(|preferences| preferences.semantic_ranking = true))
        .expect("saved");
    // A model held in memory, vectors computed, files on disk.
    let store = state.store.get().expect("placed").clone();
    runtime.block_on(async { drop(state.resident(&store)) });
    assert!(state.is_resident());
    rank(
        "q",
        vec![relation("customers")],
        &state.vectors,
        by_initial,
        far_future(),
    )
    .expect("ranked");
    assert_eq!(state.vector_count(), 1);
    std::fs::create_dir_all(store.dir()).expect("the model directory");
    std::fs::write(store.dir().join("tokenizer.json"), b"{}").expect("a file");

    let snapshot = runtime
        .block_on(backend.disable_semantic_ranking())
        .expect("turned off");
    assert!(!snapshot.enabled);
    assert_eq!(snapshot.model, ModelState::Absent);
    assert!(!state.is_resident(), "the model and its idle task are gone");
    assert_eq!(state.vector_count(), 0, "no vector is kept");
    assert!(!store.dir().exists(), "the files are deleted");
    assert!(root.path().exists(), "only the model's own directory");
    assert!(!backend.inner.settings.preferences.semantic_ranking());
}

/// Both generations full, the question's vectors spread over them: every
/// relation is scored, although promoting the previous generation's makes
/// the current one rotate (Codex review of #215).
#[test]
fn a_full_cache_scores_every_relation_it_holds() {
    let vectors = Mutex::new(VectorCache::with_generation(2));
    // Four relations embedded in order: `a`, `b` go to the previous
    // generation once `c`, `d` fill the current one.
    let all = || {
        vec![
            relation("a_t"),
            relation("b_t"),
            relation("c_t"),
            relation("d_t"),
        ]
    };
    rank("q", all(), &vectors, by_initial, far_future()).expect("embedded");
    assert_eq!(vectors.lock().len(), 4, "both generations full");

    let calls = AtomicUsize::new(0);
    let counting = |texts: &[&str]| {
        calls.fetch_add(texts.len(), Ordering::SeqCst);
        by_initial(texts)
    };
    let ranked = rank("q", all(), &vectors, counting, far_future()).expect("ranked");
    assert_eq!(ranked.unscored, 0, "no relation lost its score");
    assert_eq!(ranked.scores.len(), 4);
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "only the question is embedded"
    );
}

#[test]
fn a_question_still_embedding_when_turned_off_keeps_nothing() {
    let vectors = Mutex::new(VectorCache::default());
    let clearing = |texts: &[&str]| {
        if texts.len() > 1 {
            // The option is turned off while this batch runs.
            vectors.lock().clear();
        }
        by_initial(texts)
    };
    let ranked = rank(
        "q",
        vec![relation("customers"), relation("orders")],
        &vectors,
        clearing,
        far_future(),
    )
    .expect("no error");
    assert!(ranked.scores.is_empty());
    assert_eq!(vectors.lock().len(), 0, "the cleared cache stays empty");
}

/// The journal lines a closure writes on this thread, at `debug`.
///
/// A subscriber scoped to the thread, and a current-thread runtime in the
/// closure: the semantic step's traces are emitted by its async code, which
/// then runs here. The callsite cache is rebuilt so a callsite another test
/// saw disabled is evaluated again.
fn journal_of(work: impl FnOnce()) -> String {
    #[derive(Clone, Default)]
    struct Lines(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Lines {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    let lines = Lines::default();
    let writer = lines.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || {
        tracing::callsite::rebuild_interest_cache();
        work();
    });
    let bytes = lines.0.lock().clone();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Why the step did not run is traced — a fixed reason —, and neither the
/// question nor a relation's name ever is (I-03).
#[test]
fn a_skipped_ranking_says_why_and_names_nothing() {
    const QUESTION: &str = "which QUESTION-MARK invoices are unpaid?";
    let catalog = catalog_of(&["customer_invoices_relation_mark"]);
    let journal = journal_of(|| {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("a test runtime starts");
        let (backend, _root) = placed(&runtime);
        // Off: nothing runs.
        assert!(Ranking::new(&backend.inner, QUESTION).is_none());
        // On, and no model on disk.
        runtime
            .block_on(backend.save_preferences(|preferences| preferences.semantic_ranking = true))
            .expect("saved");
        let ranking = Ranking::new(&backend.inner, QUESTION).expect("on");
        let scores = runtime.block_on(ranking.scores_within(&catalog, super::SEMANTIC_DEADLINE));
        assert!(scores.is_empty());
    });
    // Presence first: a capture that saw nothing would pass the rest.
    assert!(
        journal.contains(r#"semantic ranking skipped; lexical ranking only reason="option off""#),
        "{journal}"
    );
    assert!(journal.contains(r#"reason="model absent""#), "{journal}");
    assert!(!journal.contains("QUESTION-MARK"), "{journal}");
    assert!(!journal.contains("relation_mark"), "{journal}");
}
