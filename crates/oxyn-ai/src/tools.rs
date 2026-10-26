//! The tools offered to agents — that is, the core's `Command`s.
//!
//! **Non-negotiable architecture point** (ADR-0004, I-01): there is no second
//! API "for the AI". A tool is a translation of a model call into an existing
//! [`Command`], and nothing else. A tool that cannot be expressed as a
//! `Command` is not a missing tool: it is a missing command, and that is
//! settled in `oxyn-core`, not here.
//!
//! # What the model cannot say
//!
//! A tool's arguments carry **only** what the model has the right to choose.
//! Everything else comes from the [`ToolScope`], which the caller builds:
//!
//! | What the model writes | What the scope imposes |
//! |---|---|
//! | the statement's text | the connection, the session, the language |
//! | | the intent and the risk, **classified by `oxyn-query`** |
//! | | the [`ExecLimits`], derived from that classification |
//!
//! Direct consequences: an agent cannot target a connection other than the one
//! the user opened it on — hence no exfiltration to a third-party database —
//! and it cannot declare itself read-only to bypass the `PolicyGate`, since it
//! never writes that field. The JSON schemas carry
//! `additionalProperties: false` (`deny_unknown_fields`): an invented argument
//! makes the translation fail instead of being silently ignored.
//!
//! # What is deliberately not exposed
//!
//! * [`Command::CreateConnection`], [`Command::UpdateConnection`],
//!   [`Command::DeleteConnection`] — an agent that could create a connection
//!   to the host of its choice would have an exfiltration channel. The
//!   `PolicyGate` would submit them to approval; here they do not exist at
//!   all, which is stronger than a refusal.
//! * [`Command::Export`] — the agent would choose a file path, hence write
//!   wherever it wants on the user's disk.
//! * [`Command::Cancel`] — cancelling is a user act; and the execution handle
//!   is never in the conversation.
//! * [`Command::PreviewRelation`] **directly** — an agent that read rows would
//!   see them arrive in the grid, never in its conversation; it would gain
//!   nothing. It only triggers it through [`REQUEST_SAMPLE`], which has it
//!   preceded by the user's approval, column by column
//!   ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//! * [`Command::OpenDocument`] and [`Command::WriteDocument`] —
//!   `// TODO(phase 4)`: they need a `DocumentId` the scope would have to
//!   carry, which only makes sense once workspace documents are written.
//!
//! # The classification made here is not the one that protects
//!
//! `oxyn-exec` systematically **reclassifies** the text before submitting to
//! the `PolicyGate` (ARCHITECTURE §8): the intent carried by a `Command` comes
//! from the caller, and an agent is a caller. What this module classifies
//! serves to set honest limits and to feed the preview, not to decide.

use std::fmt;

use oxyn_core::{
    Command, ConnectionId, ExecLimits, ExecRequest, MAX_CATALOG_FOCUS_BYTES, PreviewShape,
    QueryLanguage, SessionId,
};
use oxyn_llm::{ToolCall, ToolSpec};
use schemars::{JsonSchema, SchemaGenerator, generate::SchemaSettings};
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::error::AiError;

/// Name of the tool that runs a statement.
pub const EXECUTE_QUERY: &str = "execute_query";

/// Name of the tool that re-reads the catalog from the server.
pub const REFRESH_CATALOG: &str = "refresh_catalog";

/// Name of the tool that describes the database's structure, from the local
/// catalog.
pub const DESCRIBE_SCHEMA: &str = "describe_schema";

/// The instruction that tells the model how to have Oxyn draw an
/// entity-relationship diagram.
///
/// A macro and not a constant: `concat!` only takes literals, and tool
/// descriptions are literals. It keeps **one** sentence for the description
/// of [`DESCRIBE_SCHEMA`] and an external agent's prompt; the shipped agent
/// files repeat it word for word, which a test checks — two wordings would
/// end up stating two formats, and the panel only draws one.
macro_rules! erd_hint {
    () => {
        "To show an entity-relationship diagram, write a fenced code block whose language \
         is `erd` and that lists one table name per line, nothing else: Oxyn draws the \
         diagram from its catalog. Do not draw one in ASCII or in another diagram language."
    };
}

