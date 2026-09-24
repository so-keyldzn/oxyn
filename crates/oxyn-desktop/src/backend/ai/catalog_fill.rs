//! Completing the local catalog before the assistant reads it.
//!
//! The tree loads the catalog on demand: on connect, the server and its first
//! level; a schema's relations when the user expands it; a table's fields when
//! they open it. The assistant reads the same cache, so without this module
//! what it knows of a database is what the user happened to click — « 0 of 0
//! known relations » on a database of eleven tables
//! ([ADR-0036](../../../../../docs/adr/0036-l-assistant-complete-le-catalogue.md)).
//!
//! Before a context is built, and before `describe_schema` reads the cache,
//! Oxyn itself reads what is missing, and nothing more:
//!
//! 1. the relations of each schema never listed — at most [`MAX_LISTINGS`];
//! 2. the fields, indexes and foreign keys of the relations the gate will
//!    describe — mentions first, then what the question finds, at most
//!    [`MAX_DESCRIPTIONS`], the gate's own bound;
//!
//! all within [`FILL_DEADLINE`]. What does not fit is not an error: the
//! context leaves with what is loaded, and the gate says what is not.
//!
//! # Through the bus, as metadata
//!
//! Each read is a `RefreshCatalogScope` submitted through [`ExecutorSink`], so
//! the `PolicyGate` decides and the journal records it
//! ([I-01](../../../../../CLAUDE.md#i-01)) — the command the tree's expansion
//! submits, never a driver call. It reads no row. The actor is the one at the
//! origin of the read: [`Actor::Human`] for what the user's question makes Oxyn
//! read, the agent's own for what its tool call makes Oxyn read.
//!
//! # Serial, on purpose
//!
//! The executor reads a connection's catalog on the one session reserved for
//! it, under one lock: reads sent in parallel would queue on that lock and win
//! nothing. The bounds are what keep a question on a database of ten thousand
//! tables from waiting: a few dozen reads, a few seconds.
//!
//! # Nothing is read twice
//!
//! A level already read, and not invalidated since, is not read again: the
//! freshness is the cache's own ([`Freshness`]). A read that failed is not
//! retried within the same completion; the next question may try it again,
//! once, within its own bounds.

use std::collections::HashSet;
use std::time::Duration;

use oxyn_ai::context::wanted_relations;
use oxyn_ai::{ContextPolicy, Mention};
use oxyn_catalog::{CatalogCache, CatalogPath, CatalogScope, Freshness, SharedCatalog};
use oxyn_core::{Actor, CancelToken, Capabilities, Command, ConnectionId};
use oxyn_exec::{DispatchReport, ExecutorSink};
use tokio::time::Instant;

use crate::ipc::ai::AiEvent;

/// The longest a completion may take, reads included.
///
/// Product bound. A remote PostgreSQL at 50 ms of round trip describes a table
/// in three queries: [`MAX_DESCRIPTIONS`] of them take about four seconds,
/// the listings a few hundred milliseconds more. Past this, the user waits on
/// Oxyn rather than on the model, and a question stays better answered with
/// part of the structure — said so — than late.
pub(super) const FILL_DEADLINE: Duration = Duration::from_secs(5);

/// The most schemas whose relations one completion lists.
///
/// Product bound. A working database has a handful of schemas; one per tenant
/// can have hundreds, each listing a round trip. The next question continues
/// with those left, since they are still unread.
pub(super) const MAX_LISTINGS: usize = 32;

/// The most relations one completion describes: the gate's own bound.
///
/// Describing more than the gate can show would read for nothing. The value
/// is [`ContextPolicy::max_relations`] of the policy the gate uses on this host
/// — the default one; a test holds the two equal.
pub(super) const MAX_DESCRIPTIONS: usize = 24;

/// How long a read cancelled at the deadline gets to wind down.
///
/// The read is cancelled through its token and awaited rather than dropped:
/// the provider finishes its protocol cleanup, and the catalog session stays
/// usable. A provider that ignores cancellation is let go after this.
const CANCEL_GRACE: Duration = Duration::from_secs(2);

