//! Ce qui monte une conversation d'agent, entièrement hors du fil d'interface.
//!
//! Trois choses n'existent qu'ici, parce qu'elles ne sont possibles nulle part
//! ailleurs sans enfreindre un invariant :
//!
//! * **la clé sort du trousseau**, au dernier moment, et n'est jamais persistée
//!   à côté de la déclaration ([I-03](../../../CLAUDE.md#i-03)) ;
//! * **le point d'accès est reclassé**, par une résolution DNS bloquante, à
//!   chaque ouverture — jamais lu depuis la base
//!   ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)) ;
//! * **le contexte est assemblé** sous le niveau de la connexion, par la porte
//!   unique de `oxyn-ai` ([I-04](../../../CLAUDE.md#i-04)).
//!
//! Tout cela est bloquant ou long. Rien n'en revient sur le fil d'interface
//! autrement que par le canal d'événements ([I-05](../../../CLAUDE.md#i-05)).

use super::*;

use chrono::{DateTime, Utc};
use oxyn_ai::{AgentRuntime, AgentSession, ContextBuilder, ToolRegistry, ToolScope, sql_agent};
use oxyn_core::{AiProviderConfig, PrivacyTier, ProviderId, QueryLanguage};
use oxyn_llm::Reach;
use tokio::sync::mpsc::UnboundedReceiver;

use crate::ai::{AgentSink, AiEvent, ChannelObserver};

/// Ce qu'il faut savoir pour ouvrir une conversation.
///
/// Le niveau de confidentialité y figure explicitement, et vient de la
/// **connexion** : le prendre sur la session ou sur le fournisseur ferait
/// exactement ce qu'ADR-0006 refuse — un réglage pris sur une base de test
/// s'appliquant à la base client ouverte trois jours plus tard.
pub(crate) struct ConversationRequest {
    /// La déclaration choisie par l'utilisateur.
    pub(crate) provider: AiProviderConfig,
    /// La connexion sur laquelle l'agent travaille.
    pub(crate) connection: ConnectionId,
    /// La session ouverte sur cette connexion.
    pub(crate) session: SessionId,
    /// Le niveau de **cette connexion**.
    pub(crate) tier: PrivacyTier,
    /// Le langage de requête de la connexion.
    pub(crate) language: QueryLanguage,
    /// Le dialecte, pour que les propositions soient écrites dans celui de la
    /// base et non en ANSI générique.
    pub(crate) dialect: SqlDialect,
    /// La question de l'utilisateur.
    pub(crate) question: String,
}