/// The instruction of `erd_hint!`, for whoever composes a prompt at runtime:
/// the same sentence as that of the system prompts and of [`DESCRIBE_SCHEMA`].
pub const ERD_HINT: &str = erd_hint!();

/// Name of the tool through which an agent **requests** a sample of rows.
///
/// Requesting is not reading: nothing is read or sent before the user has
/// checked, in the panel, the columns that leave
/// ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
pub const REQUEST_SAMPLE: &str = "request_sample";

/// Rows requested when the agent says nothing: those of a sample the user pins
/// themselves. Enough to illustrate a shape of data.
pub const DEFAULT_SAMPLE_ROWS: u32 = 5;

/// The most rows a requested sample carries, whatever the agent writes.
///
/// A product bound: a sample illustrates values, it does not serve to extract a
/// table. Every row is a row that leaves the machine.
pub const MAX_SAMPLE_ROWS: u32 = 20;

/// The most columns a request can name. Beyond that, the agent requests the
/// whole relation by omitting `columns`, and the user checks.
pub const MAX_SAMPLE_COLUMNS: usize = 64;

/// The longest designation accepted — a relation, namespace or column name —,
/// in bytes. Names, never query text.
pub const MAX_SAMPLE_NAME_BYTES: usize = 256;

/// What the caller imposes, and the model does not choose.
///
/// Built by the caller when a conversation opens, from the connection and the
/// session the user opened themselves. It is the **principle of least
/// authority** applied to the letter: an agent only reaches what this type
/// names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolScope {
    /// The connection the agent works on.
    pub connection: ConnectionId,
    /// The session open on that connection.
    pub session: SessionId,
    /// This session's query language. Also determines which analyzer
    /// classifies the text: a non-SQL language is not analyzed, hence
    /// `Unknown`, hence mutating.
    pub language: QueryLanguage,
}

impl ToolScope {
    /// Builds a tool scope.
    #[must_use]
    pub const fn new(
        connection: ConnectionId,
        session: SessionId,
        language: QueryLanguage,
    ) -> Self {
        Self {
            connection,
            session,
            language,
        }
    }
}

/// Arguments of [`EXECUTE_QUERY`].
///
/// A single field, and that is the point: everything that could weaken the
/// policy — connection, read-only, declared intent — is absent from the
/// schema, hence out of the model's reach.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecuteQueryArgs {
    /// The description goes to the model: it is in English, like all code text
    /// (CLAUDE.md). The attribute takes precedence over this comment, which is
    /// meant for reviewers.
    #[schemars(
        description = "The statement to run, exactly as it should reach the database. \
                       Write one statement. Reads run immediately; writes and DDL are \
                       held for the user's approval before anything happens."
    )]
    pub statement: String,
}

/// Arguments of [`DESCRIBE_SCHEMA`].
///
/// A single, optional field: search words. The connection comes from the
/// [`ToolScope`], and nothing here composes a query — the words serve to rank
/// names of the local catalog.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DescribeSchemaArgs {
    /// The description goes to the model: it is in English. It writes the bound
    /// in bytes, because `maxLength` counts characters and would not say the
    /// same thing as the refusal of the translation and of the executor
    /// ([`MAX_CATALOG_FOCUS_BYTES`]); a test keeps the figure aligned.
    #[schemars(
        description = "Optional search words — a table, collection or column name, or a \
                       topic — to describe the most relevant objects first. Omit it to \
                       describe the database from the start. At most 256 bytes of UTF-8."
    )]
    #[serde(default)]
    pub search: Option<String>,
}

