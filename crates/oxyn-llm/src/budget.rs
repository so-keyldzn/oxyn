//! What a generation can accumulate, in total.
//!
//! The SSE decoder bounds the **current** frame; nothing bounded their sum. A
//! sequence of small frames, each valid and under the ceiling, made the text,
//! the arguments of a tool call or the number of open blocks grow without
//! end — until the system killed the process, without a trace.
//!
//! A [`GenerationBudget`] is kept for **one** generation. Every byte
//! accumulated or emitted is counted before it is, and every open block or
//! tool call too. The first overrun stops the generation: the decoder reports
//! it with an error that names the limit, throws away what was not closed — a
//! cut tool call is not a proposed action — and ends the stream with
//! [`StopReason::Interrupted`](crate::types::StopReason): the provider may
//! have continued, and billed, what we stopped reading (I-13).
//!
//! The limits are Oxyn's choices, not provider values: they bound memory,
//! they do not try to match the largest model of the moment. Each one is set
//! far above what an SQL assistant produces to be read.

use std::fmt;

/// Content bytes accumulated over a generation: text, refusal, reasoning,
/// tool names and arguments, identifiers.
///
/// Same order of magnitude as the bound of an SSE frame
/// (`sse::DEFAULT_BUFFER_LIMIT`): a response of eight mebibytes takes hours
/// to read, and beyond that the generation serves no one but can still
/// exhaust memory.
pub const MAX_GENERATION_BYTES: usize = 8 * 1024 * 1024;

/// Argument bytes of **one** tool call.
///
/// The arguments of an Oxyn tool are an SQL statement or a catalog address. A
/// mebibyte of SQL proposed by a model is no longer reviewable by the user who
/// must approve it; it is a stream going off the rails.
pub const MAX_TOOL_ARGUMENTS_BYTES: usize = 1024 * 1024;

/// Bytes of a tool name.
///
/// Valid names are those of Oxyn's registry, a few dozen bytes. A longer name
/// will never designate a known tool: letting it grow — some servers fragment
/// it — would have no use.
pub const MAX_TOOL_NAME_BYTES: usize = 256;

/// Tool calls open in a generation.
///
/// Each call becomes a `Command` that the `PolicyGate` judges, and that the
/// user may have to approve one by one. Beyond a few dozen per turn, it is no
/// longer a conversation but a burst.
pub const MAX_TOOL_CALLS: usize = 64;

/// Content blocks open in a generation — text, reasoning, tool, unknown types
/// included.
///
/// Each open block carries a state until it closes; a server that opens some
/// without ever closing them makes this state grow without emitting anything.
/// The ceiling is above [`MAX_TOOL_CALLS`], since a tool call is a block.
pub const MAX_CONTENT_BLOCKS: usize = 256;

/// A limit reached. The message names it and gives its value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum BudgetExceeded {
    /// [`MAX_GENERATION_BYTES`].
    Generation,
    /// [`MAX_TOOL_ARGUMENTS_BYTES`], for the call of the given index.
    ToolArguments {
        /// The index of the call in the generation.
        index: u32,
    },
    /// [`MAX_TOOL_NAME_BYTES`], for the call of the given index.
    ToolName {
        /// The index of the call in the generation.
        index: u32,
    },
    /// [`MAX_TOOL_CALLS`].
    ToolCalls,
    /// [`MAX_CONTENT_BLOCKS`].
    ContentBlocks,
}

impl fmt::Display for BudgetExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let arret = "Oxyn stopped reading the answer";
        match self {
            Self::Generation => write!(
                f,
                "the answer exceeded {MAX_GENERATION_BYTES} bytes; {arret}"
            ),
            Self::ToolArguments { index } => write!(
                f,
                "the arguments of tool call #{index} exceeded {MAX_TOOL_ARGUMENTS_BYTES} bytes; {arret}"
            ),
            Self::ToolName { index } => write!(
                f,
                "the name of tool call #{index} exceeded {MAX_TOOL_NAME_BYTES} bytes; {arret}"
            ),
            Self::ToolCalls => write!(
                f,
                "the answer opened more than {MAX_TOOL_CALLS} tool calls; {arret}"
            ),
            Self::ContentBlocks => write!(
                f,
                "the answer opened more than {MAX_CONTENT_BLOCKS} content blocks; {arret}"
            ),
        }
    }
}

impl std::error::Error for BudgetExceeded {}

/// The account of a generation.
///
/// Nothing is ever "given back" to the budget: a closed block has already
/// cost its memory at some point, and the emitted text is accumulated further
/// on by the caller.
#[derive(Debug, Default, Clone)]
pub struct GenerationBudget {
    bytes: usize,
    tool_calls: usize,
    blocks: usize,
}

impl GenerationBudget {
    /// A fresh budget, for one generation.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Counts `len` content bytes.
    ///
    /// # Errors
    /// [`BudgetExceeded::Generation`] if the total would exceed
    /// [`MAX_GENERATION_BYTES`]; nothing is counted then.
    pub fn charge(&mut self, len: usize) -> Result<(), BudgetExceeded> {
        let total = self.bytes.saturating_add(len);
        if total > MAX_GENERATION_BYTES {
            return Err(BudgetExceeded::Generation);
        }
        self.bytes = total;
        Ok(())
    }