impl Backend {
    /// Lit les fournisseurs déclarés.
    ///
    /// C'est ce qui décide si le workspace IA existe : une liste vide n'est pas
    /// un état dégradé, c'est l'absence d'entrée dans l'interface
    /// ([ADR-0006](../../../docs/adr/0006-ai-privacy-tiers.md)).
    pub(crate) fn ai_providers(
        &self,
    ) -> oneshot::Receiver<Result<Vec<AiProviderConfig>, OxynError>> {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let issue = inner
                .executor
                .dispatch(Actor::Human, Command::ListAiProviders, &CancelToken::new())
                .await;
            let _ = sender.send(match issue {
                Ok(Outcome::AiProvidersListed { providers }) => Ok(providers),
                Ok(autre) => Err(OxynError::Internal(format!(
                    "listing AI providers answered with {autre:?}"
                ))),
                Err(erreur) => Err(erreur),
            });
        });
        receiver
    }

    /// Liste les agents externes déclarés, hors du fil d'interface.
    ///
    /// Aucun classement de portée ne l'accompagne, à la différence des
    /// fournisseurs : celle d'un agent externe est **inconnaissable**, et il n'y
    /// a donc rien à mesurer ni à dater
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    pub(crate) fn external_agents(
        &self,
    ) -> oneshot::Receiver<Result<Vec<oxyn_core::ExternalAgentConfig>, OxynError>> {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let issue = inner
                .executor
                .dispatch(
                    Actor::Human,
                    Command::ListExternalAgents,
                    &CancelToken::new(),
                )
                .await;
            let _ = sender.send(match issue {
                Ok(Outcome::ExternalAgentsListed { agents }) => Ok(agents),
                Ok(autre) => Err(OxynError::Internal(format!(
                    "listing external agents answered with {autre:?}"
                ))),
                Err(erreur) => Err(erreur),
            });
        });
        receiver
    }

    /// Écrit la clé d'un fournisseur dans le trousseau, hors du fil d'interface.
    ///
    /// Rend la référence à persister avec la déclaration. L'ordre importe et
    /// c'est le même que pour une connexion : le trousseau d'abord, la
    /// déclaration ensuite. Un échec après l'écriture laisse une entrée
    /// orpheline, inoffensive parce qu'inatteignable ; l'inverse laisserait une
    /// déclaration dont la clé n'existe pas.
    pub(crate) fn store_provider_key(
        &self,
        provider: ProviderId,
        key: String,
    ) -> oneshot::Receiver<Result<String, OxynError>> {
        let (sender, receiver) = oneshot::channel();
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let issue = tokio::task::spawn_blocking(move || {
                inner
                    .credentials
                    .store_provider_key(&provider, &key)
                    .map(|reference| reference.as_str().to_owned())
            })
            .await
            .unwrap_or_else(|erreur| {
                Err(OxynError::Internal(format!(
                    "the keyring task did not finish: {erreur}"
                )))
            });
            let _ = sender.send(issue);
        });
        receiver
    }

    /// Oublie la clé d'une déclaration retirée, sans rien attendre.
    ///
    /// Rien n'est rendu, et c'est délibéré : un échec ici laisse une entrée de
    /// trousseau que plus aucune déclaration ne référence, donc inatteignable.
    /// Le faire remonter obligerait l'écran à annoncer un échec sur une
    /// suppression qui, elle, a bien eu lieu — et l'utilisateur chercherait
    /// une déclaration qui n'est plus là. Le crier suffit.
    pub(crate) fn forget_provider_key(&self, reference: String) {
        let inner = Arc::clone(&self.inner);
        self.runtime.spawn(async move {
            let issue = tokio::task::spawn_blocking(move || {
                inner.credentials.forget_provider_key(&reference)
            })
            .await;
            match issue {
                Ok(Ok(())) => {}
                Ok(Err(erreur)) => {
                    tracing::warn!(error = %erreur, "orphan provider key left in the keyring");
                }
                Err(erreur) => {
                    tracing::warn!(error = %erreur, "the keyring task did not finish");
                }
            }
        });
    }

    /// Ouvre une conversation et rend de quoi la suivre et l'arrêter.
    ///
    /// Retourne **immédiatement** : ce qui suit se déroule sur le runtime. Le
    /// canal porte chaque étape dans l'ordre où elle a lieu, et se ferme quand
    /// la conversation est finie — répondue, annulée, bornée par le plafond de
    /// tours, ou interrompue par une panne du fournisseur.
    ///
    /// Le jeton rendu est celui que le panneau annule. Il est propagé jusqu'au
    /// fournisseur **et** jusqu'aux commandes que l'agent a soumises : un
    /// `SELECT` de quarante secondes lancé par un agent s'arrête avec la
    /// conversation, il ne lui survit pas.
    pub(crate) fn start_conversation(
        &self,
        request: ConversationRequest,
    ) -> (UnboundedReceiver<AiEvent>, CancelToken) {
        let (observer, receiver) = ChannelObserver::new();
        let cancel = CancelToken::new();
        let inner = Arc::clone(&self.inner);
        let jeton = cancel.clone();
        self.runtime.spawn(async move {
            if let Err(erreur) = converse(&inner, request, &observer, &jeton).await {
                observer.send(AiEvent::Failed(erreur));
            }
        });
        (receiver, cancel)
    }
}