/// Arguments of [`REQUEST_SAMPLE`].
///
/// **Names**, never query text: the relation and the columns are looked up in
/// the local catalog, and the read is composed by the driver, which quotes
/// every identifier ([I-10](../../../CLAUDE.md#i-10)). The connection comes
/// from the [`ToolScope`].
///
/// The schema announces the bounds the translation applies: an agent that does
/// not see them exceeds them, gets refused, and does not know by how much to
/// correct. The attributes' figures are literals — `schemars` does not accept a
/// constant in a description —, and a test keeps them aligned with
/// [`MAX_SAMPLE_ROWS`], [`MAX_SAMPLE_COLUMNS`] and [`MAX_SAMPLE_NAME_BYTES`].
/// The names' bound is written in bytes in the description: `maxLength` counts
/// characters, and would not say the same thing as the refusal.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestSampleArgs {
    #[schemars(
        description = "The table, view or collection to sample, exactly as describe_schema \
                       names it, without quotes. At most 256 bytes of UTF-8."
    )]
    pub relation: String,
    #[schemars(
        description = "Its schema or namespace, without quotes, when several objects share \
                       the name. Omit it otherwise. At most 256 bytes of UTF-8."
    )]
    #[serde(default)]
    pub namespace: Option<String>,
    #[schemars(
        description = "The columns you need, without quotes. Omit it to let the user choose \
                       among all of them. At most 64 columns, each name at most 256 bytes \
                       of UTF-8.",
        length(max = 64)
    )]
    #[serde(default)]
    pub columns: Option<Vec<String>>,
    #[schemars(
        description = "How many rows: 5 when omitted, from 1 to 20.",
        range(min = 1, max = 20)
    )]
    #[serde(default)]
    pub rows: Option<u32>,
}

/// Arguments of [`REFRESH_CATALOG`]: none.
///
/// The connection comes from the [`ToolScope`]. An empty object rather than no
/// schema: several providers refuse a tool without a `parameters` object.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshCatalogArgs {}

/// Translates validated arguments into a request.
type Translate = fn(&serde_json::Value, &ToolScope) -> Result<ToolRequest, AiError>;

/// What a tool call asks of the sink, once translated.
///
/// Two forms, and **a single command** at the end of each: there is no request
/// that does not carry one ([I-01](../../../CLAUDE.md#i-01)).
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum ToolRequest {
    /// A command to submit as is.
    Dispatch(Command),
    /// A sample to have approved, then read.
    Sample(SampleAsk),
}

impl ToolRequest {
    /// The command the request carries — for a sample, the read that will only
    /// leave after the approval.
    #[must_use]
    pub const fn command(&self) -> &Command {
        match self {
            Self::Dispatch(command) => command,
            Self::Sample(ask) => &ask.command,
        }
    }
}

/// A sample requested by an agent: the read, and the wanted columns.
///
/// The read is a bounded [`Command::PreviewRelation`] — the very command the
/// sample pinned by the user takes. It only runs **after** the approval, which
/// the sink obtains from the user; the columns named here are only a request,
/// which the approval restricts.
///
/// The names are written by the model: they have not been checked against the
/// catalog yet. It is the sink that does it, before showing anything
/// ([ADR-0034](../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
#[derive(Clone, PartialEq)]
pub struct SampleAsk {
    /// The read: `PreviewRelation`, bounded to [`MAX_SAMPLE_ROWS`].
    pub command: Command,
    /// The wanted columns, in the agent's order; empty for "all, at the user's
    /// choice".
    pub columns: Vec<String>,
}

// Column names: metadata, but kept out of the logs all the same — a column
// name can be the data (`hiv_status`).
impl fmt::Debug for SampleAsk {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SampleAsk")
            .field("command", &self.command.name())
            .field("columns", &self.columns.len())
            .finish()
    }
}

impl SampleAsk {
    /// The number of rows requested, already bounded by the translation.
    #[must_use]
    pub const fn rows(&self) -> u32 {
        match &self.command {
            Command::PreviewRelation { limit, .. } => *limit,
            _ => 0,
        }
    }
}

