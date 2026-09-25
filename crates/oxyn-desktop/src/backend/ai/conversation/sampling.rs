//! An agent asks for a row sample: the screen, the wait, the read.
//!
//! What the `request_sample` tool becomes once `oxyn-ai` has translated it and
//! refused it outside `Sampled` — for the internal assistant and an external
//! agent alike, since both reach Oxyn through [`AgentSink`]
//! ([ADR-0034](../../../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//!
//! # In this order, and nothing read before the user answers
//!
//! 1. the tier is read **now**, from the store: anything but `Sampled` is a
//!    refusal, and no screen opens ([I-04](../../../../../../CLAUDE.md#i-04));
//! 2. the relation and the columns are looked up in the local catalog — names
//!    the catalog does not know are refused, never guessed. A relation listed
//!    whose columns the cache does not hold has them read first, as the
//!    agent, through the bus: a metadata read, no row;
//! 3. the approval screen opens — the one the user's own pin opens, naming the
//!    agent that asks — and the call waits, bounded by
//!    [`samples::ASK_LIFETIME`] and by the question's stop;
//! 4. declined, expired or stopped: « the user declined », and nothing is read;
//! 5. approved: the tier is read again, the exchange is marked as leaving no
//!    memory, and the read goes to the bus as the agent — a `PreviewRelation`
//!    the driver composes and quotes ([I-10](../../../../../../CLAUDE.md#i-10)),
//!    that the `PolicyGate` decides ([I-01](../../../../../../CLAUDE.md#i-01)),
//!    projected on the ticked columns so the server returns nothing else;
//! 6. those columns are copied, the tier is read a third time, and
//!    the rows go back to `oxyn-ai` — which renders them by
//!    `ContextBuilder::build`, never here;
//! 7. only once that rendering kept them does `oxyn-ai` release the
//!    [`SampleReceipt`]: the audit is written and the panel told « sent ». A
//!    sample the rendering dropped is neither.
//!
//! # One screen per exchange, and never over a pending write
//!
//! Both before step 3, for the internal assistant and an external agent alike,
//! since both pass here:
//!
//! * **a request of this agent waits for the user** — a write it proposed:
//!   refused, no screen. Two screens stacked are how the second gets clicked
//!   without being read;
//! * **the exchange already had its screen** — approved, declined or expired:
//!   refused with the same words whatever the first answer was. A « no » the
//!   agent can ask again is a « no » until the user tires; nothing else bounds
//!   the asking — the internal loop has no per-call ceiling, and the bridge's
//!   allows eight screens.
//!
//! The answer comes from `ai_answer_sample`, a Tauri command: the user's
//! gesture. Nothing the agent sends names the request — its id travels to the
//! webview only.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use async_trait::async_trait;
use oxyn_ai::tools::SampleAsk;
use oxyn_ai::{
    DispatchOutcome, SampleReceipt, SampleRelease, context::RowSample, external::mcp::TierSource,
};
use oxyn_catalog::{CatalogCache, CatalogHandle, CatalogPath};
use oxyn_core::{Actor, CancelToken, Command, PrivacyTier};
use oxyn_exec::DispatchReport;
use oxyn_store::EgressRecord;

use super::{AgentSink, ForgetResult, REQUEST_WITHDRAWN, StoredTier, announce_call, egress_reach};
use crate::backend::ai::catalog_fill::CatalogFill;
use crate::backend::ai::samples::{self, Recipient, RecipientKind, SampleAsks};
use crate::backend::ai::threads::Thread;
use crate::ipc::ai::{AiEvent, SampleRequest};
use crate::ipc::{CatalogAddress, RelationField};
use oxyn_exec::Executor;

/// What a sink needs to put an agent's request before the user.
pub(in crate::backend::ai) struct Sampling {
    pub(in crate::backend::ai) asks: Arc<SampleAsks>,
    /// Who would receive the rows, as the audit records it.
    pub(in crate::backend::ai) recipient: Recipient,
    /// The destination, as the screen names it.
    pub(in crate::backend::ai) destination: String,
    /// Who asks, as the screen names it.
    pub(in crate::backend::ai) requested_by: String,
    /// Set when this exchange's screen opens, and never cleared: the sink
    /// lives for one exchange, so this is « the exchange had its screen ».
    spent: AtomicBool,
}

impl Sampling {
    pub(in crate::backend::ai) fn new(
        asks: Arc<SampleAsks>,
        recipient: Recipient,
        destination: String,
        requested_by: String,
    ) -> Self {
        Self {
            asks,
            recipient,
            destination,
            requested_by,
            spent: AtomicBool::new(false),
        }
    }
}

