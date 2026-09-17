//! Ce que la conversation dit, indépendamment de la façon dont on la dessine.
//!
//! Tout ce qui est ici est une fonction ou un type **libre** : aucun accès à une
//! `Entity`, aucune fenêtre, aucun thème. C'est ce qui rend vérifiables les
//! garanties d'UX-SPEC sur ce panneau — une commande montrée avant son résultat,
//! une réponse qui dit qu'elle a été coupée, un plafond de tours qui n'est ni un
//! succès ni une panne — sans avoir à ouvrir une fenêtre
//! (`.claude/rules/tests.md`, § Les tests d'interface).

use super::*;

/// How a conversation ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum Ending {
    /// The model answered.
    Answered {
        /// Turns consumed.
        turns: usize,
        /// The provider stopped it at its token ceiling.
        truncated: bool,
    },
    /// The user cancelled.
    Cancelled {
        /// Turns consumed before the cancellation took effect.
        turns: usize,
    },
    /// The turn ceiling was reached without a final answer.
    TurnLimit {
        /// Turns consumed, equal to the agent's ceiling.
        turns: usize,
    },
}

/// One line of the transcript.
///
/// The order of this vector **is** the order of the conversation, and that is
/// the whole guarantee: a [`Entry::Command`] always precedes its
/// [`Entry::Report`] because the events arrive in that order and nothing here
/// reorders them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::workspace) enum Entry {
    /// What the user asked.
    Question(String),
    /// A turn started.
    Turn {
        /// The turn that started, counting from 1.
        turn: usize,
        /// The agent's ceiling, so « 3 / 8 » needs no other source.
        max_turns: usize,
    },
    /// The answer, accumulated from the stream.
    Answer(String),
    /// A command left for the bus. Shown **before** its result.
    Command {
        /// The tool name, as the model asked for it.
        tool: String,
        /// The audit-log name of the command.
        command: &'static str,
        /// The connection, by name — never by identifier.
        target: String,
        /// Whether it may change data.
        mutating: bool,
    },
    /// The report came back.
    Report {
        /// The tool name.
        tool: String,
        /// What happened, whole: this is what the user reads.
        outcome: DispatchOutcome,
        /// Whether the model received less than the line above.
        withheld: bool,
    },
    /// A call refused at translation. Nothing was submitted.
    Rejected {
        /// The tool name, as the model asked for it.
        tool: String,
        /// The refusal.
        error: String,
    },
    /// Something the panel could not do, and why — the conversation continues.
    ///
    /// Distinct from [`Failed`](Self::Failed), which says the conversation broke
    /// off: here it is intact, and only one gesture was refused. Confondre les
    /// deux ferait croire à une panne là où il n'y a qu'un refus expliqué.
    Notice(String),
    /// The conversation ended normally.
    Ended(Ending),
    /// The conversation broke off.
    Failed(String),
}

/// What the panel names as the target of a command.
///
/// A [`ConnectionId`] never reaches the screen, not even here where the user
/// would arguably like to know ([I-03](../../../CLAUDE.md#i-03)). What answers
/// the question without leaking the identity is the connection's **name**, and
/// this workspace only knows its own: anything else is named as being elsewhere,
/// which is exactly the fact that matters.
pub(in crate::workspace) fn command_target(
    connection: Option<ConnectionId>,
    current: ConnectionId,
    name: &str,
) -> String {
    match connection {
        Some(target) if target == current => name.to_owned(),
        Some(_) => "a connection outside this workspace".to_owned(),
        None => "no connection".to_owned(),
    }
}