/// Produces the JSON schema of a tool's arguments.
type Schema = fn() -> serde_json::Value;

/// A tool, that is, a `Command` made callable by a model.
#[derive(Clone)]
pub struct ToolDefinition {
    name: &'static str,
    description: &'static str,
    /// Name of the [`Command`] variant produced. Serves the audit log and
    /// review: the tool → command mapping must be readable without unrolling
    /// the translation function.
    command: &'static str,
    schema: Schema,
    translate: Translate,
}

impl ToolDefinition {
    /// Name under which the model calls this tool.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// What the tool does, in one sentence meant for the model.
    #[must_use]
    pub const fn description(&self) -> &'static str {
        self.description
    }

    /// The [`Command`] variant this tool produces.
    #[must_use]
    pub const fn command(&self) -> &'static str {
        self.command
    }

    /// The declaration to pass to the provider.
    #[must_use]
    pub fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.name, self.description, (self.schema)())
    }
}

impl fmt::Debug for ToolDefinition {
    /// Written by hand: a function pointer in a derived `Debug` is an address,
    /// which teaches nothing. The name of the produced command does.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ToolDefinition")
            .field("name", &self.name)
            .field("command", &self.command)
            .finish_non_exhaustive()
    }
}

/// The tools Oxyn can translate.
///
/// The registry is **closed to content, open to declaration**: the
/// translations are Rust code — they build `Command`s, so they cannot come
/// from a plugin —, while the choice of the tools granted to an agent is
/// declarative ([`AgentSpec::allowed_tools`](crate::spec::AgentSpec)). That is
/// what lets an agent come from a plugin without opening a second path to the
/// drivers.
#[derive(Debug, Clone)]
pub struct ToolRegistry {
    tools: Vec<ToolDefinition>,
}

impl ToolRegistry {
    /// The registry shipped with Oxyn.
    #[must_use]
    pub fn builtin() -> Self {
        Self {
            tools: vec![
                ToolDefinition {
                    name: EXECUTE_QUERY,
                    // "Reads return rows" said the opposite of what the tool
                    // returns: an agent that expected rows from `sqlite_master`
                    // concluded it could not read the database. The schema is
                    // in the context, not here.
                    description: "Run one statement against the database the user opened. \
                                  Write it against the structure described to you. \
                                  Reads run immediately: the rows go to the user's result \
                                  grid, and you receive only the shape of the result (row \
                                  and batch counts), never the values — so do not query \
                                  system tables to learn the schema: call describe_schema. \
                                  Writes, DDL and anything the analyzer \
                                  cannot classify are held for the user's explicit approval, \
                                  so never assume a statement ran until the tool result says \
                                  so. You cannot choose the connection.",
                    command: "Execute",
                    schema: schema_of::<ExecuteQueryArgs>,
                    translate: translate_execute_query,
                },
                ToolDefinition {
                    name: DESCRIBE_SCHEMA,
                    description: concat!(
                        "Describe the structure of the database the user opened: \
                         its objects (tables, views, collections, indexes, key \
                         patterns, labels…), their fields and types as the server \
                         names them, keys and indexes, and the query language to \
                         write in. Read from Oxyn's catalog of this connection; \
                         what it has not loaded yet — the objects of a schema, the \
                         fields of the objects that match — Oxyn first reads from \
                         the server, as metadata only and within a few seconds. It \
                         never returns row values. The answer is bounded and says \
                         what is not loaded yet; when it says objects were left \
                         out or not loaded, call it again with search words. ",
                        erd_hint!()
                    ),
                    command: "DescribeCatalog",
                    schema: schema_of::<DescribeSchemaArgs>,
                    translate: translate_describe_schema,
                },
                ToolDefinition {
                    name: REQUEST_SAMPLE,
                    description: "Ask the user to share real rows of one table, view or \
                                  collection with you — when the structure is not enough, \
                                  for instance to see how values are written. Nothing is read \
                                  until the user approves, column by column, in Oxyn; you \
                                  receive only the columns they approve, or `the user \
                                  declined`. Allowed only when the connection's privacy tier \
                                  is `sampled`: under any other tier it is refused, and asking \
                                  again changes nothing. Ask once per answer, for the columns \
                                  you need.",
                    command: "PreviewRelation",
                    schema: schema_of::<RequestSampleArgs>,
                    translate: translate_request_sample,
                },
                ToolDefinition {
                    name: REFRESH_CATALOG,
                    description: "Re-read the structure of the database from the server: \
                                  its identity, then the objects of each schema, within a \
                                  bound of schemas and seconds. Use it after a schema change, \
                                  or when the schema shown to you looks out of date. It is \
                                  slow on large schemas; describe_schema already reads what \
                                  was never loaded.",
                    command: "RefreshCatalog",
                    schema: schema_of::<RefreshCatalogArgs>,
                    translate: translate_refresh_catalog,
                },
            ],
        }
    }

