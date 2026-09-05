//! L'exécution d'une demande, côté thread porteur.
//!
//! Tout ce qui est ici tourne **sur le thread de la connexion** : c'est le seul
//! endroit où un `Statement` et ses `Rows` existent, et ils n'en sortent jamais.
//! Ce qui traverse le canal, ce sont des `RecordBatch` Arrow.
//!
//! # Un lot de plusieurs instructions
//!
//! Le découpage d'un script appartient à `oxyn-query`
//! ([ARCHITECTURE §3](../../../docs/ARCHITECTURE.md)) ; ce driver accepte
//! néanmoins plusieurs instructions dans une soumission, et la règle est en une
//! phrase :
//!
//! > **Toutes les instructions sont exécutées, dans l'ordre ; le curseur porte
//! > le résultat de la dernière.**
//!
//! Un jeu de résultats produit par une instruction qui n'est pas la dernière est
//! consommé et jeté — l'instruction s'exécute quand même, parce que ses effets
//! comptent. `SELECT 1; INSERT INTO t VALUES (2);` insère donc bien, et rend un
//! résultat vide.
//!
//! Les instructions sont préparées **au fur et à mesure**, pas toutes d'avance :
//! préparer `SELECT * FROM t` avant d'avoir exécuté le `CREATE TABLE t` qui le
//! précède échouerait.
//!
//! # Ce qui est refusé, et pourquoi
//!
//! * **Des paramètres liés avec un lot de plusieurs instructions.** Rien ne dit
//!   à laquelle ils se rapportent. Deviner reviendrait à lier des valeurs à une
//!   instruction que l'utilisateur ne visait pas.
//! * **Une écriture sous `ExecLimits::read_only`.** La question n'est pas posée
//!   au texte mais au moteur : `sqlite3_stmt_readonly` sait ce que
//!   l'instruction compilée peut faire, là où une analyse lexicale se ferait
//!   avoir par une vue, un déclencheur ou une table virtuelle.

use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::datatypes::{Schema, SchemaRef};
use arrow::record_batch::RecordBatch;
use oxyn_core::{ErrorClass, ExecRequest, OxynError, Result};
use rusqlite::fallible_iterator::FallibleIterator as _;
use rusqlite::{Batch, Connection, Row, Rows, Statement};
use tokio::sync::{mpsc, oneshot};

use crate::convert::{ColumnBuilder, ColumnPlan, Observed, ProbeValue, schema_of, value_bytes};
use crate::error::{self, Effect, SqliteError};
use crate::options::BatchLimits;
use crate::params;

/// La demande d'un lot, par le curseur.
pub(crate) type Pull = oneshot::Sender<Result<Pulled>>;

/// Ce qu'une demande de lot rapporte.
pub(crate) enum Pulled {
    /// Un lot de lignes.
    Batch(RecordBatch),
    /// Le flux est terminé.
    Done {
        /// Des lignes manquent, parce que `ExecLimits::max_rows` a coupé.
        truncated: bool,
    },
}

/// Ce que l'exécution rend dès que le schéma est connu.
///
/// Le premier lot est **déjà là** : le résoudre a demandé de lire des lignes, et
/// les jeter pour les relire ensuite serait absurde.
pub(crate) struct StreamStart {
    /// Le schéma des lots, stable pour toute la durée du flux.
    pub schema: SchemaRef,
    /// Le premier lot, ou la fin du flux s'il n'y a pas de ligne.
    pub first: Pulled,
    /// Lignes affectées par les instructions qui n'ont pas produit de colonnes.
    pub affected: u64,
}

/// Une exécution confiée au thread porteur.
pub(crate) struct StreamJob {
    /// La demande, telle que l'appelant l'a formulée.
    pub request: ExecRequest,
    /// Les bornes d'un lot Arrow.
    pub limits: BatchLimits,
    /// Où répondre une fois le schéma connu.
    pub start: oneshot::Sender<Result<StreamStart>>,
    /// Par où le curseur demande la suite.
    pub pulls: mpsc::UnboundedReceiver<Pull>,
}