/// Folds one event into the transcript.
///
/// Text fragments accumulate into the trailing [`Entry::Answer`] and start a new
/// one when something else came in between: an answer split around a tool call
/// is two paragraphs in the conversation, not one paragraph with a hole.
pub(in crate::workspace) fn apply_event(
    entries: &mut Vec<Entry>,
    event: AiEvent,
    current: ConnectionId,
    name: &str,
) {
    match event {
        // Rien à montrer : la provenance signe ce que la conversation écrira,
        // elle n'est pas une étape de conversation. `Workspace::on_ai_event` la
        // retient dans `Assistant::provenance` **avant** d'appeler cette
        // fonction, et `open_proposal` la joint au texte qu'elle envoie.
        AiEvent::Started(_) => {}
        AiEvent::TurnStarted { turn, max_turns } => entries.push(Entry::Turn { turn, max_turns }),
        AiEvent::TextDelta(fragment) => match entries.last_mut() {
            Some(Entry::Answer(text)) => text.push_str(&fragment),
            _ => entries.push(Entry::Answer(fragment)),
        },
        AiEvent::CommandSubmitted {
            tool,
            command,
            connection,
            mutating,
        } => entries.push(Entry::Command {
            tool,
            command,
            target: command_target(connection, current, name),
            mutating,
        }),
        AiEvent::CommandReported {
            tool,
            outcome,
            withheld,
        } => entries.push(Entry::Report {
            tool,
            outcome,
            withheld,
        }),
        AiEvent::CallRejected { tool, error } => entries.push(Entry::Rejected { tool, error }),
        AiEvent::Finished(outcome) => entries.push(Entry::Ended(match outcome {
            AgentOutcome::Answered {
                turns, truncated, ..
            } => Ending::Answered { turns, truncated },
            AgentOutcome::Cancelled { turns } => Ending::Cancelled { turns },
            AgentOutcome::TurnLimit { turns } => Ending::TurnLimit { turns },
            // `AgentOutcome` est `#[non_exhaustive]`. Une fin inconnue se lit
            // comme une interruption, jamais comme une réponse : annoncer une
            // réponse que personne n'a lue est le seul rendu qui trompe.
            _ => {
                entries.push(Entry::Failed(
                    "The conversation ended in a way this Oxyn build does not know how \
                     to report. Nothing was hidden; nothing more is known."
                        .to_owned(),
                ));
                return;
            }
        })),
        AiEvent::Failed(message) => entries.push(Entry::Failed(message)),
    }
}

/// The SQL blocks an answer proposes, in the order they appear.
///
/// Fenced blocks only, and nothing is inferred from prose: a paragraph that
/// merely mentions `DELETE` is not a proposal, and offering to open it in a
/// console would put a statement in front of the user that the model never
/// meant to hand over.
///
/// A block is **text** and stays text. Opening it executes nothing
/// ([I-07](../../../CLAUDE.md#i-07)).
pub(in crate::workspace) fn sql_proposals(answer: &str) -> Vec<String> {
    let mut blocks = Vec::new();
    let mut current: Option<String> = None;
    for line in answer.lines() {
        let trimmed = line.trim_end();
        if let Some(rest) = trimmed.trim_start().strip_prefix("```") {
            match current.take() {
                // A closing fence: keep what it wrapped, if anything.
                Some(block) => {
                    let block = block.trim().to_owned();
                    if !block.is_empty() {
                        blocks.push(block);
                    }
                }
                // An opening fence. An unlabelled block is taken as SQL: this
                // agent answers with queries, and a model that omits the label
                // has not stopped proposing one.
                None => {
                    let label = rest.trim().to_ascii_lowercase();
                    if label.is_empty() || label == "sql" {
                        current = Some(String::new());
                    }
                }
            }
            continue;
        }
        if let Some(block) = current.as_mut() {
            block.push_str(line);
            block.push('\n');
        }
    }
    blocks
}

/// The five states of the transcript area
/// ([UX-SPEC](../../../../docs/UX-SPEC.md#états-dune-vue)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum PanelState {
    /// Nothing has been asked yet.
    Initial,
    /// A conversation is under way.
    Running,
    /// The conversation produced something to read.
    Answered,
    /// It ended, and the model said nothing at all.
    ///
    /// Distinct from an error on purpose: a provider that answers with an empty
    /// body has not failed, and telling the user it did would send them looking
    /// for a fault that is not there.
    Empty,
    /// It broke off.
    Failed,
}