    /// Tool calls already open: the index of the next one.
    #[must_use]
    pub const fn tool_calls(&self) -> usize {
        self.tool_calls
    }

    /// Counts an open content block.
    ///
    /// # Errors
    /// [`BudgetExceeded::ContentBlocks`] beyond [`MAX_CONTENT_BLOCKS`].
    pub fn open_block(&mut self) -> Result<(), BudgetExceeded> {
        if self.blocks >= MAX_CONTENT_BLOCKS {
            return Err(BudgetExceeded::ContentBlocks);
        }
        self.blocks += 1;
        Ok(())
    }

    /// Counts an open tool call, which is also a block.
    ///
    /// # Errors
    /// [`BudgetExceeded::ToolCalls`] beyond [`MAX_TOOL_CALLS`], or the refusal
    /// of [`open_block`](Self::open_block).
    pub fn open_tool_call(&mut self) -> Result<(), BudgetExceeded> {
        if self.tool_calls >= MAX_TOOL_CALLS {
            return Err(BudgetExceeded::ToolCalls);
        }
        self.open_block()?;
        self.tool_calls += 1;
        Ok(())
    }

    /// Counts a fragment of arguments of call `index`, whose arguments are
    /// already `current` bytes.
    ///
    /// # Errors
    /// [`BudgetExceeded::ToolArguments`] if the call would exceed
    /// [`MAX_TOOL_ARGUMENTS_BYTES`], or the refusal of [`charge`](Self::charge).
    pub fn charge_tool_arguments(
        &mut self,
        index: u32,
        current: usize,
        fragment: usize,
    ) -> Result<(), BudgetExceeded> {
        if current.saturating_add(fragment) > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(BudgetExceeded::ToolArguments { index });
        }
        self.charge(fragment)
    }

    /// Checks that a **complete** call fits in [`MAX_TOOL_ARGUMENTS_BYTES`],
    /// without counting it in the total.
    ///
    /// For the caller that receives the whole call after its fragments: those
    /// are already counted, counting them again would halve the budget. For a
    /// provider that delivers the call in one block, it is the only check.
    ///
    /// # Errors
    /// [`BudgetExceeded::ToolArguments`] beyond the limit.
    pub fn check_tool_arguments(&self, index: u32, len: usize) -> Result<(), BudgetExceeded> {
        if len > MAX_TOOL_ARGUMENTS_BYTES {
            return Err(BudgetExceeded::ToolArguments { index });
        }
        Ok(())
    }

    /// Counts a fragment of the name of call `index`, whose name is already
    /// `current` bytes.
    ///
    /// # Errors
    /// [`BudgetExceeded::ToolName`] if the name would exceed
    /// [`MAX_TOOL_NAME_BYTES`], or the refusal of [`charge`](Self::charge).
    pub fn charge_tool_name(
        &mut self,
        index: u32,
        current: usize,
        fragment: usize,
    ) -> Result<(), BudgetExceeded> {
        if current.saturating_add(fragment) > MAX_TOOL_NAME_BYTES {
            return Err(BudgetExceeded::ToolName { index });
        }
        self.charge(fragment)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_generation_budget_refuses_the_byte_past_its_limit_and_counts_nothing() {
        let mut budget = GenerationBudget::new();
        budget
            .charge(MAX_GENERATION_BYTES - 1)
            .expect("under the limit");
        budget.charge(1).expect("exactly at the limit");
        assert_eq!(budget.charge(1), Err(BudgetExceeded::Generation));
        assert_eq!(budget.charge(usize::MAX), Err(BudgetExceeded::Generation));
    }

    #[test]
    fn a_tool_call_is_also_a_block() {
        let mut budget = GenerationBudget::new();
        for _ in 0..MAX_TOOL_CALLS {
            budget.open_tool_call().expect("under the call limit");
        }
        assert_eq!(budget.open_tool_call(), Err(BudgetExceeded::ToolCalls));
        for _ in MAX_TOOL_CALLS..MAX_CONTENT_BLOCKS {
            budget.open_block().expect("under the block limit");
        }
        assert_eq!(budget.open_block(), Err(BudgetExceeded::ContentBlocks));
    }

    #[test]
    fn each_message_names_its_limit() {
        for (limite, valeur) in [
            (BudgetExceeded::Generation, MAX_GENERATION_BYTES),
            (
                BudgetExceeded::ToolArguments { index: 3 },
                MAX_TOOL_ARGUMENTS_BYTES,
            ),
            (BudgetExceeded::ToolName { index: 3 }, MAX_TOOL_NAME_BYTES),
            (BudgetExceeded::ToolCalls, MAX_TOOL_CALLS),
            (BudgetExceeded::ContentBlocks, MAX_CONTENT_BLOCKS),
        ] {
            let rendu = limite.to_string();
            assert!(rendu.contains(&valeur.to_string()), "{rendu}");
            assert!(rendu.contains("stopped"), "{rendu}");
        }
    }
}
