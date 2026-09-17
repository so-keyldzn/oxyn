//! The conversation panel: what an agent is doing, while it does it.
//!
//! # What this module owns, and what it deliberately does not
//!
//! It owns a transcript and one cancellation token. It owns **no** execution
//! path: every command an agent submits goes to the same scheduler as the
//! user's own, through [`Backend::start_conversation`], and this file never
//! reaches a driver ([I-01](../../../CLAUDE.md#i-01)).
//!
//! It does not own the privacy tier either. The tier belongs to the connection,
//! travels in the [`ConversationRequest`], and is re-checked by
//! `AgentRuntime::run` against a reach it measured itself. What this panel
//! decides — whether `Ask AI` is offered at all — is therefore an **affordance**
//! and never a gate: a stale classification here can cost a refusal, never a
//! send ([I-04](../../../CLAUDE.md#i-04)).
//!
//! # Why the transcript is built by a free function
//!
//! [`apply_event`] takes the events and the vector, and nothing else. Every
//! guarantee UX-SPEC states about this panel — a command shown *before* its
//! result, an answer that says it was cut, a turn limit that is neither a
//! success nor a failure — is then a fact about a `Vec<Entry>` that a test can
//! assert without a window. A transcript assembled inside `render` would be
//! testable at no level at all (`.claude/rules/ui-gpui.md`).
//!
//! # What is never written here
//!
//! A connection identifier. [`AiEvent::CommandSubmitted`] carries one, because
//! the runtime reads it off the command itself; what the panel shows is the
//! **name** of the connection when it is this workspace's, and a neutral phrase
//! otherwise ([I-03](../../../CLAUDE.md#i-03), [`transcript::command_target`]).

use super::*;
use crate::ai::AiEvent;
use crate::backend::{ClassifiedProvider, ConversationRequest};
use oxyn_ai::{AgentOutcome, DispatchOutcome};
use oxyn_core::{ConnectionId, PrivacyTier, Provenance};
use oxyn_ui::{FieldEvent, TextField};

#[cfg(test)]
mod tests;
mod transcript;
mod view;

pub(in crate::workspace) use transcript::{Entry, apply_event, sql_proposals};

/// What the `Ask AI` entry of the connection bar is right now.
///
/// Three states, and the first one is the point: with no provider declared
/// there is no entry, no badge and no mention. A greyed control inviting the
/// user to configure something would be an advertisement, not a feature
/// ([UX-SPEC](../../../docs/UX-SPEC.md#le-workspace-ia-nexiste-que-sil-a-été-configuré)).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::workspace) enum AskAi {
    /// Nothing is drawn at all.
    Absent,
    /// Drawn, inert, and carrying the reason it is inert.
    Disabled(&'static str),
    /// Drawn and usable.
    Enabled,
}

/// Declarations exist, but this connection's tier admits none of them.
///
/// The entry stays visible: making it disappear would read as a defect, and
/// ADR-0006 asks that an unavailable feature explain itself.
///
/// The wording names **both** kinds, because under `Local` both are closed and
/// for the same reason — Oxyn cannot see where the data would go. Naming only
/// providers would send the user to declare an external agent instead, which
/// this tier refuses just as firmly.
const NO_USABLE_DECLARATION: &str = "This connection's privacy tier only admits a provider that resolved to this machine, \
     and no declared provider is local. An external agent cannot serve it either: nothing \
     says where it sends data.";

/// A session whose driver has no SQL at all.
///
/// The built-in agent writes SQL and its only tool executes SQL, so on such a
/// session it has nothing to offer. Said rather than hidden
/// ([ADR-0003](../../../docs/adr/0003-driver-capabilities.md)).
const NO_SQL: &str = "This session does not support SQL, and the assistant only writes SQL.";

/// The declarations could not be read at all.
///
/// **Not the same thing as none being declared**, and the entry must not say so.
/// An empty list means « Oxyn works without AI » ; a failed read means Oxyn does
/// not know, and hiding the entry then sends the user to re-declare a provider
/// that is already there. The entry therefore stays visible and inert, like the
/// `Local` case, with a reason of its own.
const UNREADABLE: &str = "The declared providers could not be read from local state.";

