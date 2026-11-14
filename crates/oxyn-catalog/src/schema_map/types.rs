//! Bounded metadata for local maps; no row values, layout or execution capability.
use crate::{CatalogGenerationId, RelationKind, SchemaSampleLimits};
use oxyn_core::{ConnectionId, SessionId};
use serde::Serialize;

/// Existing ERD object display limit, shared by every map consumer.
pub const MAX_MAP_OBJECTS: usize = 40;
/// Existing per-object ERD field display limit, including nested fields here.
pub const MAX_MAP_FIELDS: usize = 12;
/// Link publication/rendering guard; omissions remain explicit.
pub const MAX_MAP_LINKS: usize = 200;

/// Exact native optional levels, as an IPC object rather than a parsed label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapAddress {
    /// Catalog/database, when this native level exists.
    pub catalog: Option<String>,
    /// Namespace/schema, when this native level exists.
    pub namespace: Option<String>,
    /// Object; absent on a native container.
    pub relation: Option<String>,
}
/// Immutable authority owner and transient cache version for one projection.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapSource {
    /// Original connection, never substituted by a same-named source.
    pub connection: ConnectionId,
    /// Exact live catalog or console owner.
    pub session: SessionId,
    /// Epoch observed while reading the cache.
    pub generation: CatalogGenerationId,
}
/// Value state is separate from native support and request failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MapFreshness {
    /// No retained value has been read.
    Unread,
    /// Retained metadata has not been invalidated.
    Fresh,
    /// Retained metadata is invalidated, not current source truth.
    Stale,
}
/// Why a field appears, never a guess based on its name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FieldOrigin {
    /// The source's metadata reports the field/mapping.
    Reported,
    /// A human explicitly requested bounded document inference.
    Sampled,
    /// A key's preview layout, not a declared collection schema.
    PreviewLayout,
}
/// One field or nested member; paths retain literal dots and nested segments.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapField {
    /// Source/owner/object/field-path identity, not the display name.
    pub key: String,
    /// Field key containing this member, when nested.
    pub parent: Option<String>,
    /// Exact segments, never split from a dotted name.
    pub path: Vec<String>,
    /// Literal field name.
    pub name: String,
    /// Reported type text, bounded for rendering.
    pub raw_type: String,
    /// Whether the displayed type text was shortened.
    pub type_shortened: bool,
    /// Native nullability, without inferring from row values.
    pub nullable: bool,
    /// Reported membership in a primary key.
    pub primary_key: bool,
    /// Reported or explicitly observed provenance.
    pub origin: FieldOrigin,
}
/// An object remains useful when fields or relationships are unavailable.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapObject {
    /// Stable complete identity across redraws of the same owner.
    pub key: String,
    /// Exact native address.
    pub address: MapAddress,
    /// Native reported object kind.
    pub kind: RelationKind,
    /// Browsable records, not permission to execute them automatically.
    pub holds_records: bool,
    /// Selected bounded field set; no defaults/comments/sample values.
    pub fields: Vec<MapField>,
    /// Fields omitted by the display/input bound.
    pub omitted_fields: usize,
    /// Description freshness, including known empty.
    pub fields_state: MapFreshness,
    /// Whether describing unread fields requires explicit inference.
    pub sampling_required: bool,
    /// Outgoing keys' freshness, independent from fields.
    pub keys_state: MapFreshness,
    /// Native key introspection support.
    pub keys_supported: bool,
}
/// A native parent or source container, not an invented SQL schema.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapContainer {
    /// Owner-qualified parent identity.
    pub key: String,
    /// Native path, preserving absent levels.
    pub address: MapAddress,
    /// Native container level or the explicit source root.
    pub level: String,
}
/// Link meanings must remain separate in both the canvas and list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MapLinkKind {
    /// A source-reported foreign key, not necessarily enforced on existing data.
    ForeignKey,
    /// A provider-reported technical dependency; no DDL/name guessing.
    Dependency,
    /// Native catalog/namespace membership.
    Membership,
    /// Structural parent/member nesting.
    Containment,
}
/// A reference can retain an unloaded endpoint without silently discovering it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapLink {
    /// Owner-qualified link identity, including ordered composite members.
    pub key: String,
    /// Semantic provenance of this link.
    pub kind: MapLinkKind,
    /// Object/container/field source key.
    pub from: String,
    /// Reported target key, including an unresolved object.
    pub to: String,
    /// Exact target object/container path.
    pub target: MapAddress,
    /// Whether its target object/container/field is projected.
    pub target_loaded: bool,
    /// Reported name, never executed.
    pub name: String,
    /// Carrying fields, preserving composite order.
    pub fields: Vec<String>,
    /// Referenced fields, preserving composite order.
    pub referenced_fields: Vec<String>,
    /// A malformed native key is not repaired by pairing guesses.
    pub well_formed: bool,
    /// Freshness of the metadata establishing this link.
    pub freshness: MapFreshness,
    /// Only known uniqueness is retained; absent means unknown cardinality.
    pub source_unique: Option<bool>,
    /// Only explicitly proven target uniqueness; absent means unknown.
    pub target_unique: Option<bool>,
}
/// Counts distinguish bounded omissions from the absence of reported metadata.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapCoverage {
    /// Known objects considered within this projection's requested set.
    pub known_objects: usize,
    /// Projected object cards, not a complete database schema count.
    pub drawn_objects: usize,
    /// Known objects outside display/input bounds.
    pub omitted_objects: usize,
    /// Field members omitted from the selected objects.
    pub omitted_fields: usize,
    /// Links omitted by bounds, never silently invented or truncated.
    pub omitted_links: usize,
    /// Objects whose field description is unread.
    pub unread_objects: usize,
    /// Objects with invalidated descriptions or keys.
    pub stale_objects: usize,
    /// Reported references whose target is not drawn.
    pub unresolved_links: usize,
}
/// A pure catalog snapshot. Incoming edges cover loaded outgoing keys only.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaMapProjection {
    /// Source ownership and epoch.
    pub source: MapSource,
    /// Existing native parents of selected objects.
    pub containers: Vec<MapContainer>,
    /// Bounded objects, including objects without fields/keys.
    pub objects: Vec<MapObject>,
    /// Reported relationships, membership and containment.
    pub links: Vec<MapLink>,
    /// Explicit counts of omitted/unread/stale metadata.
    pub coverage: MapCoverage,
    /// False until an actual provider exposes structured dependency metadata.
    pub dependencies_supported: bool,
    /// Bounds shown before an optional explicit inference action.
    pub sample_limits: Option<SchemaSampleLimits>,
}