    /// The known names, in declaration order.
    #[must_use]
    pub fn names(&self) -> Vec<&'static str> {
        self.tools.iter().map(ToolDefinition::name).collect()
    }

    /// The definition carrying this name.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&ToolDefinition> {
        self.tools.iter().find(|tool| tool.name == name)
    }

    /// Does this tool exist?
    #[must_use]
    pub fn contains(&self, name: &str) -> bool {
        self.get(name).is_some()
    }

    /// The declarations to pass to the provider for this agent.
    ///
    /// The order follows the agent's allowlist: what it declares first is
    /// presented first.
    ///
    /// # Errors
    /// [`AiError::UnknownTool`] if the allowlist names a tool the registry does
    /// not know. Failing here rather than ignoring the entry is deliberate: an
    /// agent that believes it has a missing tool produces lost conversation
    /// turns, with nothing to signal it.
    pub fn specs_for(&self, allowed: &[String]) -> Result<Vec<ToolSpec>, AiError> {
        allowed
            .iter()
            .map(|name| {
                self.get(name)
                    .map(ToolDefinition::spec)
                    .ok_or_else(|| AiError::UnknownTool { name: name.clone() })
            })
            .collect()
    }

    /// Translates a model call into a core command.
    ///
    /// Three possible refusals, in this order: the tool does not exist, it is
    /// not granted to this agent, its arguments do not match the schema. The
    /// order matters for the message returned to the model — "unknown" and
    /// "not granted" do not call for the same correction.
    ///
    /// The returned command **has executed nothing**: it must still go through
    /// the `PolicyGate` carrying `Actor::Agent` (I-07).
    ///
    /// **[`REQUEST_SAMPLE`] is refused here.** Its read only exists after the
    /// user's approval, which only
    /// [`CommandSink::request_sample`](crate::runtime::CommandSink::request_sample)
    /// obtains: returning it bare would make this public function a second path
    /// to a read of values without consent. A caller that must serve this tool
    /// goes through [`ToolRegistry::request`].
    ///
    /// # Errors
    /// [`AiError::UnknownTool`], [`AiError::ToolNotAllowed`] — including for
    /// [`REQUEST_SAMPLE`] — or [`AiError::InvalidArguments`].
    pub fn translate(
        &self,
        call: &ToolCall,
        allowed: &[String],
        scope: &ToolScope,
    ) -> Result<Command, AiError> {
        match self.request(call, allowed, scope)? {
            ToolRequest::Dispatch(command) => Ok(command),
            ToolRequest::Sample(_) => Err(AiError::ToolNotAllowed {
                name: call.name.clone(),
            }),
        }
    }

    /// Translates a model call into a request to the sink: a command, or a
    /// sample to have approved.
    ///
    /// The same refusals, in the same order, as [`ToolRegistry::translate`].
    ///
    /// # Errors
    /// [`AiError::UnknownTool`], [`AiError::ToolNotAllowed`] or
    /// [`AiError::InvalidArguments`].
    pub fn request(
        &self,
        call: &ToolCall,
        allowed: &[String],
        scope: &ToolScope,
    ) -> Result<ToolRequest, AiError> {
        let Some(tool) = self.get(&call.name) else {
            return Err(AiError::UnknownTool {
                name: call.name.clone(),
            });
        };
        if !allowed.iter().any(|name| name == tool.name) {
            return Err(AiError::ToolNotAllowed {
                name: call.name.clone(),
            });
        }
        (tool.translate)(&call.arguments, scope)
    }
}