/// Exécute une demande et diffuse ses lots jusqu'à épuisement ou destruction du
/// curseur.
pub(crate) fn run(connection: &Connection, job: StreamJob) {
    let StreamJob {
        request,
        limits,
        start,
        mut pulls,
    } = job;

    let mut affected = 0_u64;
    let mut batch = Batch::new(connection, &request.text);
    let last = match select_last(connection, &mut batch, &request, &mut affected) {
        Ok(statement) => statement,
        Err(err) => {
            let _ = start.send(Err(err));
            return;
        }
    };
    drop(batch);

    let Some(mut statement) = last else {
        // Un texte vide, ou seulement des commentaires.
        let _ = start.send(Ok(nothing(affected)));
        return;
    };

    if let Err(err) = guard_read_only(&statement, &request) {
        let _ = start.send(Err(err));
        return;
    }
    if let Err(err) = params::bind(&mut statement, &request.params) {
        let _ = start.send(Err(error::driver(err, ErrorClass::Permanent)));
        return;
    }

    let effect = effect_of(&statement);
    if statement.column_count() == 0 {
        // Écriture ou DDL : pas de colonnes, donc pas de flux. Ce que
        // l'utilisateur attend, c'est le compte de lignes affectées.
        let before = connection.total_changes();
        if let Err(err) = statement.raw_execute() {
            let _ = start.send(Err(error::engine(err, effect)));
            return;
        }
        affected = affected.saturating_add(changes_since(connection, before));
        let _ = start.send(Ok(nothing(affected)));
        return;
    }

    stream_rows(statement, &request, limits, effect, start, &mut pulls);
}

/// Le résultat d'une exécution qui ne produit aucune colonne.
fn nothing(affected: u64) -> StreamStart {
    StreamStart {
        schema: Arc::new(Schema::empty()),
        first: Pulled::Done { truncated: false },
        affected,
    }
}

/// Exécute toutes les instructions sauf la dernière, et rend celle-ci.
///
/// Rend `None` si le texte ne contient aucune instruction.
fn select_last<'conn>(
    connection: &'conn Connection,
    batch: &mut Batch<'conn, '_>,
    request: &ExecRequest,
    affected: &mut u64,
) -> Result<Option<Statement<'conn>>> {
    let mut pending = next_statement(batch)?;
    loop {
        let Some(mut statement) = pending.take() else {
            return Ok(None);
        };

        // Sans paramètres liés, une instruction sans colonnes s'exécute avant
        // que la suivante soit préparée. C'est ce qui fait marcher
        // `CREATE TABLE t; SELECT * FROM t;` : la seconde ne se prépare qu'une
        // fois la table créée.
        if request.params.is_empty() && statement.column_count() == 0 {
            guard_read_only(&statement, request)?;
            let effect = effect_of(&statement);
            let before = connection.total_changes();
            statement
                .raw_execute()
                .map_err(|err| error::engine(err, effect))?;
            *affected = affected.saturating_add(changes_since(connection, before));
            pending = next_statement(batch)?;
            continue;
        }

        // Il faut savoir s'il reste une instruction après celle-ci. La préparer
        // maintenant est sans danger : tout ce qui la précède a déjà tourné, et
        // préparer n'exécute rien.
        let next = next_statement(batch)?;
        if next.is_none() {
            return Ok(Some(statement));
        }
        if !request.params.is_empty() {
            return Err(error::driver(
                SqliteError::ParametersWithBatch,
                ErrorClass::Permanent,
            ));
        }
        guard_read_only(&statement, request)?;
        // Ce jeu de résultats est écrasé par celui de l'instruction suivante. On
        // l'exécute quand même — ses effets comptent — et on jette ses lignes.
        discard(&mut statement, connection, affected)?;
        pending = next;
    }
}

/// L'instruction suivante du lot, préparée.
fn next_statement<'conn>(batch: &mut Batch<'conn, '_>) -> Result<Option<Statement<'conn>>> {
    // Préparer ne modifie rien : une erreur de préparation n'est jamais ambiguë.
    batch
        .next()
        .map_err(|err| error::engine(err, Effect::ReadOnly))
}

/// Exécute une instruction et jette ses lignes.
fn discard(
    statement: &mut Statement<'_>,
    connection: &Connection,
    affected: &mut u64,
) -> Result<()> {
    let effect = effect_of(statement);
    let before = connection.total_changes();
    let mut rows = statement.raw_query();
    while step(&mut rows, effect)?.is_some() {}
    drop(rows);
    *affected = affected.saturating_add(changes_since(connection, before));
    Ok(())
}