/// Independent metadata demand whose failure must not erase other objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MapFacet {
    /// Native hierarchy or object identities.
    Names,
    /// Field description, excluding implicit inference.
    Fields,
    /// Reported outgoing foreign keys.
    OutgoingKeys,
    /// Explicit neighbor discovery, never claimed exhaustive.
    IncomingKeys,
}
/// Why an individual part was not loaded; no raw provider diagnostic/value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MapIssueKind {
    /// A supported native read failed or was denied.
    Failed,
    /// The live session does not expose this optional facet.
    Unavailable,
    /// The requested path was not observed or referenced in this source.
    NotObserved,
    /// Native identity or graph bounds prevented inclusion.
    Omitted,
}
/// Partial read information, safe to render next to retained metadata.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MapLoadIssue {
    /// Actual native identity involved, not another source's label.
    pub address: MapAddress,
    /// Failed/unavailable metadata part.
    pub facet: MapFacet,
    /// Structured recovery category.
    pub kind: MapIssueKind,
}
/// One bounded human map load and its actual resulting cache coverage.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SchemaMapLoaded {
    /// Source-qualified pure graph snapshot.
    pub projection: SchemaMapProjection,
    /// Individual unavailable/failed objects do not erase successful nodes.
    pub issues: Vec<MapLoadIssue>,
    /// Native read scopes submitted, not network round trips.
    pub reads: u32,
    /// A native container level needs the user to choose its child scope.
    pub choose_namespace: bool,
    /// Cancellation/time/source-change state, if the pass did not settle normally.
    pub stopped: Option<String>,
}