/// Whether — and how — the connection bar draws its `Ask AI` entry.
///
/// `providers` is what the store answered, already classified. An empty slice
/// and « not read yet » are the same drawing on purpose: both mean there is
/// nothing to ask, and a bar that flickers an entry into existence for the
/// duration of one SQLite read is worse than one that waits.
pub(in crate::workspace) fn ask_ai_entry(
    providers: &[ClassifiedProvider],
    agents: &[oxyn_core::ExternalAgentConfig],
    unreadable: bool,
    tier: PrivacyTier,
    capabilities: Capabilities,
) -> AskAi {
    // Avant tout le reste : ne pas savoir n'est pas savoir qu'il n'y a rien.
    if unreadable {
        return AskAi::Disabled(UNREADABLE);
    }
    if providers.is_empty() && agents.is_empty() {
        return AskAi::Absent;
    }
    if !capabilities.contains(Capabilities::SQL) {
        return AskAi::Disabled(NO_SQL);
    }
    // `provider_for` plutôt qu'une condition équivalente réécrite ici : c'est
    // **lui** qui choisira ensuite, et deux formulations de la même règle
    // finissent par diverger. Elles avaient divergé — la garde admettait des
    // cas où `provider_for` rendait toujours `Some`, ce qui rendait le repli
    // vers un agent externe inatteignable, donc mort.
    if provider_for(providers, tier).is_none() && usable_agent(agents, tier).is_none() {
        return AskAi::Disabled(NO_USABLE_DECLARATION);
    }
    AskAi::Enabled
}

/// The first declared agent this tier admits, if any.
///
/// An external agent is always `Reach::Unresolved` — nothing in the protocol
/// says where its model runs — so `Local` closes every one of them, and no
/// amount of declaring changes that
/// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
pub(in crate::workspace) fn usable_agent(
    agents: &[oxyn_core::ExternalAgentConfig],
    tier: PrivacyTier,
) -> Option<&oxyn_core::ExternalAgentConfig> {
    agents
        .iter()
        .find(|agent| oxyn_ai::privacy::allows_external_agent(tier, agent))
}

/// The declaration a conversation on this tier may use.
///
/// Under `Local`, only a provider that resolved to this machine is eligible —
/// an unresolved endpoint counts as remote, and this function inherits that
/// from [`ClassifiedProvider::is_local`] rather than re-deciding it.
pub(in crate::workspace) fn provider_for(
    providers: &[ClassifiedProvider],
    tier: PrivacyTier,
) -> Option<&ClassifiedProvider> {
    providers
        .iter()
        .find(|provider| provider.is_local() || tier.allows_remote_provider())
}

/// The conversation of one connection, and the field that starts it.
///
/// Grouped rather than spread over the workspace: `active` and `cancelling`
/// only mean anything next to each other, and a caller updating one without the
/// other would leave a cancel button that no longer cancels.
pub(in crate::workspace) struct Assistant {
    /// The declared providers, classified. `None` until the store answered.
    pub(in crate::workspace) providers: Option<Vec<ClassifiedProvider>>,
    /// Les agents externes déclarés. `None` tant que rien n'a été lu —
    /// distinct d'une liste vide, qui est une installation sans agent.
    pub(in crate::workspace) agents: Option<Vec<oxyn_core::ExternalAgentConfig>>,
    /// Why the provider list could not be read, if it could not.
    pub(in crate::workspace) providers_error: Option<String>,
    /// The question field. Typing in it starts nothing.
    pub(in crate::workspace) question: Entity<TextField>,
    /// The transcript, in the order events arrived.
    pub(in crate::workspace) entries: Vec<Entry>,
    /// The token of the conversation under way, if any.
    pub(in crate::workspace) active: Option<CancelToken>,
    /// A cancellation the provider has not acted on yet.
    ///
    /// UX-SPEC is explicit that the request is taken immediately and its
    /// *effect* depends on the provider. The panel says so rather than
    /// pretending the conversation already stopped.
    pub(in crate::workspace) cancelling: bool,
    /// Where the transcript is scrolled.
    pub(in crate::workspace) scroll: gpui::ScrollHandle,
    /// Which read of the declarations is the one that counts.
    ///
    /// Two reads can be in flight — the one this workspace starts at
    /// construction, and the one the settings screen asks for after saving —
    /// and they answer on a Tokio runtime that promises nothing about order.
    /// Without this counter the older answer can land last and put the list
    /// back the way it was, which is a provider saved and an entry that never
    /// appears. Same shape as `console_attempt` and `preview_active`: the
    /// request that is still awaited names itself.
    pub(in crate::workspace) read: u64,
    /// Ce qui signera le texte que cette conversation propose.
    ///
    /// Posé par [`AiEvent::Started`], avant le premier tour
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    ///
    /// Ce commentaire affirmait qu'« une proposition sans provenance est
    /// impossible par construction ». C'était vrai quand un fournisseur était la
    /// seule destination. **Ce n'est plus vrai** : un tour d'agent externe
    /// produit des propositions sans jamais émettre `Started`, parce que
    /// [`Provenance`] exige une famille de fournisseur et un modèle, dont un
    /// agent n'a ni l'un ni l'autre
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    ///
    /// En attendant que ce type sache décrire un agent, `open_proposal` refuse
    /// d'écrire sans provenance : une absence visible vaut mieux qu'une marque
    /// fausse, et `None` dans un document veut dire « écrit par l'utilisateur ».
    pub(in crate::workspace) provenance: Option<Provenance>,
}