impl Backend {
    /// Démarre un tour avec un **agent externe**, hors du fil d'interface.
    ///
    /// Même forme que [`start_conversation`](Self::start_conversation) — un
    /// canal d'événements et un jeton d'annulation — pour que le panneau n'ait
    /// pas à savoir laquelle des deux sortes lui parle. Ce qui change est
    /// derrière : pas de clé, pas de point de passage vers un fournisseur, et
    /// le refus de niveau tombe **avant** le lancement du processus
    /// ([ADR-0026](../../../docs/adr/0026-agents-externes-acp.md)).
    pub(crate) fn start_agent_turn(
        &self,
        agent: oxyn_core::ExternalAgentConfig,
        tier: oxyn_core::PrivacyTier,
        question: String,
    ) -> (UnboundedReceiver<AiEvent>, CancelToken) {
        let (observer, receiver) = ChannelObserver::new();
        let cancel = CancelToken::new();
        // Le jeton part **dans** la tâche autant qu'il revient à l'appelant :
        // rendu sans être transmis, il ferait un bouton d'arrêt qui change
        // l'affichage pendant que le processus enfant continue de parler à un
        // service dont Oxyn ne sait pas où il est.
        let pour_la_tache = cancel.clone();
        self.runtime.spawn(async move {
            let observateur: Arc<dyn oxyn_ai::AgentObserver> = Arc::new(observer.clone());
            // L'invite naît **ici**, par la seule porte qui sait la
            // fabriquer, et le niveau y est appliqué avant que quoi que ce soit
            // ne parte ([ADR-0027](../../../docs/adr/0027-porte-unique-pour-les-deux-destinations.md)).
            let invite = match oxyn_ai::external::prompt::AgentPrompt::from_user(tier, &question) {
                Ok(invite) => invite,
                Err(erreur) => {
                    observer.send(AiEvent::Failed(erreur.to_string()));
                    return;
                }
            };
            match oxyn_ai::external::turn::run_turn(
                &agent,
                tier,
                &invite,
                &pour_la_tache,
                observateur,
            )
            .await
            {
                // La raison d'arrêt du protocole se traduit, elle ne se jette
                // pas. `MaxTokens` devient `truncated` : une réponse coupée qui
                // ne le dit pas ressemble à une réponse fausse, et c'est
                // exactement ce que ce champ existe pour empêcher.
                //
                // `text` reste vide : les fragments sont déjà passés par
                // l'observateur, et les recopier ici les afficherait deux fois.
                Ok(fin) => observer.send(AiEvent::Finished(match fin {
                    oxyn_ai::external::turn::TurnEnd::Cancelled => {
                        oxyn_ai::AgentOutcome::Cancelled { turns: 1 }
                    }
                    oxyn_ai::external::turn::TurnEnd::Answered { truncated } => {
                        oxyn_ai::AgentOutcome::Answered {
                            text: String::new(),
                            turns: 1,
                            truncated,
                            // An agent's protocol folds its reasons into
                            // `truncated`: no finer one to report.
                            stop: oxyn_core::ai::StopReason::Unspecified,
                        }
                    }
                    // `TurnEnd` est `#[non_exhaustive]` : une fin ajoutée dans
                    // `oxyn-ai` ne doit pas se lire comme une réponse aboutie.
                    _ => oxyn_ai::AgentOutcome::Cancelled { turns: 1 },
                })),
                // Le message du domaine passe tel quel : il ne recopie ni
                // l'invite ni la commande, c'est vérifié là où il est construit.
                Err(erreur) => observer.send(AiEvent::Failed(erreur.to_string())),
            }
        });
        (receiver, cancel)
    }
}

/// Monte la conversation, puis la déroule.
///
/// Rend une `Err` **seulement** pour ce qui empêche de commencer ou ce qui
/// vient du fournisseur. Un refus du `PolicyGate`, un échec de requête, une
/// annulation : ce sont des étapes de la conversation, elles passent par
/// l'observateur et n'apparaissent pas ici.
async fn converse(
    inner: &Arc<Inner>,
    request: ConversationRequest,
    observer: &ChannelObserver,
    cancel: &CancelToken,
) -> Result<(), String> {
    let ConversationRequest {
        provider,
        connection,
        session,
        tier,
        language,
        dialect,
        question,
    } = request;

    // La clé et le classement du point d'accès sont deux lectures bloquantes —
    // trousseau du système et résolution DNS. Elles partent ensemble sur le
    // pool bloquant, et ni l'une ni l'autre n'est mise en cache : une clé
    // révoquée doit cesser de marcher, et un nom qui résolvait vers la boucle
    // locale hier peut résoudre ailleurs aujourd'hui (ADR-0023).
    let declaration = provider.clone();
    let identifiants = Arc::clone(&inner.credentials);
    let assemblage = tokio::task::spawn_blocking(move || {
        let key = identifiants
            .provider_key(&declaration)
            .map_err(|erreur| erreur.to_string())?;
        let transport = oxyn_llm::build_provider(declaration.kind, &declaration.base_url, key)
            .map_err(|erreur| erreur.to_string())?;
        let reach = oxyn_llm::endpoint_reach(&declaration.base_url);
        Ok::<_, String>((transport, reach))
    })
    .await
    .map_err(|erreur| format!("the provider could not be prepared: {erreur}"))??;
    let (transport, reach) = assemblage;

    // Le contexte se construit à partir du catalogue **local** : un agent qui
    // irait chercher lui-même ce dont il a besoin contournerait à la fois la
    // porte unique et le command bus.
    let catalogue = inner
        .executor
        .catalog(connection)
        .ok_or_else(|| "this connection has no catalogue to work from yet".to_owned())?;
    let contexte = {
        let cache = catalogue.read();
        ContextBuilder::new(&cache, tier)
            .with_dialect(dialect)
            .focused_on(question.clone())
            .build()
    };

    let agent = sql_agent();
    let moteur = AgentRuntime::new(
        agent.clone(),
        transport,
        reach,
        ToolRegistry::builtin(),
        provider.model.clone(),
    )
    .map_err(|erreur| erreur.to_string())?;

    let perimetre = ToolScope::new(connection, session, language);
    let mut conversation = AgentSession::new(&agent, &contexte, perimetre);
    conversation.ask(question);

    // L'identité de la conversation est frappée ici et nulle part ailleurs. Le
    // panneau en a besoin pour marquer ce que cette conversation écrira ; sans
    // cet événement il ne la connaîtrait jamais, et une proposition arriverait
    // dans une console sans qu'on puisse dire d'où elle vient (ADR-0023).
    observer.send(AiEvent::Started(Box::new(oxyn_core::Provenance::new(
        agent.id,
        conversation.id(),
        provider.kind,
        provider.model.clone(),
    ))));

    // Le puits est **attaché** à cette conversation : une commande portant un
    // autre acteur, `Actor::Human` compris, est refusée sans atteindre
    // l'ordonnanceur. C'est ce qui empêche un agent de se présenter comme
    // l'utilisateur et d'échapper à la moitié « agent » de la matrice de
    // politique.
    let puits = AgentSink::for_agent(Arc::clone(&inner.executor), agent.id, conversation.id());

    match moteur
        .run(&mut conversation, &puits, observer, cancel)
        .await
    {
        Ok(_) => Ok(()),
        // `run` rend l'erreur plutôt que de l'annoncer elle-même : l'annoncer
        // des deux côtés donnerait deux affichages pour un seul incident.
        Err(erreur) => Err(erreur.to_string()),
    }
}