/// Diffuse les lignes d'une instruction.
fn stream_rows(
    mut statement: Statement<'_>,
    request: &ExecRequest,
    limits: BatchLimits,
    effect: Effect,
    start: oneshot::Sender<Result<StreamStart>>,
    pulls: &mut mpsc::UnboundedReceiver<Pull>,
) {
    // Relevé avant d'emprunter l'instruction pour la lecture : `columns()`
    // emprunte, `raw_query()` emprunte mutablement.
    let declared: Vec<(String, Option<String>)> = statement
        .columns()
        .iter()
        .map(|column| {
            (
                column.name().to_owned(),
                column.decl_type().map(str::to_owned),
            )
        })
        .collect();
    let width = declared.len();
    let mut remaining = request.limits.max_rows;
    let mut rows = statement.raw_query();

    let sonde = match probe(&mut rows, width, limits, &mut remaining, effect) {
        Ok(sonde) => sonde,
        Err(err) => {
            let _ = start.send(Err(err));
            return;
        }
    };

    let plans: Vec<ColumnPlan> = declared
        .into_iter()
        .zip(sonde.observed)
        .map(|((name, declared), observed)| ColumnPlan::resolve(name, declared, observed))
        .collect();
    let schema = schema_of(&plans);
    for (index, plan) in plans.iter().enumerate() {
        if plan.observed.is_mixed() {
            // Ni le nom de la colonne ni aucune valeur : seulement l'index et
            // les classes rencontrées (I-03). Le signal destiné à l'interface
            // est dans les métadonnées du champ ; celui-ci est pour le
            // diagnostic.
            tracing::debug!(
                column = index,
                storage_classes = %plan.observed.names(),
                resolved = %plan.kind,
                "colonne au typage mêlé : le rendu retenu est un repli"
            );
        }
    }
    let capacity = sonde.rows.max(64);
    let mut builders: Vec<ColumnBuilder> = plans
        .iter()
        .enumerate()
        .map(|(index, plan)| ColumnBuilder::new(plan.kind, index, capacity))
        .collect();

    let mut finished = sonde.finished;
    let mut truncated = sonde.truncated;

    let first = if sonde.rows == 0 {
        Pulled::Done { truncated }
    } else {
        match assemble(&mut builders, &sonde.values, &schema) {
            Ok(batch) => Pulled::Batch(batch),
            Err(err) => {
                let _ = start.send(Err(err));
                return;
            }
        }
    };

    if start
        .send(Ok(StreamStart {
            schema: Arc::clone(&schema),
            first,
            affected: 0,
        }))
        .is_err()
    {
        // L'appelant a renoncé avant même de lire le premier lot.
        return;
    }

    while let Some(reply) = pulls.blocking_recv() {
        if finished {
            let _ = reply.send(Ok(Pulled::Done { truncated }));
            continue;
        }
        let filled = match fill(&mut rows, &mut builders, limits, &mut remaining, effect) {
            Ok(filled) => filled,
            Err(err) => {
                let _ = reply.send(Err(err));
                return;
            }
        };
        finished = filled.finished;
        truncated |= filled.truncated;

        let answer = if filled.rows == 0 {
            Ok(Pulled::Done { truncated })
        } else {
            finish_batch(&mut builders, &schema).map(Pulled::Batch)
        };
        let failed = answer.is_err();
        if reply.send(answer).is_err() || failed {
            return;
        }
    }
}

/// Ce que la sonde du premier lot a appris.
struct Probe {
    /// Les valeurs du premier lot, ligne par ligne, colonne par colonne.
    values: Vec<ProbeValue>,
    /// Les classes de stockage vues, colonne par colonne.
    observed: Vec<Observed>,
    /// Lignes lues.
    rows: usize,
    /// La source est épuisée, ou la borne de lignes est atteinte.
    finished: bool,
    /// Des lignes manquent.
    truncated: bool,
}

/// Lit le premier lot **en valeurs brutes**, pour décider du type des colonnes.
///
/// C'est le prix de l'honnêteté sur le typage dynamique de SQLite : sans cette
/// sonde, le type d'une colonne sans déclaration ne pourrait venir que de la
/// première ligne, et une colonne mêlant entiers et texte serait typée sur son
/// premier échantillon. La copie ne concerne que le premier lot, dont la taille
/// est bornée en lignes **et** en octets.
fn probe(
    rows: &mut Rows<'_>,
    width: usize,
    limits: BatchLimits,
    remaining: &mut Option<usize>,
    effect: Effect,
) -> Result<Probe> {
    let mut values = Vec::new();
    let mut observed = vec![Observed::default(); width];
    let mut count = 0_usize;
    let mut bytes = 0_usize;
    let mut finished = false;
    let mut truncated = false;

    loop {
        if *remaining == Some(0) {
            truncated = step(rows, effect)?.is_some();
            finished = true;
            break;
        }
        let Some(row) = step(rows, effect)? else {
            finished = true;
            break;
        };
        for (index, seen) in observed.iter_mut().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|err| error::engine(err, effect))?;
            seen.observe(value);
            bytes = bytes.saturating_add(value_bytes(value));
            values.push(ProbeValue::capture(value));
        }
        count = count.saturating_add(1);
        consume_one(remaining);
        if limits.reached(count, bytes) {
            break;
        }
    }

    Ok(Probe {
        values,
        observed,
        rows: count,
        finished,
        truncated,
    })
}