/// Said to the agent when the user said no, or said nothing in time.
const DECLINED: &str = "the user declined to share this sample. Nothing was read or sent. Work \
     from the structure, and do not ask for it again in this answer.";

/// Said when the user did not answer before [`samples::ASK_LIFETIME`].
const EXPIRED: &str = "the user declined: no answer came in time. Nothing was read or sent. \
     Work from the structure, and do not ask for it again in this answer.";

/// Said to any request after the exchange's first screen, whatever the user
/// answered there. Constant: it must not tell an approval from a refusal, nor
/// quote anything the agent asked.
const ONE_PER_ANSWER: &str = "a sample was already asked of the user in this answer, and only \
     one may be. Nothing was asked, read or sent. Work from what you have, and do not ask again.";

/// Said when a request of this agent already waits for the user's decision.
const WRITE_WAITING: &str = "a request from this agent is already waiting for the user's \
     approval; nothing was asked. Wait for the user's decision instead of asking for a sample";

/// Said when the tier, read now, lets no row value leave.
const TIER: &str = "row samples are shared only on a connection whose privacy tier is \
     `sampled`, and this one's no longer is. Nothing was read or sent; do not ask again.";

fn denied(reason: impl Into<String>) -> DispatchOutcome {
    DispatchOutcome::Denied {
        reason: reason.into(),
    }
}

/// Why [`resolve`] found no fields to offer.
enum Unresolved {
    /// Listed, but its columns are not in the cache: to read, then resolve
    /// again.
    NotDescribed(CatalogPath),
    /// Refused, with the reason the agent is given.
    Refused(String),
}

/// The relation the agent named, its path, and the fields offered.
///
/// A name, never a pattern: equal to the catalog's, under the namespace when
/// one is given. The reasons quote only what the agent wrote itself.
fn resolve(
    cache: &CatalogCache,
    namespace: Option<&str>,
    relation: &str,
    requested: &[String],
) -> Result<(CatalogPath, Vec<RelationField>), Unresolved> {
    let found: Vec<_> = cache
        .iter_relations()
        .filter(|(summary, _)| {
            summary.name() == relation
                && namespace.is_none_or(|namespace| summary.path().namespace() == Some(namespace))
        })
        .collect();
    let [(summary, described)] = found.as_slice() else {
        return Err(Unresolved::Refused(if found.is_empty() {
            format!(
                "`{relation}` is not in Oxyn's catalog of this connection; call describe_schema \
                 for its exact name. Nothing was asked."
            )
        } else {
            format!(
                "several objects are named `{relation}`; name its namespace. Nothing was asked."
            )
        }));
    };
    let Some(described) = described else {
        return Err(Unresolved::NotDescribed(summary.path()));
    };
    if let Some(unknown) = requested
        .iter()
        .find(|column| !described.fields.iter().any(|field| &field.name == *column))
    {
        return Err(Unresolved::Refused(format!(
            "`{unknown}` is not a column of `{relation}`; call describe_schema for its exact \
             name. Nothing was asked."
        )));
    }
    let fields = described
        .fields
        .iter()
        .filter(|field| requested.is_empty() || requested.contains(&field.name))
        .map(RelationField::from)
        .collect();
    Ok((summary.path(), fields))
}