impl std::fmt::Debug for Assistant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Assistant")
            .field("entries", &self.entries.len())
            .field("running", &self.active.is_some())
            .finish_non_exhaustive()
    }
}

impl Assistant {
    /// Creates the question field and subscribes to what the user accepts.
    ///
    /// Called from the workspace constructor alone: the subscriptions belong to
    /// that entity, and a second field would leave two live ways to start a
    /// conversation.
    pub(in crate::workspace) fn new(cx: &mut Context<'_, Workspace>) -> Self {
        let question = cx.new(|cx| TextField::new(String::new(), false, cx));
        cx.subscribe(
            &question,
            |this: &mut Workspace, _, event: &FieldEvent, cx| match event {
                FieldEvent::Submit => this.ask_assistant(cx),
                // The field swallows Escape, so no workspace shortcut ever sees
                // it. Without this line the only way out of a running
                // conversation would vanish for whoever is standing in the
                // field — which is precisely where they are while it runs.
                FieldEvent::Escape => this.cancel_conversation(cx),
                _ => {}
            },
        )
        .detach();
        Self {
            providers: None,
            agents: None,
            providers_error: None,
            question,
            entries: Vec::new(),
            active: None,
            cancelling: false,
            scroll: gpui::ScrollHandle::new(),
            read: 0,
            provenance: None,
        }
    }

    /// The `Ask AI` entry this connection currently warrants.
    pub(in crate::workspace) fn entry(
        &self,
        tier: PrivacyTier,
        capabilities: Capabilities,
    ) -> AskAi {
        ask_ai_entry(
            self.providers.as_deref().unwrap_or(&[]),
            self.agents.as_deref().unwrap_or(&[]),
            self.providers_error.is_some(),
            tier,
            capabilities,
        )
    }
}

impl Workspace {
    /// Relit les agents externes déclarés et les montre.
    ///
    /// Pas de génération de lecture comme pour les fournisseurs : il n'y a ni
    /// résolution DNS ni classement à faire vieillir, donc pas de réponse
    /// tardive qui pourrait écraser une plus récente avec des données périmées.
    pub(crate) fn reload_external_agents(&mut self, cx: &mut Context<'_, Self>) {
        let listed = self.backend.external_agents();
        cx.spawn(async move |this, cx| {
            let answer = listed.await;
            let _ = this.update(cx, |this: &mut Self, cx| {
                // Une lecture qui échoue **n'est pas** « aucun agent » : la
                // liste reste ce qu'elle était plutôt que de disparaître, pour
                // la raison qui vaut déjà pour les fournisseurs — une panne
                // locale ne doit pas se lire comme une absence de déclaration.
                if let Ok(Ok(agents)) = answer {
                    this.assistant.agents = Some(agents);
                    this.refresh_agent_settings(cx);
                } else {
                    tracing::warn!("external agents could not be read; the list is kept as is");
                }
            });
        })
        .detach();
    }

    /// Reads the declared providers, off the UI thread.
    ///
    /// Called once at construction and again whenever the settings screen says
    /// the declarations changed — which is what makes the first provider appear
    /// without a restart, and the last one disappear
    /// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
    pub(crate) fn reload_ai_providers(&mut self, cx: &mut Context<'_, Self>) {
        let listed = self.backend.ai_providers_classified();
        self.assistant.read = self.assistant.read.saturating_add(1);
        let read = self.assistant.read;
        cx.spawn(async move |this, cx| {
            let answer = listed.await;
            let _ = this.update(cx, |this: &mut Self, cx| {
                this.receive_ai_providers(read, answer, cx);
            });
        })
        .detach();
    }

