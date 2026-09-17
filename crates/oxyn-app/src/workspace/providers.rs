//! Ce qui relie l'écran de configuration des fournisseurs au bus.
//!
//! L'écran (`oxyn_ui::provider_settings`) ne connaît ni le bus, ni le
//! trousseau, ni les dates : il émet ce que l'utilisateur a décidé, et ce
//! module le traduit en `Command`. Une vue qui atteindrait le bus elle-même
//! créerait le second chemin d'exécution qu'[I-01](../../../CLAUDE.md#i-01)
//! interdit.
//!
//! # L'ordre d'écriture n'est pas symétrique
//!
//! La clé part au trousseau **avant** que la déclaration n'atteigne le bus,
//! comme pour une connexion. Un échec après l'écriture du trousseau laisse une
//! entrée orpheline, inoffensive parce qu'inatteignable ; l'inverse laisserait
//! une déclaration dont la clé n'existe pas, et l'utilisateur ne comprendrait
//! l'échec qu'au premier message envoyé.
//!
//! # Ce qui fait apparaître l'entrée `Ask AI`
//!
//! Rien de neuf : [`Workspace::reload_ai_providers`] est appelée après chaque
//! écriture réussie. Le bus d'exécution ne gagne **aucune** variante — ce qui a
//! changé n'est pas l'état d'une base mais une configuration locale, dont la
//! vue qui l'écrit connaît le sort
//! ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).

use super::*;

use gpui::SharedString;
use oxyn_core::ProviderId;
use oxyn_ui::provider_settings::{
    Operation, ProviderDraft, ProviderSettings, ProviderSettingsEvent,
};

use crate::ai::{config_of, declared};

impl Workspace {
    /// Abonne le workspace aux décisions de l'écran de configuration.
    pub(super) fn watch_provider_settings(
        settings: &Entity<ProviderSettings>,
        cx: &mut Context<'_, Self>,
    ) {
        cx.subscribe(settings, |this, _, event: &ProviderSettingsEvent, cx| {
            match event {
                ProviderSettingsEvent::SaveRequested(draft) => this.save_provider(draft, cx),
                ProviderSettingsEvent::RemovalConfirmed(rank) => this.remove_provider(*rank, cx),
                ProviderSettingsEvent::AgentRemovalConfirmed(rank) => {
                    this.remove_external_agent(*rank, cx);
                }
                ProviderSettingsEvent::AgentSaveRequested(draft) => {
                    this.save_external_agent(draft, cx);
                }
                // L'abandon n'a rien à traverser : aucune écriture n'a
                // commencé, et la vue a déjà oublié la clé.
                ProviderSettingsEvent::CancelRequested => {}
                // `ProviderSettingsEvent` est `#[non_exhaustive]` : une décision
                // ajoutée dans `oxyn-ui` est ignorée ici plutôt que d'empêcher
                // la compilation. Le prix est un geste sans effet, jamais un
                // geste mal interprété.
                _ => {}
            }
        })
        .detach();
    }

    /// Montre à l'écran ce que le workspace sait des déclarations.
    ///
    /// Appelée à chaque fois que la liste change : c'est le seul chemin qui
    /// alimente l'écran, et il part de la même lecture classifiée que l'entrée
    /// `Ask AI`. Deux lectures donneraient deux campagnes de résolution DNS et
    /// deux réponses qui peuvent diverger.
    pub(super) fn refresh_provider_settings(&self, cx: &mut Context<'_, Self>) {
        let Some(providers) = self.assistant.providers.as_ref() else {
            return;
        };
        let montrables = providers
            .iter()
            .map(|classified| {
                declared(&classified.config, classified.reach, classified.measured_at)
            })
            .collect();
        self.provider_settings
            .update(cx, |settings, cx| settings.set_providers(montrables, cx));
    }

