//! Bounded preload progress; it contains metadata identities, never row values.

use super::{MetadataLevel, MetadataScope};
use crate::{CommandId, ConnectionId, SessionId};
use serde::{Deserialize, Serialize};

/// One independent part an explicit preload can request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MetadataFacet {
    /// Listing of relation identities in the chosen native scope.
    Names,
    /// Fields and properties reported without implicit document sampling.
    Description,
    /// Declared indexes, independently of fields.
    Indexes,
    /// Declared foreign keys leaving an object.
    OutgoingKeys,
    /// Supported stored or reconstructed object definition.
    Definition,
}

impl MetadataLevel {
    /// The preset's explicit reads; inference is never a planned facet.
    #[must_use]
    pub const fn preload_facets(self) -> &'static [MetadataFacet] {
        match self {
            Self::Minimal => &[],
            Self::Structure => &[
                MetadataFacet::Description,
                MetadataFacet::Indexes,
                MetadataFacet::OutgoingKeys,
            ],
            Self::Extended => &[
                MetadataFacet::Description,
                MetadataFacet::Indexes,
                MetadataFacet::OutgoingKeys,
                MetadataFacet::Definition,
            ],
        }
    }
}

/// A terminal partial result is distinct from successful completion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MetadataPreloadState {
    /// The job owns a live command and may submit reads.
    Running,
    /// Stop was requested; protocol cleanup has not completed.
    Cancelling,
    /// The selected page completed; unsupported facets remain counted separately.
    Complete,
    /// Bounds, failures or explicit sampling leave reported work unfinished.
    Partial,
    /// The owning command, policy or source ended before completion.
    Cancelled,
    /// The requested reads failed without making new metadata available.
    Failed,
}

/// A human-requested next page, valid only for the same source generation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataPreloadCursor {
    /// Retired by relisting, invalidation or eviction.
    pub generation: uuid::Uuid,
    /// Exact namespace reviewed for this page.
    pub scope: MetadataScope,
    /// A changed preference invalidates continuation.
    pub policy_revision: u64,
    /// Last fully processed relation's exact name; never a display label.
    pub after: String,
}

/// Reviewed source/policy and an optional explicit continuation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MetadataPreloadRequest {
    /// Existing connection, never opened by preloading.
    pub connection: ConnectionId,
    /// Exact live catalog/console owner.
    pub session: SessionId,
    /// Observed native catalog/namespace identity.
    pub scope: MetadataScope,
    /// Policy revision reviewed by the human.
    pub expected_revision: u64,
    /// The human explicitly requested refreshing already available metadata.
    #[serde(default)]
    pub refresh: bool,
    /// Continuation is never followed without another human request.
    pub after: Option<MetadataPreloadCursor>,
}

impl MetadataPreloadRequest {
    /// Validates bounded input without interpreting or folding identifiers.
    ///
    /// # Errors
    /// Invalid scope or continuation identifiers, or an excessive revision.
    pub fn validate(&self) -> crate::Result<()> {
        self.scope.validate()?;
        if self.expected_revision > super::MAX_METADATA_POLICY_REVISION {
            return Err(crate::OxynError::Config(
                "invalid metadata policy revision".into(),
            ));
        }
        if let Some(cursor) = &self.after {
            super::validate_segment(&cursor.after)?;
            if cursor.scope != self.scope || cursor.policy_revision != self.expected_revision {
                return Err(crate::OxynError::Config(
                    "metadata continuation no longer matches the reviewed scope/policy".into(),
                ));
            }
        }
        Ok(())
    }
}

/// What one window's explicit preload has actually done.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPreloadProgress {
    /// Tracked command; another window cannot inspect/cancel its live progress.
    pub command: CommandId,
    /// Ordinary connection authority, unchanged by this job.
    pub connection: ConnectionId,
    /// Exact live metadata owner; no sibling fallback.
    pub session: SessionId,
    /// Exact native scope selected by the human.
    pub scope: MetadataScope,
    /// Preset resolved from the checked persisted policy.
    pub level: MetadataLevel,
    /// Policy the user actually reviewed before starting.
    pub policy_revision: u64,
    /// Current or terminal state.
    pub state: MetadataPreloadState,
    /// Planned facets once names are known.
    pub planned: u32,
    /// Read scopes submitted, including waiting/cancelled scopes, excluding unsupported facets.
    /// A scope can require more than one provider call; this is not network accounting.
    pub reads: u32,
    /// Successfully read facets, not a promise that eviction kept every value.
    pub loaded: u32,
    /// Fresh parts reused without a provider read.
    pub cached: u32,
    /// Failed read scopes, without raw server diagnostics or data.
    pub failed: u32,
    /// Unsupported or explicitly sample-required parts.
    pub unavailable: u32,
    /// Unavailable descriptions that require a separate explicit sample.
    pub sampling_required: u32,
    /// Names observed in this scope, unknown until its listing succeeds.
    pub known_relations: Option<u32>,
    /// Relations outside this job's fixed 128-relation bound.
    pub unread_relations: u32,
    /// Terminal count of selected facets still held fresh; unknown until page end.
    pub available_facets: Option<u32>,
    /// Terminal invalidated-facet count; unknown until page end.
    pub stale_facets: Option<u32>,
    /// Terminal supported-but-unread/evicted count inside this page, not outside it.
    pub unread_facets: Option<u32>,
    /// Encoded internal metadata bytes, not protocol/network traffic.
    pub metadata_bytes: u64,
    /// Wall-clock elapsed time, including cancellation cleanup.
    pub elapsed_ms: u64,
    /// A closed set is not promised: future stop reasons remain informative.
    pub stopped: Option<String>,
    /// Explicit continuation; never followed automatically to defeat eviction.
    pub next: Option<MetadataPreloadCursor>,
    /// First bounded failures, identified without raw server diagnostics.
    pub failures: Vec<MetadataPreloadIssue>,
    /// Failures outside the bounded detail list remain counted explicitly.
    pub omitted_failures: u32,
}

/// Safe partial-failure identity; no query, row or credential is retained.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MetadataPreloadIssue {
    /// Absent for the scope's name listing.
    pub relation: Option<String>,
    /// The independent part that failed.
    pub facet: MetadataFacet,
}