/// What a completion is for.
#[derive(Debug, Clone, Copy)]
pub(super) enum Want<'a> {
    /// A question that opens a context: listings, then what the gate will
    /// describe for it.
    Question {
        focus: &'a str,
        mentions: &'a [Mention],
    },
    /// A question that follows a session already told the structure: only
    /// the relations it mentions.
    Mentions(&'a [Mention]),
    /// `describe_schema`: listings, then what its search words select.
    Search(&'a str),
    /// `refresh_catalog`, after the server level was read again: the
    /// relations of every schema, listed again even when fresh.
    Relist,
    /// The first `@` of the panel: the names of the relations, for the
    /// user to choose from — listings only, nothing described.
    Names,
}

impl Want<'_> {
    const fn lists(self) -> bool {
        !matches!(self, Self::Mentions(_))
    }

    const fn relists(self) -> bool {
        matches!(self, Self::Relist)
    }
}

/// Why a completion stopped before it was done.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Stop {
    Deadline,
    Cancelled,
}

impl Stop {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Deadline => "deadline",
            Self::Cancelled => "cancelled",
        }
    }
}

/// What a completion did, and what is still missing — counts only.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct Filled {
    /// Reads submitted, successful or not. Zero: nothing was missing.
    pub attempted: usize,
    /// Listings read: the server level, a catalog's schemas, a schema's relations.
    pub listed: usize,
    /// Relations described.
    pub described: usize,
    /// Reads that failed or were refused.
    pub failed: usize,
    /// Relations the gate will describe by name only.
    pub not_loaded: usize,
    /// Schemas whose relations were never listed.
    pub unlisted: usize,
    pub stopped: Option<Stop>,
}

impl Filled {
    /// The panel's account of the completion, once it is over.
    pub(super) fn event(self) -> AiEvent {
        AiEvent::CatalogRead {
            listed: self.listed,
            described: self.described,
            failed: self.failed,
            not_loaded: self.not_loaded,
            unlisted: self.unlisted,
            stopped: self.stopped.map(Stop::as_str),
        }
    }
}

/// One completion of one connection's catalog, by one actor.
pub(super) struct CatalogFill<'a> {
    /// The scheduler's sink: unbound for the user, bound to the agent for
    /// its tool calls — where it refuses any other actor.
    pub sink: &'a ExecutorSink,
    pub actor: Actor,
    pub connection: ConnectionId,
    pub catalog: SharedCatalog,
}

enum Step {
    Done,
    Failed,
    Stopped(Stop),
}

