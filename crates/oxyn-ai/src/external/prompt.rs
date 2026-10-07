//! What an external agent receives, and the only gate that can make it.
//!
//! Authority: [ADR-0027](../../../../docs/adr/0027-porte-unique-pour-les-deux-destinations.md).
//!
//! # The defect this module closes
//!
//! [`run_turn`](super::turn::run_turn) took its prompt as a `&str`. The
//! connection's tier governed the agent's **launch** there — refusal before
//! the process starts — but not **the assembly of what is sent to it**, since
//! there was nothing to assemble.
//!
//! Nothing leaked: the only caller only passed the question typed by the user.
//! What was missing was not a protection, it was the **guarantee** — for a
//! provider, "what left?" is answered by rereading one function; for an agent,
//! one would have had to reread every caller, present and future. It is exactly
//! the property [I-04](../../../../CLAUDE.md#i-04) exists to remove.
//!
//! `.claude/rules/ia.md` names the shortcut that destroys it: "just for the
//! schema, it's `Metadata` anyway". It would have been written here as a
//! `format!`, without any type or test turning red.
//!
//! # Why a thin type, and not the provider path's `AgentContext`
//!
//! An external agent receives a text, and that is all: it has no use for a
//! system message, nor for a tool conversation session. Imposing on it the
//! provider path's `AgentSession` would be the abstraction for a single caller
//! that [CLAUDE.md](../../../../CLAUDE.md#code-organization) advises against.
//! This type therefore carries the text — and, when a schema comes with it,
//! the [`AgentContext`] that rendered it, so that the caller can say what left.
//!
//! This module takes option **B** of ADR-0027: a type that only carries what
//! leaves, whose constructors require the connection's tier.
//!
//! # The schema, and through which gate it comes in
//!
//! An external agent that only receives the question does not know the
//! database: it runs `SELECT name FROM sqlite_master`, receives "11 rows" — the
//! tool returns the shape, never the values (ADR-0030 § 4) — and ends up
//! proposing `SELECT * FROM your_table`. That is what the user observed on
//! 2026-09-23.
//!
//! [`AgentPrompt::with_schema`] answers it **without writing a second gate**:
//! the schema is rendered by [`ContextBuilder::build`], the gateway of I-04, the
//! same code, under the same tier, with the same budget as for the internal
//! assistant. This module renders nothing itself; it places an already rendered
//! block.
//!
//! The selection is the internal assistant's too, semantic ranking included
//! ([ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)):
//! [`AgentPrompt::with_schema_ranked`] hands the host's per-relation scores to
//! [`ContextBuilder::with_semantic_scores`], and ranks nothing itself. Scores
//! can widen the selection up to the agent's `max_relations`, under the same
//! tier. [`AgentPrompt::following`] takes none: it only describes mentions,
//! which no score reorders.
//!
//! # The approved sample, and through the same gate
//!
//! A sample the user approved column by column comes into the prompt through
//! the **same** call: [`AgentPrompt::with_schema`] passes it to
//! [`ContextBuilder::with_samples`], which drops it under any tier other than
//! `Sampled`. This module renders no value itself
//! ([ADR-0034](../../../../docs/adr/0034-echantillon-pour-toute-destination.md)).
//!
//! The agent's memory is not this type's business, and that is why
//! [`AgentPrompt::from_user`] takes no sample: a prompt that **continues** a
//! session talks to a process that remembers, and a value shown there would
//! stay there. The caller that attaches a sample opens a new session —
//! `with_schema` is the opening constructor — and releases it after the
//! exchange, which leaves no memory.

use std::fmt;

use oxyn_catalog::CatalogCache;
use oxyn_core::{OxynError, QueryLanguage};

use crate::agent_file::{PromptTarget, render_system_prompt};
use crate::context::{
    AgentContext, ContextBuilder, Mention, QUESTION_HEADER, RowSample, SemanticScores,
};
use crate::privacy::PrivacyTier;
use crate::spec::AgentSpec;
use crate::untrusted;

/// What opens the conversation's agent block, so that the external agent reads
/// it as Oxyn's and not as the user's question.
///
/// In English, because it goes to a model. It does not claim more than it is:
/// ACP has no system message, and these instructions arrive below the external
/// agent's own (ADR-0049 § 7).
pub const AGENT_HEADER: &str =
    "Oxyn's instructions for this conversation, written by Oxyn and not by the user:";

/// The agent an opening prompt carries, and the target it is rendered for.
///
/// One argument rather than two: the pair is what
/// [`render_system_prompt`] reads, and nothing else of the conversation.
#[derive(Debug, Clone, Copy)]
pub struct AgentInstructions<'a> {
    /// The conversation's agent.
    pub spec: &'a AgentSpec,
    /// The connection's dialect, driver and marking, and this recipient.
    pub target: &'a PromptTarget,
}