impl Default for ToolRegistry {
    /// The registry shipped with Oxyn. An empty registry would have no use.
    fn default() -> Self {
        Self::builtin()
    }
}

/// Produces the JSON schema of a type's arguments.
///
/// Four settings, each for a reason:
///
/// * `meta_schema: None` — the `$schema` field makes the strict validation of
///   several providers fail;
/// * `inline_subschemas: true` — no `$ref` or `$defs`, which not all providers
///   can follow;
/// * `title` removed — it is the Rust type's name, which teaches the model
///   nothing and leaks an implementation detail into the prompt;
/// * root `description` removed — `schemars` takes it from the type's `///`,
///   which is meant for reviewers. What the model must read is in
///   [`ToolDefinition::description`] and in the fields' `schemars` attributes.
fn schema_of<T: JsonSchema>() -> serde_json::Value {
    let mut settings = SchemaSettings::draft2020_12();
    settings.meta_schema = None;
    settings.inline_subschemas = true;
    let mut schema = SchemaGenerator::new(settings).into_root_schema_for::<T>();
    let _ = schema.remove("title");
    let _ = schema.remove("description");
    schema.to_value()
}

/// Decodes arguments, tolerating the absence of an object.
///
/// Several providers pass `null` rather than an empty object for a tool with
/// no argument. Refusing that would make a correct call fail.
fn parse_args<T: DeserializeOwned>(
    name: &'static str,
    raw: &serde_json::Value,
) -> Result<T, AiError> {
    let value = if raw.is_null() {
        serde_json::Value::Object(serde_json::Map::new())
    } else {
        raw.clone()
    };
    serde_json::from_value(value).map_err(|err| AiError::InvalidArguments {
        name: name.to_owned(),
        detail: err.to_string(),
    })
}

/// [`EXECUTE_QUERY`] → [`Command::Execute`].
fn translate_execute_query(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: ExecuteQueryArgs = parse_args(EXECUTE_QUERY, raw)?;
    let statement = args.statement.trim();
    if statement.is_empty() {
        return Err(AiError::InvalidArguments {
            name: EXECUTE_QUERY.to_owned(),
            detail: "`statement` is empty".to_owned(),
        });
    }

    // The intent and the risk come from analyzing the text, never from a field
    // the model would have filled. `oxyn-exec` will redo this work before the
    // `PolicyGate`: here, it serves not to lie in the preview and to set
    // consistent limits.
    let analysis = oxyn_query::classify_language(scope.language, statement);

    // The default limits forbid writing. They are only loosened when the
    // analysis says the text writes — and an agent's write is submitted to
    // approval anyway.
    let limits = if analysis.is_mutating() {
        ExecLimits::default().writable()
    } else {
        ExecLimits::default()
    };

    let request = ExecRequest::new(scope.language, statement)
        .with_intent(analysis.intent)
        .with_risk(analysis.risk)
        .with_limits(limits);

    Ok(ToolRequest::Dispatch(Command::Execute {
        connection: scope.connection,
        session: scope.session,
        request: Box::new(request),
    }))
}