/// Ce qu'un lot de régime a rapporté.
struct Filled {
    /// Lignes ajoutées.
    rows: usize,
    /// Plus rien ne viendra.
    finished: bool,
    /// Des lignes manquent.
    truncated: bool,
}

/// Remplit un lot en régime établi : le type des colonnes est déjà décidé.
fn fill(
    rows: &mut Rows<'_>,
    builders: &mut [ColumnBuilder],
    limits: BatchLimits,
    remaining: &mut Option<usize>,
    effect: Effect,
) -> Result<Filled> {
    let mut count = 0_usize;
    let mut bytes = 0_usize;
    let mut finished = false;
    let mut truncated = false;

    loop {
        if *remaining == Some(0) {
            truncated = step(rows, effect)?.is_some();
            finished = true;
            break;
        }
        let Some(row) = step(rows, effect)? else {
            finished = true;
            break;
        };
        for (index, builder) in builders.iter_mut().enumerate() {
            let value = row
                .get_ref(index)
                .map_err(|err| error::engine(err, effect))?;
            bytes = bytes.saturating_add(value_bytes(value));
            builder
                .append(value)
                .map_err(|err| error::driver(err, ErrorClass::Permanent))?;
        }
        count = count.saturating_add(1);
        consume_one(remaining);
        if limits.reached(count, bytes) {
            break;
        }
    }

    Ok(Filled {
        rows: count,
        finished,
        truncated,
    })
}

/// Avance d'une ligne.
fn step<'a, 'stmt>(rows: &'a mut Rows<'stmt>, effect: Effect) -> Result<Option<&'a Row<'stmt>>> {
    rows.next().map_err(|err| error::engine(err, effect))
}

/// Décompte une ligne de la borne `ExecLimits::max_rows`, quand il y en a une.
fn consume_one(remaining: &mut Option<usize>) {
    if let Some(left) = remaining {
        *left = left.saturating_sub(1);
    }
}

/// Le nombre de lignes modifiées depuis un relevé.
///
/// `total_changes` est monotone sur la durée de la connexion, contrairement à
/// `changes()` qui ne parle que de la dernière instruction — et qui garde sa
/// valeur précédente après un `SELECT` ou un `CREATE TABLE`. Le compte inclut
/// les lignes touchées par les déclencheurs, ce qui est ce que l'utilisateur
/// veut savoir.
fn changes_since(connection: &Connection, before: u64) -> u64 {
    connection.total_changes().saturating_sub(before)
}

/// Verse les valeurs mises de côté par la sonde dans les constructeurs.
fn assemble(
    builders: &mut [ColumnBuilder],
    values: &[ProbeValue],
    schema: &SchemaRef,
) -> Result<RecordBatch> {
    let width = builders.len();
    if width > 0 {
        for line in values.chunks(width) {
            for (builder, value) in builders.iter_mut().zip(line) {
                builder
                    .append(value.borrow())
                    .map_err(|err| error::driver(err, ErrorClass::Permanent))?;
            }
        }
    }
    finish_batch(builders, schema)
}

/// Ferme les constructeurs et assemble le `RecordBatch`.
fn finish_batch(builders: &mut [ColumnBuilder], schema: &SchemaRef) -> Result<RecordBatch> {
    let columns: Vec<ArrayRef> = builders.iter_mut().map(ColumnBuilder::finish).collect();
    RecordBatch::try_new(Arc::clone(schema), columns)
        .map_err(|err| error::driver(SqliteError::Arrow(err), ErrorClass::Permanent))
}

/// Ce que l'instruction compilée peut faire à la base.
fn effect_of(statement: &Statement<'_>) -> Effect {
    if statement.readonly() {
        Effect::ReadOnly
    } else {
        Effect::Mutating
    }
}

/// Refuse une écriture quand la demande se déclare en lecture seule.
///
/// La question est posée au **moteur** (`sqlite3_stmt_readonly`), pas au texte :
/// une analyse lexicale se ferait avoir par une vue, un déclencheur ou une table
/// virtuelle qui écrit.
fn guard_read_only(statement: &Statement<'_>, request: &ExecRequest) -> Result<()> {
    if request.limits.read_only && !statement.readonly() {
        return Err(OxynError::PolicyDenied {
            reason: "l'exécution est déclarée en lecture seule et l'instruction peut écrire"
                .to_owned(),
        });
    }
    Ok(())
}