/// What precedes the schema: where it comes from, and what can be expected of
/// it.
///
/// In English, because it goes to a model. It also says what the tool does
/// **not** return: an agent that expects rows from `execute_query` concludes it
/// cannot read the database, and that is the failure observed. Composed at the
/// call: the names of the tools and of the server come from their constants,
/// not from a copy.
fn schema_intro() -> String {
    format!(
        "You are working inside Oxyn, a database workspace. The structure of the database the \
         user has open is described below, as far as it fits. Write statements only against \
         the objects and fields it names; for anything it leaves out, call the \
         `{describe}` tool of the `{server}` MCP server with search words rather than guessing \
         a name. Run a statement with its `{execute}` tool: the rows appear in the user's \
         result grid, and the tool returns you the shape of the result — row and batch \
         counts — never the values. {erd}",
        describe = crate::tools::DESCRIBE_SCHEMA,
        erd = crate::tools::ERD_HINT,
        execute = crate::tools::EXECUTE_QUERY,
        server = super::mcp::SERVER_NAME,
    )
}

/// What leaves for an external agent, once the tier is applied.
///
/// **No naive public constructor.** The only ways to get one are
/// [`AgentPrompt::from_user`] and [`AgentPrompt::with_schema`], which require
/// the connection's tier. A `From<String>` or a `new(&str)` would reopen
/// exactly the hole this type closes — it is the discipline to maintain, and
/// the only one.
///
/// The `Debug` is written by hand: the text carries table and column names of
/// the user's database, which a `tracing::debug!("{prompt:?}")` would write to
/// a log.
#[derive(Clone, PartialEq)]
pub struct AgentPrompt {
    text: String,
    context: Option<AgentContext>,
}

impl fmt::Debug for AgentPrompt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentPrompt")
            .field(
                "text",
                &format_args!("<redacted, {} bytes>", self.text.len()),
            )
            .field("context", &self.context)
            .finish()
    }
}

impl AgentPrompt {
    /// Composes the prompt from what **the user typed**, and nothing else.
    ///
    /// # What this gate guarantees, and what it does not
    ///
    /// It guarantees that an external agent prompt can only be born from user
    /// input, under a tier that admits this destination. It does not claim to
    /// filter the content of the input: what the user writes belongs to them,
    /// and [ADR-0006](../../../../docs/adr/0006-ai-privacy-tiers.md) was never
    /// meant to censor their own question.
    ///
    /// Context only reaches this prompt through [`AgentPrompt::with_schema`],
    /// which has it rendered by [`ContextBuilder::build`].
    ///
    /// # Errors
    ///
    /// [`OxynError::Config`] if the tier closes external agents, or if the
    /// question is empty. The message never copies the input.
    pub fn from_user(tier: PrivacyTier, question: &str) -> Result<Self, OxynError> {
        // The same refusal as `run_turn`, but **before** a prompt exists: under
        // a tier that closes agents, there is nothing to compose.
        if !tier.allows_remote_provider() {
            return Err(OxynError::Config(
                "this connection is marked local-only, and Oxyn cannot see where an external \
                 agent sends its prompts; declare a local model provider instead"
                    .into(),
            ));
        }
        let question = question.trim();
        if question.is_empty() {
            return Err(OxynError::Config("an empty question is not sent".into()));
        }
        Ok(Self {
            text: question.to_owned(),
            context: None,
        })
    }

    /// Composes the prompt that opens an agent session: the conversation's
    /// agent, the structure of the database, then the question.
    ///
    /// The agent's block is [`render_system_prompt`]'s, rendered here after
    /// the tier check and placed **before** the schema, under
    /// [`AGENT_HEADER`]: Oxyn's text, never mixed with what the database
    /// wrote. Only the opening carries it — [`following`](Self::following)
    /// does not repeat it, the process remembers (ADR-0049 § 7).
    ///
    /// The schema is rendered by [`ContextBuilder::build`] under `tier` — the
    /// gateway of [I-04](../../../../CLAUDE.md#i-04), with its budget, its
    /// question-driven selection and its `untrusted` fence. The preamble that
    /// says what a fence is comes **before** the fence, as in the internal
    /// assistant's system message: a model that reads the instruction after the
    /// data has already read the data.
    ///
    /// `samples` are the samples the user approved for **this** question. They
    /// come in through [`ContextBuilder::with_samples`], and only leave under
    /// `Sampled`: under any other tier they are dropped, and
    /// [`AgentContext::dropped_samples`] says so. The caller that attaches some
    /// opens a new session and releases it after the exchange (see the module
    /// header).
    ///
    /// `mentions` are the objects the user named with an `@`: described first
    /// by [`ContextBuilder::with_mentions`], under the same budget.
    ///
    /// # Errors
    ///
    /// Those of [`AgentPrompt::from_user`], checked **before** anything is
    /// rendered; then [`OxynError::Config`] when the agent's prompt does not
    /// render — a variable outside the closed list, a recipient without a
    /// fragment.
    pub fn with_schema(
        tier: PrivacyTier,
        question: &str,
        cache: &CatalogCache,
        language: QueryLanguage,
        samples: Vec<RowSample>,
        mentions: Vec<Mention>,
        agent: AgentInstructions<'_>,
    ) -> Result<Self, OxynError> {
        Self::with_schema_ranked(
            tier,
            question,
            cache,
            language,
            samples,
            mentions,
            agent,
            SemanticScores::new(),
        )
    }