    /// Takes what the store answered, and closes the panel if nothing is left.
    ///
    /// A failure to read the declarations is **not** treated as « no provider ».
    /// The entry stays visible and inert, carrying the reason: hiding it would
    /// make a local SQLite error indistinguishable from « nothing is declared »,
    /// and send the user to re-declare a provider that is already stored.
    ///
    /// This used to hide the entry and claim the panel would explain — the panel
    /// that the missing entry made unreachable.
    pub(super) fn receive_ai_providers(
        &mut self,
        read: u64,
        answer: Result<Result<Vec<ClassifiedProvider>, OxynError>, oneshot::error::RecvError>,
        cx: &mut Context<'_, Self>,
    ) {
        // An answer to a read that has been superseded says nothing about what
        // is declared **now**, and applying it would undo the newer one.
        if read != self.assistant.read {
            return;
        }
        match answer {
            Ok(Ok(providers)) => {
                self.assistant.providers_error = None;
                self.assistant.providers = Some(providers);
            }
            Ok(Err(erreur)) => {
                self.assistant.providers_error = Some(erreur.to_string());
                self.assistant.providers = Some(Vec::new());
            }
            Err(_) => {
                self.assistant.providers_error =
                    Some("The provider listing worker stopped answering.".to_owned());
                self.assistant.providers = Some(Vec::new());
            }
        }
        // The last declaration removed closes the panel it opened: a
        // conversation on a provider that no longer exists would offer a field
        // that can no longer send anything (UX-SPEC).
        if self.panel == WorkspacePanel::Assistant
            && matches!(
                self.assistant
                    .entry(self.display.privacy_tier, self.capabilities),
                AskAi::Absent
            )
        {
            self.cancel_conversation(cx);
            self.panel = WorkspacePanel::Sql;
        }
        // La même lecture alimente l'entrée `Ask AI` et l'écran de
        // configuration. Une seconde lecture pour l'écran ferait une seconde
        // campagne de résolution DNS, et deux réponses qui peuvent diverger.
        self.refresh_provider_settings(cx);
        cx.notify();
    }