impl AgentSink {
    /// The agent's request for a sample, from the screen to the rows. See the
    /// module: every refusal comes before anything is read.
    pub(super) async fn sample(
        &self,
        actor: Actor,
        ask: SampleAsk,
        cancel: &CancelToken,
    ) -> DispatchOutcome {
        let (call, _, _) = announce_call(&self.thread, self.node, None);
        let Some(sampling) = &self.sampling else {
            return denied(
                "this conversation cannot show the user an approval screen; nothing was read",
            );
        };
        if sampling.spent.load(Ordering::Acquire) {
            return denied(ONE_PER_ANSWER);
        }
        if self.awaits_the_user(actor) {
            return denied(WRITE_WAITING);
        }
        let Some(scope) = self.thread.scope() else {
            return denied("no question is in progress; nothing was read");
        };
        let rows_asked = ask.rows();
        let SampleAsk { command, columns } = ask;
        let Command::PreviewRelation {
            connection,
            session,
            namespace,
            relation,
            ..
        } = command
        else {
            return denied("this Oxyn build cannot read that sample; nothing was read");
        };
        // The scope's connection, always: `oxyn-ai` built the command from it,
        // and a sink that trusted the command would be the second place that
        // decides where a read goes.
        if connection != scope.connection {
            return denied("a sample is read on this conversation's connection only");
        }
        let tiers = StoredTier {
            executor: Arc::clone(&self.executor),
            connection,
        };
        if tiers.current().await != Some(PrivacyTier::Sampled) {
            return denied(TIER);
        }
        let Some(catalog) = self.executor.catalog(connection) else {
            return denied("this connection has no catalog yet; nothing was asked");
        };
        let resolved = resolve(&catalog.read(), namespace.as_deref(), &relation, &columns);
        let resolved = match resolved {
            Err(Unresolved::NotDescribed(path)) => {
                // The agent-bound sink: the read is the agent's, as its
                // `describe_schema` completions are.
                let fill = CatalogFill {
                    sink: &self.sink,
                    actor,
                    connection,
                    catalog: Arc::clone(&catalog),
                };
                if fill.describe(&path, cancel).await.is_err() {
                    // The server's words stay out of the prompt and the log:
                    // they may name what the agent did not.
                    tracing::debug!("the columns of a sampled relation were not read");
                    return denied(format!(
                        "the columns of `{relation}` could not be read from the server. \
                         Nothing was asked."
                    ));
                }
                resolve(&catalog.read(), namespace.as_deref(), &relation, &columns)
            }
            other => other,
        };
        let (path, fields) = match resolved {
            Ok(resolved) => resolved,
            Err(Unresolved::Refused(reason)) => return denied(reason),
            Err(Unresolved::NotDescribed(_)) => {
                return denied(format!(
                    "the columns of `{relation}` could not be read from the server. \
                     Nothing was asked."
                ));
            }
        };
        let offered: Vec<String> = fields.iter().map(|field| field.name.clone()).collect();
        let open = match sampling.asks.open(connection, offered) {
            Ok(open) => open,
            Err(refused) => return denied(refused.to_string()),
        };

        // Shown only while the question is open, under its lock: a screen
        // opened for an answer marked finished would be approved by reflex.
        {
            let question = self.question.0.lock();
            if !*question {
                return denied(REQUEST_WITHDRAWN);
            }
            // Checked again at the last moment: minutes of catalog lookup can
            // separate the first check from here, and two calls can race.
            if self.awaits_the_user(actor) {
                return denied(WRITE_WAITING);
            }
            if sampling.spent.swap(true, Ordering::AcqRel) {
                return denied(ONE_PER_ANSWER);
            }
            self.thread.emit(
                self.node,
                AiEvent::SampleRequested {
                    call,
                    request: SampleRequest {
                        id: open.id.clone(),
                        requested_by: Some(sampling.requested_by.clone()),
                        source: path.to_string(),
                        address: CatalogAddress::of(&path),
                        rows: rows_asked,
                        fields,
                        destination: sampling.destination.clone(),
                        reach: sampling.recipient.reach.into(),
                    },
                },
            );
        }
        // From here the screen is shown, and whatever ends this call closes
        // it — an answer, but also the call's future dropped mid-wait.
        let mut screen = Shown {
            open,
            thread: Arc::clone(&self.thread),
            node: self.node,
            approved: false,
        };
        let answer = tokio::select! {
            answer = &mut screen.open.answer => answer.map_err(|_| DECLINED),
            () = tokio::time::sleep(samples::ASK_LIFETIME) => Err(EXPIRED),
            () = cancel.cancelled() => Err(DECLINED),
        };
        screen.approved = answer.is_ok();
        // Withdrawn and closed now, answered or not: nothing of it stays
        // approvable, and the panel stops showing it.
        drop(screen);
        let columns = match answer {
            Ok(columns) => columns,
            Err(reason) => return denied(reason),
        };

        // Read again: minutes may have passed on the screen.
        if tiers.current().await != Some(PrivacyTier::Sampled) {
            return denied(TIER);
        }
        // First, before anything is read: whatever happens next, this exchange
        // leaves no memory.
        self.thread.withhold_memory(self.node);
        let rows = match self
            .read(
                actor,
                (connection, session),
                &path,
                &columns,
                rows_asked,
                cancel,
            )
            .await
        {
            Ok(rows) => rows,
            Err(reason) => return denied(reason),
        };
        if tiers.current().await != Some(PrivacyTier::Sampled) {
            return denied(TIER);
        }
        let (read_by, rows) = rows;
        let count = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        // Not written here: `oxyn-ai` may still drop the rows when it renders
        // them, and an entry written now would record a send that never was.
        let mut egress = EgressRecord::new(
            connection,
            path.to_string(),
            columns.clone(),
            count,
            sampling.recipient.provider.clone(),
            egress_reach(sampling.recipient.reach),
        )
        .read_by(read_by);
        if sampling.recipient.kind == RecipientKind::Provider {
            egress = egress.with_model(&sampling.recipient.model);
        }
        if let Some(conversation) = self.thread.conversation() {
            egress = egress.in_conversation(conversation, self.thread.stored_node(self.node));
        }
        let release = Release {
            executor: Arc::clone(&self.executor),
            thread: Arc::clone(&self.thread),
            node: self.node,
            approved: AiEvent::SampleApproved {
                rows: count,
                columns: u32::try_from(columns.len()).unwrap_or(u32::MAX),
            },
            egress: parking_lot::Mutex::new(Some(egress)),
        };
        DispatchOutcome::Sampled {
            catalog: CatalogHandle::new(catalog),
            sample: RowSample::new(path, columns, rows),
            receipt: SampleReceipt::new(Arc::new(release)),
        }
    }