/// [`DESCRIBE_SCHEMA`] → [`Command::DescribeCatalog`].
///
/// The search words are bounded here, before the command: an agent that wrote
/// a megabyte of them is told to shorten, and the executor redoes the check.
fn translate_describe_schema(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: DescribeSchemaArgs = parse_args(DESCRIBE_SCHEMA, raw)?;
    let focus = args
        .search
        .as_deref()
        .map(str::trim)
        .filter(|search| !search.is_empty())
        .map(str::to_owned);
    if focus
        .as_deref()
        .is_some_and(|focus| focus.len() > MAX_CATALOG_FOCUS_BYTES)
    {
        return Err(AiError::InvalidArguments {
            name: DESCRIBE_SCHEMA.to_owned(),
            detail: format!("`search` is limited to {MAX_CATALOG_FOCUS_BYTES} bytes"),
        });
    }
    Ok(ToolRequest::Dispatch(Command::DescribeCatalog {
        connection: scope.connection,
        focus,
    }))
}

/// [`REFRESH_CATALOG`] → [`Command::RefreshCatalog`].
fn translate_refresh_catalog(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let _args: RefreshCatalogArgs = parse_args(REFRESH_CATALOG, raw)?;
    Ok(ToolRequest::Dispatch(Command::RefreshCatalog {
        connection: scope.connection,
    }))
}

/// A name written by the model, bounded and not empty. No character is
/// refused: a name legal for the server must remain requestable, and it is the
/// driver's quoting that makes it harmless, not a filter here.
fn sample_name(field: &str, raw: &str) -> Result<String, AiError> {
    let invalid = |detail: String| AiError::InvalidArguments {
        name: REQUEST_SAMPLE.to_owned(),
        detail,
    };
    if raw.trim().is_empty() {
        return Err(invalid(format!("`{field}` is empty")));
    }
    if raw.len() > MAX_SAMPLE_NAME_BYTES {
        return Err(invalid(format!(
            "`{field}` is limited to {MAX_SAMPLE_NAME_BYTES} bytes"
        )));
    }
    Ok(raw.to_owned())
}

/// [`REQUEST_SAMPLE`] → a [`SampleAsk`] carrying [`Command::PreviewRelation`].
///
/// The names remain **data**: the relation travels as a field, never in a
/// statement text, and the driver quotes it when it composes the read
/// ([I-10](../../../CLAUDE.md#i-10)). The columns join no statement: they
/// designate what is copied from the result.
fn translate_request_sample(
    raw: &serde_json::Value,
    scope: &ToolScope,
) -> Result<ToolRequest, AiError> {
    let args: RequestSampleArgs = parse_args(REQUEST_SAMPLE, raw)?;
    let invalid = |detail: String| AiError::InvalidArguments {
        name: REQUEST_SAMPLE.to_owned(),
        detail,
    };
    let relation = sample_name("relation", &args.relation)?;
    let namespace = args
        .namespace
        .as_deref()
        .filter(|namespace| !namespace.trim().is_empty())
        .map(|namespace| sample_name("namespace", namespace))
        .transpose()?;
    let requested = args.columns.unwrap_or_default();
    if requested.len() > MAX_SAMPLE_COLUMNS {
        return Err(invalid(format!(
            "`columns` names at most {MAX_SAMPLE_COLUMNS} columns; omit it to offer all of them"
        )));
    }
    let mut columns: Vec<String> = Vec::with_capacity(requested.len());
    for column in &requested {
        let column = sample_name("columns", column)?;
        if !columns.contains(&column) {
            columns.push(column);
        }
    }
    let rows = match args.rows {
        None => DEFAULT_SAMPLE_ROWS,
        Some(rows) if (1..=MAX_SAMPLE_ROWS).contains(&rows) => rows,
        Some(_) => {
            return Err(invalid(format!(
                "`rows` must be between 1 and {MAX_SAMPLE_ROWS}"
            )));
        }
    };
    Ok(ToolRequest::Sample(SampleAsk {
        command: Command::PreviewRelation {
            connection: scope.connection,
            session: scope.session,
            catalog: None,
            namespace,
            relation,
            limit: rows,
            shape: PreviewShape::unordered(),
        },
        columns,
    }))
}

#[cfg(test)]
mod tests;