/// A declared provider, together with where its endpoint resolves.
///
/// The classification travels **beside** the declaration and is never written
/// next to it: `Reach` has no column, and a resolution from yesterday applied to
/// a send from today is exactly the proxy trap
/// ([ADR-0023](../../../docs/adr/0023-fournisseurs-declares-et-provenance.md)).
#[derive(Debug, Clone)]
pub(crate) struct ClassifiedProvider {
    /// The declaration itself.
    pub(crate) config: AiProviderConfig,
    /// Where the endpoint resolved, and when.
    ///
    /// The full [`Reach`] and not a boolean, because two callers need different
    /// halves of it and one classification must serve both: the panel only asks
    /// "does this leave the machine", while the settings screen must tell
    /// `Remote` from `Unresolved` — it says "unresolved" rather than claiming a
    /// measurement that did not happen. Reducing this to a flag here would force
    /// the settings screen to resolve every endpoint a second time, which is a
    /// second DNS campaign and a second place for the two answers to disagree.
    pub(crate) reach: Reach,
    /// When the classification was measured, for the settings screen to show
    /// beside it. Never persisted — it dates this session's measurement.
    pub(crate) measured_at: DateTime<Utc>,
}

impl ClassifiedProvider {
    /// Does the endpoint stay on this machine?
    ///
    /// `false` for one Oxyn could not resolve: the doubt does not benefit the
    /// send, and [`Reach::leaves_machine`] already holds that rule — this is a
    /// reading of it, not a second copy.
    pub(crate) const fn is_local(&self) -> bool {
        !self.reach.leaves_machine()
    }
}

impl Backend {
    /// Lists the declared providers and classifies each endpoint.
    ///
    /// Two blocking things happen here and neither may happen on the UI thread:
    /// a SQLite read, then one DNS resolution per declaration
    /// ([I-05](../../../CLAUDE.md#i-05)).
    ///
    /// This is what lets the connection bar decide, for a connection in
    /// `PrivacyTier::Local`, whether any declared provider is usable at all. It
    /// is an **affordance, not a gate**: `AgentRuntime::run` re-checks the tier
    /// against the reach it measured itself, so a stale classification here can
    /// only ever cost a refusal, never a send.
    pub(crate) fn ai_providers_classified(
        &self,
    ) -> oneshot::Receiver<Result<Vec<ClassifiedProvider>, OxynError>> {
        let (sender, receiver) = oneshot::channel();
        let listed = self.ai_providers();
        self.runtime.spawn(async move {
            let providers = match listed.await {
                Ok(Ok(providers)) => providers,
                Ok(Err(erreur)) => {
                    let _ = sender.send(Err(erreur));
                    return;
                }
                Err(_) => {
                    let _ = sender.send(Err(OxynError::Internal(
                        "the provider listing worker stopped answering".to_owned(),
                    )));
                    return;
                }
            };
            let classified = tokio::task::spawn_blocking(move || {
                providers
                    .into_iter()
                    .map(|config| {
                        let reach = oxyn_llm::endpoint_reach(&config.base_url);
                        ClassifiedProvider {
                            config,
                            reach,
                            measured_at: Utc::now(),
                        }
                    })
                    .collect::<Vec<_>>()
            })
            .await;
            let _ = sender.send(classified.map_err(|erreur| {
                OxynError::Internal(format!(
                    "the endpoint classification did not finish: {erreur}"
                ))
            }));
        });
        receiver
    }
}