    /// Does a request of `actor` wait for the user's decision? Asked of the
    /// executor, which holds the requests — the bridge asks it the same way.
    fn awaits_the_user(&self, actor: Actor) -> bool {
        self.executor
            .approvals()
            .pending()
            .iter()
            .any(|request| request.actor == actor && !request.is_expired())
    }

    /// Reads the approved sample through the agent's own sink — the executor,
    /// the `PolicyGate`, the journal —, projected on the ticked columns, and
    /// copies them.
    ///
    /// Answers the read's command id, for the audit, and the rows. The
    /// server's words never come back: an error can quote the cell it refused.
    async fn read(
        &self,
        actor: Actor,
        (connection, session): (oxyn_core::ConnectionId, oxyn_core::SessionId),
        path: &CatalogPath,
        columns: &[String],
        rows: u32,
        cancel: &CancelToken,
    ) -> Result<(oxyn_core::CommandId, Vec<Vec<oxyn_core::ScalarValue>>), String> {
        let Some(command) = samples::sample_read((connection, session), path, columns, rows) else {
            return Err("this is not a relation; nothing was read".to_owned());
        };
        match self.sink.dispatch(actor, command, cancel).await {
            DispatchReport::Completed {
                command,
                result: Some(result),
                ..
            } => {
                // Forgotten once copied: these rows were read for a prompt,
                // and no grid shows them.
                let _forget = ForgetResult {
                    executor: Arc::clone(&self.executor),
                    result,
                };
                let Some(buffer) = self.executor.result(result) else {
                    return Err("the sample's rows are gone; nothing was sent".to_owned());
                };
                let columns = columns.to_vec();
                let cancel = cancel.clone();
                let copied = tokio::task::spawn_blocking(move || {
                    samples::copy_rows(&buffer, &columns, rows, &cancel)
                })
                .await
                .ok()
                .flatten()
                .ok_or_else(|| {
                    "an approved column is no longer in this relation; nothing was sent".to_owned()
                })?;
                Ok((command, copied))
            }
            DispatchReport::AwaitingApproval { command, .. } => {
                // Not left waiting: the user already decided on this sample,
                // and a second screen for it would be clicked by reflex.
                let _withdrawn = self.executor.reject(command);
                Err(
                    "reading the sample needs an approval of its own on this connection; \
                     nothing was read"
                        .to_owned(),
                )
            }
            DispatchReport::Denied { reason, .. } => Err(format!(
                "reading the sample was refused: {reason}. Nothing was sent"
            )),
            _ => Err(
                "reading the approved sample failed; nothing was sent. The command \
                      journal has the error"
                    .to_owned(),
            ),
        }
    }
}

/// An approval screen on show, closed when this drops — however the call ends.
///
/// Closing it after the wait was not enough: an external agent that hangs up
/// mid-wait has its call's future dropped (hyper abandons the service), and
/// nothing after the `select!` ran. The screen stayed open, the user approved
/// for nobody, and was told the request was gone only on clicking.
struct Shown {
    open: samples::OpenAsk,
    thread: Arc<Thread>,
    node: u32,
    /// Set once the user approved; a drop without it is a refusal.
    approved: bool,
}

impl Drop for Shown {
    fn drop(&mut self) {
        // Withdrawn first: an approval sent between the two would otherwise
        // land on a request the panel already calls closed.
        self.open.withdraw();
        self.thread.emit(
            self.node,
            AiEvent::SampleAnswered {
                request: self.open.id.clone(),
                approved: self.approved,
            },
        );
    }
}

/// What a sample that really leaves does: the audit entry, then the panel's
/// « sent ». Run by `oxyn-ai` once the rendering kept the rows, and only then.
struct Release {
    executor: Arc<Executor>,
    thread: Arc<Thread>,
    node: u32,
    approved: AiEvent,
    /// Taken on release: a second call records nothing more.
    egress: parking_lot::Mutex<Option<EgressRecord>>,
}

#[async_trait]
impl SampleRelease for Release {
    async fn release(&self) -> bool {
        let Some(egress) = self.egress.lock().take() else {
            return false;
        };
        if !super::append_egress(&self.executor, egress).await {
            return false;
        }
        self.thread.emit(self.node, self.approved.clone());
        true
    }
}