/// Which state the panel is in, from the transcript and the token.
///
/// A conversation under way outranks everything: it is the most recent thing
/// the user did, and it is the state that must carry a way out.
pub(in crate::workspace) fn panel_state(entries: &[Entry], running: bool) -> PanelState {
    if running {
        return PanelState::Running;
    }
    if entries.is_empty() {
        return PanelState::Initial;
    }
    if entries
        .iter()
        .any(|entry| matches!(entry, Entry::Failed(_)))
    {
        return PanelState::Failed;
    }
    let said_something = entries.iter().any(|entry| {
        matches!(entry, Entry::Answer(text) if !text.trim().is_empty())
            || matches!(entry, Entry::Report { .. } | Entry::Rejected { .. })
    });
    if said_something {
        PanelState::Answered
    } else {
        PanelState::Empty
    }
}

/// The last answer of the transcript, which is where a proposal would be.
pub(super) fn last_answer(entries: &[Entry]) -> Option<&str> {
    entries.iter().rev().find_map(|entry| match entry {
        Entry::Answer(text) => Some(text.as_str()),
        _ => None,
    })
}

/// What a report says to the user, whole, and whether the model got less.
///
/// The server's message is shown as received, code included: paraphrasing it
/// would take away the only thing a professional can act on
/// (`.claude/checklists/revue-ui.md`).
pub(super) fn report_lines(outcome: &DispatchOutcome) -> (&'static str, String) {
    match outcome {
        DispatchOutcome::Completed { summary } => ("Completed", summary.clone()),
        DispatchOutcome::AwaitingApproval { reason } => {
            ("Waiting for your approval — nothing ran", reason.clone())
        }
        DispatchOutcome::Denied { reason } => ("Refused — nothing ran", reason.clone()),
        DispatchOutcome::Failed { class, message } => (class_label(*class), message.clone()),
        // `DispatchOutcome` est `#[non_exhaustive]` : une variante ajoutée dans
        // `oxyn-ai` s'affiche comme non interprétée plutôt que comme un succès.
        // Laisser croire qu'une commande a abouti est la seule lecture qui
        // pousserait l'utilisateur à agir sur un effet qui n'a pas eu lieu.
        _ => (
            "Reported — this Oxyn build cannot interpret the report",
            String::new(),
        ),
    }
}

/// The family of an error, as the driver classified it.
///
/// Carried as data, never re-derived from the message: an ambiguous timeout and
/// a permanent refusal look alike in prose and are not the same fact
/// ([I-13](../../../../CLAUDE.md#i-13)).
fn class_label(class: oxyn_core::ErrorClass) -> &'static str {
    match class {
        oxyn_core::ErrorClass::Transient => "Failed — transient",
        oxyn_core::ErrorClass::Permanent => "Failed — permanent",
        oxyn_core::ErrorClass::Ambiguous => "Failed — ambiguous, it may have applied",
        // `ErrorClass` is `#[non_exhaustive]`: an unknown family is not
        // announced as retryable, because inviting a retry on something we
        // cannot classify is how a duplicate row appears (I-13).
        _ => "Failed",
    }
}

/// What an ending says on its last line.
pub(super) fn ending_line(ending: Ending) -> String {
    match ending {
        Ending::Answered {
            turns,
            truncated: true,
        } => format!(
            "Answer cut short by the provider's token limit after {turns} turn(s). \
             It is not finished."
        ),
        Ending::Answered { turns, .. } => format!("Answered in {turns} turn(s)."),
        Ending::Cancelled { turns } => format!("Cancelled after {turns} turn(s)."),
        Ending::TurnLimit { turns } => format!(
            "Turn limit reached after {turns} turns, with no final answer. \
             This is neither a success nor a failure; what ran, ran."
        ),
    }
}