    /// Pose la question à un agent externe, faute de fournisseur utilisable.
    ///
    /// Ne fait rien si aucun agent déclaré n'est admis par le niveau de la
    /// connexion. Le choix passe par [`usable_agent`] plutôt que par le premier
    /// de la liste : prendre le premier puis se faire refuser par `run_turn`
    /// afficherait un échec là où une autre déclaration convenait.
    fn ask_external_agent(&mut self, question: String, cx: &mut Context<'_, Self>) {
        let Some(agent) = usable_agent(
            self.assistant.agents.as_deref().unwrap_or(&[]),
            self.display.privacy_tier,
        )
        .cloned() else {
            return;
        };
        let (mut events, cancel) =
            self.backend
                .start_agent_turn(agent, self.display.privacy_tier, question.clone());
        self.assistant.entries.push(Entry::Question(question));
        self.assistant.active = Some(cancel);
        self.assistant.cancelling = false;
        self.assistant
            .question
            .update(cx, |field, cx| field.set_text(String::new(), cx));
        // La même boucle que pour un fournisseur : le panneau ne sait pas
        // laquelle des deux sortes lui parle, et c'est ce qui lui évite deux
        // affichages pour une seule conversation.
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_ai_event(event, cx))
                    .is_err()
                {
                    return;
                }
            }
            // Le tour fini, le panneau se rouvre. Sans cette remise à zéro,
            // `active` resterait `Some` et `ask_assistant` sortirait à sa
            // première ligne pour toujours : le panneau serait mort pour la
            // session, sans qu'aucun message ne le dise.
            let _ = this.update(cx, |this, cx| {
                this.assistant.active = None;
                this.assistant.cancelling = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn ask_assistant(&mut self, cx: &mut Context<'_, Self>) {
        if self.assistant.active.is_some() {
            return;
        }
        if !matches!(
            self.assistant
                .entry(self.display.privacy_tier, self.capabilities),
            AskAi::Enabled
        ) {
            return;
        }
        let question = self.assistant.question.read(cx).text().trim().to_owned();
        if question.is_empty() {
            return;
        }
        let providers = self.assistant.providers.as_deref().unwrap_or(&[]);
        let Some(provider) = provider_for(providers, self.display.privacy_tier) else {
            // Aucun fournisseur utilisable : un agent externe déclaré peut
            // prendre le relais, si le niveau de la connexion l'autorise.
            //
            // L'ordre est délibéré — le fournisseur d'abord. Il est le seul
            // dont Oxyn sache où vont les données, et préférer ce qu'on peut
            // vérifier à ce qu'on ne peut que nommer n'a pas besoin d'autre
            // justification ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
            self.ask_external_agent(question, cx);
            return;
        };
        let request = ConversationRequest {
            provider: provider.config.clone(),
            connection: self.connection,
            session: self.session,
            tier: self.display.privacy_tier,
            language: QueryLanguage::Sql(self.dialect),
            dialect: self.dialect,
            question: question.clone(),
        };
        let (mut events, cancel) = self.backend.start_conversation(request);
        self.assistant.entries.push(Entry::Question(question));
        self.assistant.active = Some(cancel);
        self.assistant.cancelling = false;
        self.assistant
            .question
            .update(cx, |field, cx| field.set_text(String::new(), cx));
        // The same shape the workspace already uses on the execution bus: spawn,
        // loop, `update`. Nothing blocks the UI thread, and the loop ends by
        // itself when the conversation closes its channel
        // ([I-05](../../../CLAUDE.md#i-05)).
        cx.spawn(async move |this, cx| {
            while let Some(event) = events.recv().await {
                if this
                    .update(cx, |this, cx| this.on_ai_event(event, cx))
                    .is_err()
                {
                    // The workspace is gone. The conversation is not this task's
                    // to stop: `Drop` cancelled its token already.
                    return;
                }
            }
            let _ = this.update(cx, |this, cx| {
                this.assistant.active = None;
                this.assistant.cancelling = false;
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    /// Folds one event of the conversation into the transcript.
    pub(super) fn on_ai_event(&mut self, event: AiEvent, cx: &mut Context<'_, Self>) {
        // Retenue avant d'être transmise : le transcrit n'a rien à en montrer,
        // mais tout texte que cette conversation propose la portera.
        if let AiEvent::Started(provenance) = &event {
            self.assistant.provenance = Some((**provenance).clone());
        }
        let name = self.display.name.clone();
        apply_event(&mut self.assistant.entries, event, self.connection, &name);
        self.assistant.scroll.scroll_to_bottom();
        cx.notify();
    }

    /// Asks the conversation to stop.
    ///
    /// The request is taken immediately; its effect is the provider's. The
    /// button stays offered either way, because a button that disappears at the
    /// moment it is pressed leaves the user with nothing to press again.
    pub(super) fn cancel_conversation(&mut self, cx: &mut Context<'_, Self>) {
        if let Some(cancel) = &self.assistant.active {
            cancel.cancel();
            self.assistant.cancelling = true;
            cx.notify();
        }
    }

    /// Puts a proposed statement in a console, as text and nothing else.
    ///
    /// Reuses the path a copied history entry already takes: a fresh console,
    /// the notice that says nothing ran, and the ordinary Run button. There is
    /// no second way to execute, and arriving here executes nothing
    /// ([I-07](../../../CLAUDE.md#i-07)).
    pub(super) fn open_proposal(&mut self, text: String, cx: &mut Context<'_, Self>) {
        // Sans provenance, on n'écrit pas. `None` dans un document ne veut pas
        // dire « on ne sait pas » : il veut dire « écrit par l'utilisateur »,
        // et c'est le contraire de la vérité pour un texte d'agent. Le cas se
        // produit aujourd'hui pour un agent externe, dont `Provenance` ne sait
        // pas encore décrire l'origine.
        let Some(provenance) = self.assistant.provenance.clone() else {
            self.assistant.entries.push(Entry::Notice(
                "This answer cannot be opened as a proposal: Oxyn cannot record where it came \
                 from, and an unmarked statement would look like one you wrote."
                    .to_owned(),
            ));
            cx.notify();
            return;
        };
        self.open_library_query(
            library::OpenQuery::Copy {
                text,
                title: "agent_proposal.sql".to_owned(),
                origin: "the assistant".to_owned(),
                // La marque naît ici, au seul endroit du parcours où l'on sait
                // que le texte vient d'un agent. Plus loin — dans la console,
                // dans l'autosauvegarde — cette information n'existe plus.
                provenance: Some(provenance),
            },
            cx,
        );
    }
}