impl CatalogFill<'_> {
    /// Reads what `want` needs and the cache does not hold, within the bounds.
    ///
    /// `emit` receives [`AiEvent::CatalogReading`] before the first read and
    /// the account after the last one — nothing when nothing was missing.
    /// Never fails: a failed read leaves its level unread, and the gate says
    /// so.
    pub(super) async fn run(
        &self,
        want: Want<'_>,
        cancel: &CancelToken,
        emit: &(dyn Fn(AiEvent) + Send + Sync),
    ) -> Filled {
        self.run_until(want, cancel, emit, Instant::now() + FILL_DEADLINE)
            .await
    }

    /// [`Self::run`], against a deadline given by the caller — a test's.
    pub(super) async fn run_until(
        &self,
        want: Want<'_>,
        cancel: &CancelToken,
        emit: &(dyn Fn(AiEvent) + Send + Sync),
        deadline: Instant,
    ) -> Filled {
        let policy = ContextPolicy::default();
        let mut tried: HashSet<CatalogScope> = HashSet::new();
        if want.relists() {
            // The tool's own command has just read the server level — and
            // with it the schemas, or the relations, of a source without a
            // catalog level: the levels whose empty path reads the root again.
            tried.extend([
                CatalogScope::Server,
                CatalogScope::Catalog(CatalogPath::empty()),
                CatalogScope::Namespace(CatalogPath::empty()),
            ]);
        }
        let mut filled = Filled::default();
        let mut listings = 0usize;
        let mut descriptions = 0usize;
        loop {
            if cancel.is_cancelled() {
                filled.stopped = Some(Stop::Cancelled);
                break;
            }
            if Instant::now() >= deadline {
                filled.stopped = Some(Stop::Deadline);
                break;
            }
            // The read lock is released before anything awaits. The command is
            // the tree's own for that level: same scope, same capabilities.
            let next = {
                let cache = self.catalog.read();
                next_read(&cache, want, &policy, &tried, listings, descriptions).map(|scope| {
                    let capabilities = cache
                        .server_info()
                        .map_or(Capabilities::empty(), |server| server.capabilities);
                    let command = crate::catalog::refresh_command(
                        self.connection,
                        scope.path(),
                        capabilities,
                    );
                    (scope, command)
                })
            };
            let Some((scope, command)) = next else {
                break;
            };
            if filled.attempted == 0 {
                emit(AiEvent::CatalogReading);
            }
            filled.attempted += 1;
            let describing = matches!(scope, CatalogScope::Relation(_));
            if describing {
                descriptions += 1;
            } else {
                listings += 1;
            }
            tried.insert(scope);
            match self.read(command, cancel, deadline).await {
                Step::Done if describing => filled.described += 1,
                Step::Done => filled.listed += 1,
                Step::Failed => filled.failed += 1,
                Step::Stopped(stop) => {
                    filled.stopped = Some(stop);
                    break;
                }
            }
        }
        {
            let cache = self.catalog.read();
            filled.unlisted = cache.unlisted_count();
            filled.not_loaded = wanted(&cache, want, &policy)
                .iter()
                .filter(|path| !fetched(cache.freshness(&CatalogScope::Relation((*path).clone()))))
                .count();
        }
        if filled.attempted > 0 {
            emit(filled.event());
        }
        filled
    }

    /// Submits one read, and stops it at the deadline or with the question.
    async fn read(&self, command: Command, cancel: &CancelToken, deadline: Instant) -> Step {
        // A child: the deadline stops this read, never the question.
        let operation = cancel.child();
        let dispatch = self.sink.dispatch(self.actor, command, &operation);
        tokio::pin!(dispatch);
        let report = tokio::select! {
            biased;
            report = &mut dispatch => report,
            () = tokio::time::sleep_until(deadline) => {
                operation.cancel();
                let _ = tokio::time::timeout(CANCEL_GRACE, dispatch).await;
                return Step::Stopped(Stop::Deadline);
            }
        };
        match report {
            DispatchReport::Completed { .. } => Step::Done,
            _ if cancel.is_cancelled() => Step::Stopped(Stop::Cancelled),
            // A metadata read is not held for approval today. Should a policy
            // ever hold one, the request would wait under a question that does
            // not show it: it is withdrawn, and the level stays unread.
            DispatchReport::AwaitingApproval { command, .. } => {
                let _withdrawn = self.sink.executor().reject(command);
                tracing::warn!("a catalog read for the assistant was held for approval");
                Step::Failed
            }
            // The server's words are not logged: they may name its objects.
            DispatchReport::Failed { class, .. } => {
                tracing::debug!(?class, "a catalog read for the assistant failed");
                Step::Failed
            }
            _ => Step::Failed,
        }
    }
}

/// The next level to read, or `None` when the completion is done.
fn next_read(
    cache: &CatalogCache,
    want: Want<'_>,
    policy: &ContextPolicy,
    tried: &HashSet<CatalogScope>,
    listings: usize,
    descriptions: usize,
) -> Option<CatalogScope> {
    if want.lists() {
        // The server level first: without the session's capabilities, the
        // cache cannot tell which levels exist.
        let server = CatalogScope::Server;
        let server_read = cache.server_info().is_some() && fetched(cache.freshness(&server));
        if !server_read && !tried.contains(&server) {
            return Some(server);
        }
        if listings < MAX_LISTINGS
            && let Some(scope) = cache.listing_scopes().into_iter().find(|scope| {
                !tried.contains(scope) && (want.relists() || !fetched(cache.freshness(scope)))
            })
        {
            return Some(scope);
        }
    }
    if descriptions >= MAX_DESCRIPTIONS {
        return None;
    }
    wanted(cache, want, policy)
        .into_iter()
        .map(CatalogScope::Relation)
        .find(|scope| !tried.contains(scope) && !fetched(cache.freshness(scope)))
}

/// The relations the gate will describe for `want`, as it selects them.
fn wanted(cache: &CatalogCache, want: Want<'_>, policy: &ContextPolicy) -> Vec<CatalogPath> {
    match want {
        Want::Question { focus, mentions } => {
            wanted_relations(cache, policy, focus, mentions, true)
        }
        Want::Mentions(mentions) => wanted_relations(cache, policy, "", mentions, false),
        Want::Search(focus) => wanted_relations(cache, policy, focus, &[], true),
        Want::Relist | Want::Names => Vec::new(),
    }
}

/// Read, and not invalidated since. A level never read, or read before a DDL
/// Oxyn ran, is to read.
const fn fetched(freshness: Freshness) -> bool {
    matches!(freshness, Freshness::Fetched(_))
}

#[cfg(test)]
mod tests;