    /// Montre à l'écran les agents externes déclarés.
    ///
    /// Séparée de `refresh_provider_settings` parce que les deux lectures sont
    /// indépendantes : celle des fournisseurs porte un classement de portée que
    /// celle des agents n'a pas, et les fusionner ferait attendre l'une pour
    /// l'autre sans raison.
    pub(super) fn refresh_agent_settings(&self, cx: &mut Context<'_, Self>) {
        let Some(agents) = self.assistant.agents.as_ref() else {
            return;
        };
        let montrables = agents
            .iter()
            .map(|agent| oxyn_ui::provider_settings::DeclaredAgent {
                label: agent.label.clone().into(),
                command: agent.command.clone().into(),
                args: agent.args.len(),
            })
            .collect();
        self.provider_settings
            .update(cx, |settings, cx| settings.set_agents(montrables, cx));
    }

    /// Écrit la clé au trousseau, puis la déclaration au bus.
    fn save_provider(&mut self, draft: &ProviderDraft, cx: &mut Context<'_, Self>) {
        self.provider_settings.update(cx, |settings, cx| {
            settings.set_working(Operation::Saving, cx)
        });

        let draft = draft.clone();
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            // Frappée **une fois**, avant tout le reste : la référence de
            // trousseau en dérive, et la déclaration doit porter exactement
            // celle sous laquelle la clé a été rangée. Deux frappes, même
            // réconciliées ensuite, feraient tenir cet appariement à une ligne
            // d'affectation que rien ne garde.
            let identite = ProviderId::for_new_declaration(draft.kind);
            let reference = match draft.key.clone() {
                Some(key) if !key.trim().is_empty() => {
                    match backend.store_provider_key(identite.clone(), key).await {
                        Ok(Ok(reference)) => Some(reference),
                        Ok(Err(erreur)) => {
                            Self::provider_failed(&this, cx, erreur.to_string());
                            return;
                        }
                        Err(_) => {
                            Self::provider_failed(&this, cx, "the keyring stopped answering");
                            return;
                        }
                    }
                }
                _ => None,
            };

            let config = config_of(identite, &draft, reference);

            let ecrit = backend.dispatch(
                CommandId::new(),
                Command::SaveAiProvider {
                    config: Box::new(config),
                },
                CancelToken::new(),
            );
            match ecrit.await {
                Ok(Ok(_)) => {
                    let _ = this.update(cx, |this: &mut Self, cx| this.reload_ai_providers(cx));
                }
                Ok(Err(erreur)) => Self::provider_failed(&this, cx, erreur.to_string()),
                Err(_) => Self::provider_failed(&this, cx, "the local worker stopped answering"),
            }
        })
        .detach();
    }

    /// Déclare un agent externe.
    ///
    /// Plus court que son jumeau, et pour la même raison que le retrait : **il
    /// n'y a pas de clé à ranger au trousseau d'abord**. La déclaration part
    /// directement au bus, et l'identité est frappée ici comme pour un
    /// fournisseur — jamais dérivée du nom, que l'utilisateur peut donner deux
    /// fois identique sans vouloir remplacer quoi que ce soit.
    fn save_external_agent(
        &mut self,
        draft: &oxyn_ui::provider_settings::row::AgentDraft,
        cx: &mut Context<'_, Self>,
    ) {
        self.provider_settings.update(cx, |settings, cx| {
            settings.set_working(Operation::Saving, cx)
        });

        let identite = oxyn_core::ProviderId::for_new_agent();
        let agent = oxyn_core::ExternalAgentConfig::new(identite, &draft.label, &draft.command)
            .with_args(oxyn_ui::provider_settings::row::parse_args(&draft.args));
        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let ecrit = backend.dispatch(
                CommandId::new(),
                Command::SaveExternalAgent {
                    agent: Box::new(agent),
                },
                CancelToken::new(),
            );
            match ecrit.await {
                Ok(Ok(_)) => {
                    let _ = this.update(cx, |this: &mut Self, cx| this.reload_external_agents(cx));
                }
                Ok(Err(erreur)) => Self::provider_failed(&this, cx, erreur.to_string()),
                Err(_) => Self::provider_failed(&this, cx, "the local worker stopped answering"),
            }
        })
        .detach();
    }

    /// Retire l'agent externe de ce rang.
    ///
    /// Plus court que son jumeau, et c'est le sujet : **il n'y a pas de clé à
    /// oublier**. Le geste s'arrête au bus.
    fn remove_external_agent(&mut self, rank: usize, cx: &mut Context<'_, Self>) {
        let Some(agent) = self
            .assistant
            .agents
            .as_ref()
            .and_then(|agents| agents.get(rank))
            .map(|agent| agent.id.clone())
        else {
            // Le rang vient d'une liste que l'écran a reçue ; elle a pu changer
            // depuis. Recharger vaut mieux que retirer la mauvaise ligne.
            self.reload_external_agents(cx);
            return;
        };

        self.provider_settings.update(cx, |settings, cx| {
            settings.set_working(Operation::Removing, cx);
        });

        let backend = self.backend.clone();
        cx.spawn(async move |this, cx| {
            let retire = backend.dispatch(
                CommandId::new(),
                Command::RemoveExternalAgent { id: agent },
                CancelToken::new(),
            );
            match retire.await {
                Ok(Ok(_)) => {
                    let _ = this.update(cx, |this: &mut Self, cx| this.reload_external_agents(cx));
                }
                Ok(Err(erreur)) => Self::provider_failed(&this, cx, erreur.to_string()),
                Err(_) => Self::provider_failed(&this, cx, "the local worker stopped answering"),
            }
        })
        .detach();
    }

    /// Retire la déclaration de ce rang, et oublie sa clé.
    fn remove_provider(&mut self, rank: usize, cx: &mut Context<'_, Self>) {
        let Some(config) = self
            .assistant
            .providers
            .as_ref()
            .and_then(|providers| providers.get(rank))
            .map(|classified| classified.config.clone())
        else {
            // Le rang vient d'une liste que l'écran a reçue ; elle a pu changer
            // depuis. Recharger vaut mieux que retirer la mauvaise ligne.
            self.reload_ai_providers(cx);
            return;
        };

        self.provider_settings.update(cx, |settings, cx| {
            settings.set_working(Operation::Removing, cx);
        });

        let backend = self.backend.clone();
        let secret_ref = config.secret_ref.clone();
        cx.spawn(async move |this, cx| {
            let retire = backend.dispatch(
                CommandId::new(),
                Command::RemoveAiProvider { id: config.id },
                CancelToken::new(),
            );
            match retire.await {
                Ok(Ok(_)) => {
                    // La clé part **après** la déclaration. Une clé oubliée
                    // d'abord, suivie d'un échec de retrait, laisserait une
                    // déclaration muette que rien n'explique.
                    if let Some(reference) = secret_ref {
                        backend.forget_provider_key(reference);
                    }
                    let _ = this.update(cx, |this: &mut Self, cx| this.reload_ai_providers(cx));
                }
                Ok(Err(erreur)) => Self::provider_failed(&this, cx, erreur.to_string()),
                Err(_) => Self::provider_failed(&this, cx, "the local worker stopped answering"),
            }
        })
        .detach();
    }

    /// Dit l'échec à l'écran, sans le paraphraser.
    fn provider_failed(
        this: &gpui::WeakEntity<Self>,
        cx: &mut gpui::AsyncApp,
        message: impl Into<SharedString>,
    ) {
        let message = message.into();
        let _ = this.update(cx, |this: &mut Self, cx| {
            this.provider_settings.update(cx, |settings, cx| {
                // Rien de ce qui échoue ici ne se rejoue tout seul : une écriture
                // de trousseau ou de store se retente à la main, en connaissance
                // de cause (I-13).
                settings.set_failed(message.clone(), false, cx);
            });
        });
    }
}