    /// [`AgentPrompt::with_schema`], the selection ordered by `semantic` as
    /// well ([ADR-0056](../../../../docs/adr/0056-local-cpu-embeddings-for-context-selection.md)).
    ///
    /// The scores go to [`ContextBuilder::with_semantic_scores`]: the same
    /// ranking, the same tier and the same budget as the internal assistant,
    /// and no text of their own. They can make more relations be described —
    /// up to the agent's `max_relations` — and so more schema leave, always
    /// under `tier`. Empty, the prompt is exactly `with_schema`'s.
    ///
    /// # Errors
    ///
    /// Those of [`AgentPrompt::with_schema`].
    #[expect(
        clippy::too_many_arguments,
        reason = "with_schema's parts, plus the scores its catalog fill ranked with"
    )]
    pub fn with_schema_ranked(
        tier: PrivacyTier,
        question: &str,
        cache: &CatalogCache,
        language: QueryLanguage,
        samples: Vec<RowSample>,
        mentions: Vec<Mention>,
        agent: AgentInstructions<'_>,
        semantic: SemanticScores,
    ) -> Result<Self, OxynError> {
        let asked = Self::from_user(tier, question)?;
        let instructions = render_system_prompt(agent.spec, agent.target)
            .map_err(|error| OxynError::Config(error.to_string()))?;
        // The agent's own budget for the opening schema (ADR-0049 § 7).
        let context = ContextBuilder::new(cache, tier)
            .with_policy(agent.spec.context.clone())
            .with_language(language)
            .focused_on(asked.text.clone())
            .with_mentions(mentions)
            .with_samples(samples)
            .with_semantic_scores(semantic)
            .build();
        let text = format!(
            "{AGENT_HEADER}\n\n{instructions}\n\n{}\n\n{}\n\n{}\n\n{QUESTION_HEADER}\n{}",
            schema_intro(),
            untrusted::PREAMBLE,
            context.prompt_block(),
            asked.text
        );
        Ok(Self {
            text,
            context: Some(context),
        })
    }

    /// Composes the prompt of a question that **follows** an already open
    /// session.
    ///
    /// Without mentions, it is [`AgentPrompt::from_user`]: the session knows
    /// the schema since its opening. With mentions, the named objects precede
    /// the question, rendered by [`ContextBuilder::build`] in
    /// [`ContextBuilder::mentioned_only`] mode — the same gate, the same tier,
    /// the same budget as at the opening; only the search adds nothing, since
    /// the rest has already been said. No sample: a prompt that continues talks
    /// to a process that remembers (see the module header).
    ///
    /// # Errors
    ///
    /// Those of [`AgentPrompt::from_user`], checked before any rendering.
    pub fn following(
        tier: PrivacyTier,
        question: &str,
        cache: &CatalogCache,
        language: QueryLanguage,
        mentions: Vec<Mention>,
    ) -> Result<Self, OxynError> {
        let asked = Self::from_user(tier, question)?;
        if mentions.is_empty() {
            return Ok(asked);
        }
        let context = ContextBuilder::new(cache, tier)
            .with_language(language)
            .with_mentions(mentions)
            .mentioned_only()
            .build();
        Ok(Self {
            text: context.follow_up(&asked.text),
            context: Some(context),
        })
    }

    /// The text that leaves on the protocol.
    ///
    /// Borrowed and not returned: this type only exists to be consumed by
    /// [`run_turn`](super::turn::run_turn).
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The attached context, if there is one — to tell the user what left: how
    /// many relations, how many dropped, what cost.
    #[must_use]
    pub const fn context(&self) -> Option<&AgentContext> {
        self.context.as_ref()
    }
}

/// The SQL agent rendered for Claude Code on a SQLite production connection:
/// the opening every test that is not about the agent block passes.
#[cfg(test)]
pub(crate) fn sql_instructions() -> AgentInstructions<'static> {
    use std::sync::LazyLock;

    use oxyn_core::{DriverId, Environment, SqlDialect};

    use crate::agent_file::{ExternalAgentKind, Recipient};

    static SPEC: LazyLock<AgentSpec> = LazyLock::new(crate::builtin::sql_agent);
    static TARGET: LazyLock<PromptTarget> = LazyLock::new(|| PromptTarget {
        dialect: SqlDialect::Sqlite,
        driver: DriverId::new(DriverId::SQLITE).expect("a valid driver name"),
        environment: Environment::Production,
        recipient: Recipient::External(ExternalAgentKind::ClaudeCode),
    });
    AgentInstructions {
        spec: &SPEC,
        target: &TARGET,
    }
}

#[cfg(test)]
mod tests;
