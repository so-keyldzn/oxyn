//! De la trame SSE aux événements du domaine, côté Anthropic.
//!
//! Ce module tient l'état d'un flux de génération. Il est **pur** : aucune
//! entrée-sortie, aucun réseau. C'est ce qui permet de l'éprouver sur les
//! séquences réelles du fournisseur, y compris leurs bizarreries, sans en
//! appeler un.
//!
//! # Ce protocole nomme ses blocs, et c'est toute la différence
//!
//! Là où un flux compatible OpenAI recolle des fragments reliés par un `index`
//! de tableau, celui-ci ouvre et ferme explicitement chaque bloc de contenu :
//! `content_block_start`, des `content_block_delta`, `content_block_stop`. Le
//! type du bloc est donné à l'ouverture, et les deltas qui suivent en
//! dépendent — un `input_json_delta` n'a de sens que dans un bloc `tool_use`.
//!
//! On en tire deux règles :
//!
//! 1. **L'état d'un bloc est tenu à l'index qu'annonce le serveur**, jamais
//!    déduit de l'ordre d'arrivée ;
//! 2. **un delta dont le bloc est inconnu est ignoré**, pas deviné. Le seul
//!    autre choix serait d'inventer un bloc, donc d'inventer son type.
//!
//! # Quand `Done` est émis
//!
//! `message_delta` porte la raison d'arrêt mais **ne termine pas** le flux :
//! `message_stop` suit, et la consommation cumulée arrive avec le
//! `message_delta`. On mémorise donc la raison et on n'émet `Done` qu'au
//! `message_stop`, à la fermeture du flux, ou sur une erreur. Exactement une
//! fois.
//!
//! # Une fin constatée n'est pas une fin annoncée
//!
//! Le protocole conclut tout message par `message_stop`, et tout bloc par
//! `content_block_stop`. Leur absence est donc une **information** : un
//! mandataire ou un répartiteur de charge qui ferme proprement la connexion en
//! pleine génération ne produit aucune erreur de transport, seulement un flux
//! qui s'arrête.
//!
//! D'où deux règles, qui valent quelle que soit la façon dont le flux s'est
//! fermé :
//!
//! 1. **un flux sans `message_stop` est [`StopReason::Interrupted`]** — même si
//!    un `message_delta` avait annoncé une raison : le serveur a peut-être
//!    produit, et facturé, davantage que ce qui a été reçu (I-13) ;
//! 2. **seul `content_block_stop` clôt un bloc.** Un appel d'outil dont le JSON
//!    se lit par chance n'est pas un appel que le modèle a fini d'écrire : il
//!    est jeté, jamais proposé.
//!
//! # Tout ce qui s'accumule est compté
//!
//! Chaque bloc ouvert, chaque fragment de texte, de raisonnement, de
//! signature ou d'arguments passe par un [`GenerationBudget`] **avant** d'être
//! retenu ou émis. Un bloc ouvert sans jamais être fermé coûte lui aussi : il
//! porte un état jusqu'à sa fermeture. Au premier dépassement, la génération
//! s'arrête : voir [`crate::budget`].

use std::collections::BTreeMap;

use super::wire::{ContentBlock, Envelope, WireError};
use crate::budget::{BudgetExceeded, GenerationBudget};
use crate::error::classify_json_error;
use crate::reasoning::ReasoningBlock;
use crate::sse::SseFrame;
use crate::stream::EventDecoder;
use crate::types::{ChatEvent, StopReason, ToolCall};

/// Nombre de trames illisibles tolérées avant d'abandonner le flux.
///
/// Une trame illisible isolée arrive ; une série signifie qu'on ne parle pas le
/// même protocole, et continuer ne ferait qu'inonder l'interface d'erreurs.
pub(crate) const MAX_DECODE_ERRORS: usize = 8;

/// Ce qu'un bloc de contenu accumule, selon son type.
#[derive(Debug)]
enum PartialBlock {
    /// Bloc de texte : rien à accumuler, les fragments sont émis au vol.
    Text,
    /// Appel d'outil en cours de reconstruction.
    ToolUse {
        id: String,
        name: String,
        arguments: String,
    },
    /// Raisonnement en cours de reconstruction.
    Thinking {
        text: String,
        signature: Option<String>,
    },
    /// Raisonnement chiffré : complet dès son ouverture.
    RedactedThinking { data: String },
    /// Bloc d'un type que cette version ne connaît pas.
    ///
    /// Il est **suivi** plutôt qu'ignoré : sans lui, les deltas qui le visent
    /// seraient comptés comme visant un bloc inconnu, et une trame parfaitement
    /// valide passerait pour une anomalie.
    Unknown,
}

/// État d'un flux de génération Anthropic.
#[derive(Debug, Default)]
pub(crate) struct MessageDecoder {
    blocks: BTreeMap<u32, PartialBlock>,
    budget: GenerationBudget,
    stop: Option<StopReason>,
    done: bool,
    errors: usize,
}

impl MessageDecoder {
    /// Décodeur pour un flux neuf.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Le flux est-il terminé ?
    pub(crate) fn finished(&self) -> bool {
        self.done
    }

    /// Consomme le champ `data` d'une trame SSE.
    pub(crate) fn on_data(&mut self, data: &str, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        let data = data.trim();
        if data.is_empty() {
            return;
        }

        let trame: Envelope = match serde_json::from_str(data) {
            Ok(trame) => trame,
            Err(err) => {
                // La trame fautive n'est pas recopiée : on ne sait pas ce qu'un
                // mandataire y met.
                self.errors += 1;
                out.push(ChatEvent::Error(format!(
                    "unreadable stream frame ({}, line {}, column {})",
                    classify_json_error(&err),
                    err.line(),
                    err.column()
                )));
                if self.errors >= MAX_DECODE_ERRORS {
                    self.stop = Some(StopReason::Interrupted);
                    self.emit_done(out);
                }
                return;
            }
        };

        let compte = match trame.r#type.as_deref() {
            Some("content_block_start") => self.on_block_start(&trame, out),
            Some("content_block_delta") => self.on_block_delta(&trame, out),
            _ => Ok(()),
        };
        if let Err(limite) = compte {
            self.exceed(limite, out);
            return;
        }

        match trame.r#type.as_deref() {
            Some("message_start") => self.on_message_start(&trame, out),
            // Traités au-dessus, où leur coût est compté.
            Some("content_block_start" | "content_block_delta") => {}
            Some("content_block_stop") => self.on_block_stop(&trame, out),
            Some("message_delta") => self.on_message_delta(&trame, out),
            Some("message_stop") => self.on_message_stop(out),
            Some("error") => self.on_error(trame.error.as_ref(), out),
            // `ping` est un maintien de connexion : il n'a rien à dire.
            Some("ping") => {}
            // La documentation annonce que de nouveaux types d'événements
            // peuvent apparaître. Un type inconnu s'ignore : le refuser
            // casserait le flux à la première évolution du protocole.
            _ => {}
        }
    }

    /// `message_start` : la consommation d'entrée est déjà connue.
    fn on_message_start(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(usage) = trame.message.as_ref().and_then(|m| m.usage.as_ref()) else {
            return;
        };
        if usage.is_empty() {
            return;
        }
        // Les jetons de cache ne sont annoncés qu'ici : le `message_delta`
        // final ne les répète pas toujours. Les taire attendrait une trame qui
        // ne viendra peut-être pas.
        out.push(ChatEvent::Usage {
            prompt_tokens: usage.input(),
            completion_tokens: usage.output(),
            cache_write_tokens: usage.cache_write(),
            cache_read_tokens: usage.cache_read(),
            // Ce protocole ne facture pas le raisonnement à part : il est
            // compté dans la sortie. Déclarer `Some(0)` serait faux.
            reasoning_tokens: None,
        });
    }

    /// `content_block_start` : un bloc s'ouvre, son type est donné.
    ///
    /// # Erreurs
    /// Le budget que ce bloc dépasserait ; il n'est alors pas ouvert.
    fn on_block_start(
        &mut self,
        trame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let Some(index) = trame.index else {
            return Ok(());
        };
        let Some(bloc) = trame.content_block.as_ref() else {
            return Ok(());
        };
        let longueur = |champ: &Option<String>| champ.as_ref().map_or(0, String::len);
        if bloc.r#type.as_deref() == Some("tool_use") {
            self.budget.open_tool_call()?;
            self.budget
                .charge_tool_name(index, 0, longueur(&bloc.name))?;
            self.budget.charge(longueur(&bloc.id))?;
        } else {
            self.budget.open_block()?;
            self.budget.charge(
                longueur(&bloc.thinking)
                    .saturating_add(longueur(&bloc.signature))
                    .saturating_add(longueur(&bloc.data)),
            )?;
        }

        let partiel = match bloc.r#type.as_deref() {
            Some("text") => PartialBlock::Text,
            Some("tool_use") => {
                let id = bloc.id.clone().unwrap_or_else(|| synthetic_id(index));
                let name = bloc.name.clone().unwrap_or_default();
                if !name.is_empty() {
                    out.push(ChatEvent::ToolCallStarted {
                        index,
                        id: id.clone(),
                        name: name.clone(),
                    });
                }
                PartialBlock::ToolUse {
                    id,
                    name,
                    arguments: String::new(),
                }
            }
            Some("thinking") => PartialBlock::Thinking {
                text: bloc.thinking.clone().unwrap_or_default(),
                signature: bloc.signature.clone(),
            },
            Some("redacted_thinking") => PartialBlock::RedactedThinking {
                data: bloc.data.clone().unwrap_or_default(),
            },
            // `server_tool_use`, résultats de recherche web, blocs à venir : ils
            // existent, Oxyn n'en propose aucun, et ils ne produisent rien ici.
            _ => PartialBlock::Unknown,
        };
        self.blocks.insert(index, partiel);
        Self::note_unused(bloc);
        Ok(())
    }

    /// Accepte sans rien faire un bloc dont on n'exploite pas les champs.
    ///
    /// Sert uniquement à rendre l'intention explicite au lecteur : tous les
    /// champs de [`ContentBlock`] sont lus au-dessus, et celui qui ne l'est pas
    /// l'est délibérément.
    const fn note_unused(_bloc: &ContentBlock) {}

    /// `content_block_delta` : un fragment arrive pour un bloc ouvert.
    ///
    /// # Erreurs
    /// Le budget que ce fragment dépasserait ; rien n'en est alors retenu.
    fn on_block_delta(
        &mut self,
        trame: &Envelope,
        out: &mut Vec<ChatEvent>,
    ) -> Result<(), BudgetExceeded> {
        let (Some(index), Some(delta)) = (trame.index, trame.delta.as_ref()) else {
            return Ok(());
        };
        // Un delta qui vise un bloc jamais ouvert s'ignore : inventer le bloc
        // reviendrait à inventer son type, donc le sens de ce qu'on accumule.
        let Some(bloc) = self.blocks.get_mut(&index) else {
            return Ok(());
        };
        let budget = &mut self.budget;

        match (delta.r#type.as_deref(), bloc) {
            (Some("text_delta"), PartialBlock::Text) => {
                if let Some(texte) = delta.text.clone()
                    && !texte.is_empty()
                {
                    budget.charge(texte.len())?;
                    out.push(ChatEvent::TextDelta(texte));
                }
            }
            (Some("input_json_delta"), PartialBlock::ToolUse { arguments, .. }) => {
                if let Some(fragment) = delta.partial_json.clone()
                    && !fragment.is_empty()
                {
                    budget.charge_tool_arguments(index, arguments.len(), fragment.len())?;
                    arguments.push_str(&fragment);
                    out.push(ChatEvent::ToolCallDelta {
                        index,
                        arguments: fragment,
                    });
                }
            }
            (Some("thinking_delta"), PartialBlock::Thinking { text, .. }) => {
                if let Some(fragment) = delta.thinking.clone()
                    && !fragment.is_empty()
                {
                    budget.charge(fragment.len())?;
                    text.push_str(&fragment);
                    out.push(ChatEvent::ReasoningDelta {
                        index,
                        text: fragment,
                    });
                }
            }
            (Some("signature_delta"), PartialBlock::Thinking { signature, .. }) => {
                // La signature arrive en une fois, juste avant la fermeture du
                // bloc. Elle ne s'émet pas : elle n'a de sens qu'au tour
                // suivant, et elle ne s'affiche jamais.
                if let Some(valeur) = delta.signature.clone() {
                    budget.charge(valeur.len())?;
                    *signature = Some(valeur);
                }
            }
            // Un delta dont le type ne correspond pas à celui du bloc est une
            // incohérence du serveur, pas une donnée à sauver.
            _ => {}
        }
        Ok(())
    }

    /// `content_block_stop` : le bloc est complet.
    fn on_block_stop(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        let Some(index) = trame.index else {
            return;
        };
        let Some(bloc) = self.blocks.remove(&index) else {
            return;
        };
        Self::close_block(index, bloc, out);
    }

    /// Clôt un bloc et émet ce qu'il produit.
    fn close_block(index: u32, bloc: PartialBlock, out: &mut Vec<ChatEvent>) {
        match bloc {
            PartialBlock::Text | PartialBlock::Unknown => {}
            PartialBlock::ToolUse {
                id,
                name,
                arguments,
            } => {
                if name.is_empty() {
                    out.push(ChatEvent::Error(format!(
                        "tool call #{index} has no name and was dropped"
                    )));
                    return;
                }
                match build_tool_call(id, name, &arguments) {
                    Ok(appel) => out.push(ChatEvent::ToolCallComplete(appel)),
                    Err(message) => out.push(ChatEvent::Error(message)),
                }
            }
            PartialBlock::Thinking { text, signature } => {
                out.push(ChatEvent::ReasoningComplete {
                    index,
                    block: ReasoningBlock::Summarized { text, signature },
                });
            }
            PartialBlock::RedactedThinking { data } => {
                out.push(ChatEvent::ReasoningComplete {
                    index,
                    block: ReasoningBlock::Redacted { data },
                });
            }
        }
    }

    /// `message_delta` : la raison d'arrêt et la consommation cumulée.
    fn on_message_delta(&mut self, trame: &Envelope, out: &mut Vec<ChatEvent>) {
        if let Some(raison) = trame.delta.as_ref().and_then(|d| d.stop_reason.as_deref()) {
            // La génération est finie ; le flux, pas encore.
            self.stop = Some(stop_reason(raison));
        }
        if let Some(usage) = trame.usage.as_ref()
            && !usage.is_empty()
        {
            out.push(ChatEvent::Usage {
                prompt_tokens: usage.input(),
                completion_tokens: usage.output(),
                cache_write_tokens: usage.cache_write(),
                cache_read_tokens: usage.cache_read(),
                reasoning_tokens: None,
            });
        }
    }

    /// `message_stop` : fin du flux, annoncée par le serveur.
    ///
    /// Un bloc encore ouvert ici est une incohérence du serveur : il est jeté
    /// comme à la fermeture du flux, et la raison annoncée est conservée.
    fn on_message_stop(&mut self, out: &mut Vec<ChatEvent>) {
        self.discard_open_blocks(out);
        self.emit_done(out);
    }

    /// Une erreur arrivée **dans** le flux, après un statut `200`.
    fn on_error(&mut self, erreur: Option<&WireError>, out: &mut Vec<ChatEvent>) {
        let message = erreur.map_or_else(
            || "no details given".to_owned(),
            super::wire::WireError::describe,
        );
        out.push(ChatEvent::Error(message));
        // Les blocs en cours sont **jetés**, pas clos : après une erreur, un
        // appel d'outil à moitié reçu n'est pas une proposition d'action.
        self.blocks.clear();
        self.stop = Some(StopReason::ProviderError);
        self.emit_done(out);
    }

    /// Arrête la génération sur un budget dépassé.
    ///
    /// L'erreur nomme la limite ; les blocs ouverts sont **jetés** — un appel
    /// d'outil coupé n'est pas une proposition d'action — et la fin est une
    /// coupure : le fournisseur a peut-être continué, et facturé, ce qu'on a
    /// cessé de lire (I-13).
    fn exceed(&mut self, limite: BudgetExceeded, out: &mut Vec<ChatEvent>) {
        out.push(ChatEvent::Error(limite.to_string()));
        self.discard_open_blocks(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Jette les blocs qui n'ont pas reçu leur `content_block_stop`.
    ///
    /// Ils ne sont **pas** clos : un bloc que le serveur n'a pas fermé a été
    /// coupé. Des arguments d'outil qui se trouvent former un JSON valide ne
    /// disent pas que le modèle avait fini de les écrire, et un raisonnement
    /// sans sa signature serait refusé au tour suivant.
    ///
    /// Le texte déjà émis ne peut pas être repris ; un appel d'outil ou un
    /// raisonnement jeté est signalé, pour que sa disparition se voie.
    fn discard_open_blocks(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, bloc) in std::mem::take(&mut self.blocks) {
            match bloc {
                PartialBlock::Text | PartialBlock::Unknown => {}
                PartialBlock::ToolUse { .. } => out.push(ChatEvent::Error(format!(
                    "tool call #{index} was not closed by the provider and was dropped"
                ))),
                PartialBlock::Thinking { .. } | PartialBlock::RedactedThinking { .. } => {
                    out.push(ChatEvent::Error(format!(
                        "reasoning block #{index} was not closed by the provider and was dropped"
                    )));
                }
            }
        }
    }

    /// Émet `Done` une seule et unique fois.
    fn emit_done(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        self.done = true;
        out.push(ChatEvent::Done {
            stop_reason: self.stop.take().unwrap_or(StopReason::Unspecified),
        });
    }
}

impl EventDecoder for MessageDecoder {
    /// Le nom de l'événement SSE est **ignoré** au profit du champ `type` de la
    /// charge : les deux sont redondants dans ce protocole, et la charge est ce
    /// qu'un mandataire a le moins de raisons d'altérer.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
        self.on_data(&frame.data, out);
    }

    fn is_done(&self) -> bool {
        self.finished()
    }

    /// Le serveur a fermé le flux **sans** `message_stop`.
    ///
    /// Sinon le décodeur serait déjà terminé et le pilote ne l'appellerait
    /// pas ; la garde couvre un appel direct. Fermeture propre ou non, c'est une
    /// coupure : voir la note du module.
    fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(
            "the stream ended before the provider announced the end of the message".to_owned(),
        ));
        self.discard_open_blocks(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Les blocs en cours sont **jetés**.
    ///
    /// Des arguments tronqués ne sont pas des arguments, et un raisonnement
    /// sans sa signature serait refusé au tour suivant : proposer l'un ou
    /// l'autre serait pire que de ne rien proposer.
    fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        self.blocks.clear();
        self.stop = Some(StopReason::Cancelled);
        self.emit_done(out);
    }

    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(detail));
        self.blocks.clear();
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }
}

/// Traduit le `stop_reason` du protocole d'Anthropic.
///
/// Vérifié le 2026-09-16, source dans
/// [`RESEARCH-NOTES`](../../../../docs/RESEARCH-NOTES.md) (I-12). Fonction de ce
/// module et non méthode de [`StopReason`] : le type vit dans `oxyn-core`, qui
/// ne connaît aucun protocole. Une valeur inconnue se **conserve** : la
/// documentation annonce que cette liste peut grandir, et rabattre l'inconnu
/// sur `EndTurn` ferait passer une réponse incomplète pour une réponse finie.
pub(crate) fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "stop_sequence" => StopReason::StopSequence,
        "tool_use" => StopReason::ToolCalls,
        "pause_turn" => StopReason::Paused,
        "refusal" => StopReason::Refusal,
        "model_context_window_exceeded" => StopReason::ContextWindowExceeded,
        autre => StopReason::Other(autre.to_owned()),
    }
}

/// Fabrique un identifiant d'appel quand le serveur n'en donne pas.
fn synthetic_id(index: u32) -> String {
    format!("toolu_{index}")
}

/// Reconstruit un appel d'outil complet à partir de ses fragments.
///
/// # Erreurs
/// Rend le message d'erreur d'analyse — **sans** la chaîne d'arguments, qui est
/// une sortie de modèle et peut recopier ce qu'on lui a donné.
fn build_tool_call(id: String, name: String, arguments: &str) -> Result<ToolCall, String> {
    let brut = arguments.trim();
    if brut.is_empty() {
        // Un outil sans paramètre : le bloc s'ouvre avec `input: {}` et aucun
        // delta ne suit.
        return Ok(ToolCall::new(id, name, serde_json::json!({})));
    }
    match serde_json::from_str::<serde_json::Value>(brut) {
        Ok(valeur) => Ok(ToolCall::new(id, name, valeur)),
        Err(err) => Err(format!(
            "cannot read the arguments of tool `{name}`: {} (line {}, column {})",
            classify_json_error(&err),
            err.line(),
            err.column()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_raisons_d_arret_d_anthropic_sont_traduites() {
        for (brut, attendu) in [
            ("end_turn", StopReason::EndTurn),
            ("max_tokens", StopReason::MaxTokens),
            ("stop_sequence", StopReason::StopSequence),
            ("tool_use", StopReason::ToolCalls),
            ("pause_turn", StopReason::Paused),
            ("refusal", StopReason::Refusal),
            (
                "model_context_window_exceeded",
                StopReason::ContextWindowExceeded,
            ),
        ] {
            assert_eq!(stop_reason(brut), attendu, "{brut}");
        }
        assert_eq!(
            stop_reason("raison_future"),
            StopReason::Other("raison_future".to_owned()),
            "la documentation annonce que cette liste peut grandir"
        );
    }

    /// Joue une suite de champs `data` et rend tous les événements produits.
    fn jouer(trames: &[&str]) -> Vec<ChatEvent> {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        for trame in trames {
            decodeur.on_data(trame, &mut sorties);
        }
        if !decodeur.finished() {
            decodeur.finish(&mut sorties);
        }
        sorties
    }

    fn textes(evenements: &[ChatEvent]) -> String {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::TextDelta(t) => Some(t.as_str()),
                _ => None,
            })
            .collect()
    }

    fn raisonnement(evenements: &[ChatEvent]) -> String {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningDelta { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect()
    }

    fn appels(evenements: &[ChatEvent]) -> Vec<&ToolCall> {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCallComplete(appel) => Some(appel),
                _ => None,
            })
            .collect()
    }

    fn blocs(evenements: &[ChatEvent]) -> Vec<&ReasoningBlock> {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ReasoningComplete { block, .. } => Some(block),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn un_flux_de_texte_se_recolle_dans_l_ordre() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":25,"output_tokens":1}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"SELECT "}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"1"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":15}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(textes(&evenements), "SELECT 1");
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        );
    }

    #[test]
    fn un_maintien_de_connexion_ne_produit_rien() {
        let evenements = jouer(&[
            r#"{"type":"ping"}"#,
            r#"{"type":"ping"}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            evenements.len(),
            1,
            "seul `Done` doit rester : {evenements:?}"
        );
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn done_est_emis_exactement_une_fois() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"}}"#,
            &mut sorties,
        );
        decodeur.on_data(r#"{"type":"message_stop"}"#, &mut sorties);
        // Le serveur ferme après `message_stop` : `finish` ne doit rien ajouter.
        decodeur.finish(&mut sorties);
        assert_eq!(sorties.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[test]
    fn la_consommation_de_cache_est_rapportee_dans_les_deux_sens() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":50,"output_tokens":1,"cache_creation_input_tokens":148,"cache_read_input_tokens":2000}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            evenements.contains(&ChatEvent::Usage {
                prompt_tokens: 50,
                completion_tokens: 1,
                cache_write_tokens: Some(148),
                cache_read_tokens: Some(2000),
                reasoning_tokens: None,
            }),
            "{evenements:?}"
        );
    }

    #[test]
    fn une_consommation_non_declaree_ne_s_invente_pas() {
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10,"output_tokens":1}}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Usage {
            cache_write_tokens,
            cache_read_tokens,
            ..
        }) = evenements
            .iter()
            .find(|e| matches!(e, ChatEvent::Usage { .. }))
        else {
            panic!("aucune consommation : {evenements:?}");
        };
        assert_eq!(*cache_write_tokens, None);
        assert_eq!(*cache_read_tokens, None);
    }

    #[test]
    fn un_appel_d_outil_fragmente_se_recolle() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"execute_query","input":{}}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":""}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"SELECT 1\"}"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1, "{evenements:?}");
        assert_eq!(complets[0].id, "toolu_01");
        assert_eq!(complets[0].name, "execute_query");
        assert_eq!(complets[0].arguments["sql"], "SELECT 1");

        assert!(evenements.contains(&ChatEvent::ToolCallStarted {
            index: 1,
            id: "toolu_01".to_owned(),
            name: "execute_query".to_owned(),
        }));
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn deux_blocs_d_outils_ne_se_melangent_pas() {
        // Ce protocole ferme un bloc avant d'en ouvrir un autre, mais rien ne
        // l'y oblige : l'état est tenu par index, pas par ordre d'arrivée.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"a","name":"lire"}}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"b","name":"ecrire"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"t\":1}"}}"#,
            r#"{"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"u\":2}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 2, "{evenements:?}");
        assert_eq!(complets[0].arguments, serde_json::json!({"t": 1}));
        assert_eq!(complets[1].arguments, serde_json::json!({"u": 2}));
    }

    #[test]
    fn un_outil_sans_argument_recoit_un_objet_vide() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"lister","input":{}}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(appels(&evenements)[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn des_arguments_invalides_produisent_une_erreur_et_pas_un_appel() {
        // Le flux à granularité fine n'est pas validé par le serveur : la
        // chaîne accumulée peut ne pas être du JSON.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"execute"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"SELECT secret"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(appels(&evenements).is_empty(), "{evenements:?}");
        let erreur = evenements
            .iter()
            .find_map(|e| match e {
                ChatEvent::Error(m) => Some(m.as_str()),
                _ => None,
            })
            .expect("une erreur doit être signalée");
        assert!(erreur.contains("execute"), "{erreur}");
        assert!(
            !erreur.contains("secret"),
            "la sortie du modèle ne doit pas être recopiée : {erreur}"
        );
    }

    #[test]
    fn un_bloc_de_raisonnement_se_recolle_avec_sa_signature() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"je pose 1071 = 2 × 462 + 147"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":", puis 462 = 3 × 147 + 21"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EqQBCgIYAhIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(
            raisonnement(&evenements),
            "je pose 1071 = 2 × 462 + 147, puis 462 = 3 × 147 + 21"
        );
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "{evenements:?}");
        assert_eq!(
            blocs[0],
            &ReasoningBlock::Summarized {
                text: "je pose 1071 = 2 × 462 + 147, puis 462 = 3 × 147 + 21".to_owned(),
                signature: Some("EqQBCgIYAhIM".to_owned()),
            }
        );
    }

    #[test]
    fn un_raisonnement_masque_reste_transportable() {
        // Le défaut de plusieurs modèles : le bloc arrive sans texte, mais
        // avec sa signature. Il doit quand même être rendu, sans quoi le tour
        // suivant est refusé.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":"","signature":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EosnCkYICxIM"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(raisonnement(&evenements), "", "rien à afficher");
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "le bloc doit exister quand même");
        assert_eq!(blocs[0].display_text(), None);
    }

    #[test]
    fn un_raisonnement_chiffre_ne_produit_aucun_texte() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"redacted_thinking","data":"EvwBCoYBGAIiQL"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);

        assert_eq!(raisonnement(&evenements), "");
        let blocs = blocs(&evenements);
        assert_eq!(blocs.len(), 1, "{evenements:?}");
        assert!(blocs[0].is_redacted());
        assert_eq!(blocs[0].display_text(), None);
    }

    #[test]
    fn une_annulation_au_milieu_d_un_raisonnement_ne_rend_pas_de_bloc_incomplet() {
        // Un bloc sans sa signature serait refusé au tour suivant : le rendre
        // ferait échouer la requête d'après, loin d'ici.
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            &mut sorties,
        );
        decodeur.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"je commence à"}}"#,
            &mut sorties,
        );
        decodeur.cancel(&mut sorties);

        assert!(blocs(&sorties).is_empty(), "{sorties:?}");
        assert_eq!(
            sorties.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn une_annulation_au_milieu_d_un_appel_d_outil_ne_propose_rien() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"c","name":"drop_table"}}"#,
            &mut sorties,
        );
        decodeur.on_data(
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"nom\":"}}"#,
            &mut sorties,
        );
        decodeur.cancel(&mut sorties);

        assert!(
            appels(&sorties).is_empty(),
            "un appel tronqué ne devient jamais une proposition d'action : {sorties:?}"
        );
        assert_eq!(
            sorties.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }

    #[test]
    fn un_refus_se_distingue_d_une_fin_de_tour() {
        let evenements = jouer(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"refusal"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Refusal
            })
        );
    }

    #[test]
    fn une_pause_de_tour_se_signale_comme_incomplete() {
        let evenements = jouer(&[
            r#"{"type":"message_delta","delta":{"stop_reason":"pause_turn"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("{evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Paused);
        assert!(stop_reason.is_truncated());
    }

    #[test]
    fn une_erreur_dans_le_flux_est_terminale() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"error","error":{"type":"overloaded_error","message":"Overloaded"}}"#,
        ]);
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("Overloaded"))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{evenements:?}"
        );
    }

    #[test]
    fn un_flux_ferme_proprement_sans_message_stop_est_interrompu() {
        // Un répartiteur de charge qui coupe à sa limite de durée ferme la
        // connexion proprement : aucune erreur de transport, seulement un flux
        // qui s'arrête. Présenter ce texte comme complet serait mentir.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
        ]);
        assert_eq!(textes(&evenements), "a");
        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("le flux doit se terminer : {evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(stop_reason.is_truncated() && stop_reason.is_ambiguous());
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "la coupure doit se voir : {evenements:?}"
        );
    }

    #[test]
    fn une_raison_annoncee_sans_message_stop_reste_une_coupure() {
        // `message_delta` annonce la fin de la génération, pas celle du
        // message : ce qui suivait n'a pas été reçu.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":3}}"#,
        ]);
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn un_appel_d_outil_lisible_mais_non_clos_n_est_jamais_propose() {
        // Le piège : le JSON reçu jusque-là se trouve être complet. Le modèle
        // n'avait peut-être pas fini — `{"sql":"DELETE FROM t"}` peut être le
        // début de `{"sql":"DELETE FROM t", "where": …}`.
        let evenements = jouer(&[
            r#"{"type":"message_start","message":{"usage":{"input_tokens":10}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\":\"DELETE FROM t\"}"}}"#,
        ]);
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{evenements:?}"
        );
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("tool call #0"))),
            "la disparition de l'appel doit se voir : {evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn un_bloc_non_clos_avant_message_stop_est_jete() {
        // Incohérence du serveur : le message se dit fini, le bloc ne l'est pas.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"execute","input":{}}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{}"}}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"tool_use"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn un_evenement_inconnu_ne_casse_pas_le_flux() {
        // La documentation annonce que de nouveaux types peuvent apparaître.
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            r#"{"type":"un_evenement_futur","charge":{"quoi":"que ce soit"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"a"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "a");
        assert!(
            !evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "un type inconnu n'est pas une erreur : {evenements:?}"
        );
    }

    #[test]
    fn un_bloc_de_type_inconnu_ne_produit_rien_mais_absorbe_ses_deltas() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtoolu_1","name":"web_search"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"query\":\"x\"}"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert!(appels(&evenements).is_empty(), "{evenements:?}");
        assert!(
            !evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
    }

    #[test]
    fn un_delta_sans_bloc_ouvert_est_ignore() {
        let evenements = jouer(&[
            r#"{"type":"content_block_delta","index":7,"delta":{"type":"text_delta","text":"fantome"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "", "{evenements:?}");
    }

    #[test]
    fn une_trame_illisible_isolee_ne_tue_pas_le_flux() {
        let evenements = jouer(&[
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            "{ceci n'est pas du json",
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"b"}}"#,
            r#"{"type":"message_stop"}"#,
        ]);
        assert_eq!(textes(&evenements), "b");
        assert!(evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))));
    }

    #[test]
    fn une_serie_de_trames_illisibles_abandonne_le_flux() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        for _ in 0..MAX_DECODE_ERRORS {
            decodeur.on_data("pas du json", &mut sorties);
        }
        assert!(decodeur.finished(), "{sorties:?}");
        assert!(sorties.last().is_some_and(ChatEvent::is_terminal));
    }

    /// Une trace SSE réelle, telle que la documentation la publie : noms
    /// d'événements, `ping` intercalé, blocs ouverts et fermés.
    const TRACE: &str = concat!(
        "event: message_start\n",
        r#"data: {"type":"message_start","message":{"id":"msg_1","type":"message","role":"assistant","content":[],"model":"claude-modele","stop_reason":null,"usage":{"input_tokens":472,"output_tokens":2}}}"#,
        "\n\n",
        "event: content_block_start\n",
        r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
        "\n\n",
        "event: ping\n",
        r#"data: {"type": "ping"}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Je vérifie le café"}}"#,
        "\n\n",
        "event: content_block_stop\n",
        r#"data: {"type":"content_block_stop","index":0}"#,
        "\n\n",
        "event: content_block_start\n",
        r#"data: {"type":"content_block_start","index":1,"content_block":{"type":"tool_use","id":"toolu_01","name":"get_weather","input":{}}}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":"{\"ville\":"}}"#,
        "\n\n",
        "event: content_block_delta\n",
        r#"data: {"type":"content_block_delta","index":1,"delta":{"type":"input_json_delta","partial_json":" \"Besançon\"}"}}"#,
        "\n\n",
        "event: content_block_stop\n",
        r#"data: {"type":"content_block_stop","index":1}"#,
        "\n\n",
        "event: message_delta\n",
        r#"data: {"type":"message_delta","delta":{"stop_reason":"tool_use","stop_sequence":null},"usage":{"output_tokens":89}}"#,
        "\n\n",
        "event: message_stop\n",
        r#"data: {"type":"message_stop"}"#,
        "\n\n",
    );

    /// Rejoue une trace à travers le vrai chemin SSE, avec un découpage donné.
    fn rejouer(morceaux: Vec<bytes::Bytes>) -> Vec<ChatEvent> {
        use futures::stream::StreamExt as _;

        let octets = morceaux
            .into_iter()
            .map(Ok::<bytes::Bytes, String>)
            .collect::<Vec<_>>();
        let flux = crate::stream::events_stream(
            Box::pin(futures::stream::iter(octets)),
            MessageDecoder::new(),
            oxyn_core::CancelToken::new(),
            None,
        );
        futures::executor::block_on(flux.collect())
    }

    #[test]
    fn une_trace_reelle_se_decode_d_un_bloc() {
        let evenements = rejouer(vec![bytes::Bytes::from_static(TRACE.as_bytes())]);

        assert_eq!(textes(&evenements), "Je vérifie le café");
        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1, "{evenements:?}");
        assert_eq!(complets[0].id, "toolu_01");
        assert_eq!(complets[0].arguments["ville"], "Besançon");
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::ToolCalls
            })
        );
    }

    #[test]
    fn la_meme_trace_decoupee_a_chaque_octet_donne_le_meme_resultat() {
        // Le réseau ne respecte pas les frontières de trame, et la trace
        // contient des caractères multi-octets (« é », « ç ») : le seul
        // découpage qui couvre tous les cas est celui qui n'en respecte aucun.
        let par_octet: Vec<bytes::Bytes> = TRACE
            .as_bytes()
            .iter()
            .map(|octet| bytes::Bytes::copy_from_slice(&[*octet]))
            .collect();

        assert_eq!(
            rejouer(par_octet),
            rejouer(vec![bytes::Bytes::from_static(TRACE.as_bytes())]),
            "le découpage du réseau ne doit rien changer"
        );
    }

    #[test]
    fn un_flux_coupe_au_milieu_d_un_tour_est_tronque_et_ambigu() {
        // Le cas qui coûte de l'argent, et qui ne se voit ni à la compilation
        // ni en revue : la connexion tombe alors que le modèle a commencé à
        // répondre. Le serveur a peut-être terminé — et facturé — le tour.
        // Rejouer paierait deux fois (I-13).
        let coupee: Vec<std::result::Result<bytes::Bytes, String>> = vec![
            Ok(bytes::Bytes::from_static(
                concat!(
                    "event: content_block_start\n",
                    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
                    "\n\n",
                    "event: content_block_delta\n",
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"la table clients "}}"#,
                    "\n\n",
                )
                .as_bytes(),
            )),
            // Ni `message_delta`, ni `message_stop` : le transport lâche.
            Err("lost the connection while receiving the stream".to_owned()),
        ];

        let evenements = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(coupee)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        // Ce qui a été reçu reste montré : l'utilisateur voit où ça s'est
        // arrêté plutôt que de perdre le début.
        assert_eq!(textes(&evenements), "la table clients ");

        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("le flux doit se terminer : {evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(
            stop_reason.is_truncated(),
            "une réponse tranchée en plein milieu qui se dit complète est une réponse fausse"
        );
        assert!(
            stop_reason.is_ambiguous(),
            "le serveur a peut-être produit la suite : rejouer paierait deux fois (I-13)"
        );
        assert_eq!(
            evenements.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{evenements:?}"
        );
    }

    #[test]
    fn un_appel_d_outil_coupe_par_le_transport_n_est_jamais_propose() {
        // La coupure tombe au milieu des arguments. Les proposer reviendrait à
        // soumettre une action dont personne ne connaît la portée.
        let coupee: Vec<std::result::Result<bytes::Bytes, String>> = vec![
            Ok(bytes::Bytes::from_static(
                concat!(
                    r#"data: {"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_9","name":"execute_query"}}"#,
                    "\n\n",
                    r#"data: {"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":"{\"sql\": \"DELETE FROM audit"}}"#,
                    "\n\n",
                )
                .as_bytes(),
            )),
            Err("lost the connection while receiving the stream".to_owned()),
        ];

        let evenements = {
            use futures::stream::StreamExt as _;
            let flux = crate::stream::events_stream(
                Box::pin(futures::stream::iter(coupee)),
                MessageDecoder::new(),
                oxyn_core::CancelToken::new(),
                None,
            );
            futures::executor::block_on(flux.collect::<Vec<_>>())
        };

        assert!(
            appels(&evenements).is_empty(),
            "des arguments tronqués ne sont pas des arguments : {evenements:?}"
        );
        // Et le SQL partiel ne doit pas ressortir dans un message d'erreur : ce
        // qu'un modèle écrit peut recopier ce qu'on lui a donné.
        for evenement in &evenements {
            if let ChatEvent::Error(message) = evenement {
                assert!(!message.contains("DELETE FROM"), "{message}");
            }
        }
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn rien_n_est_decode_apres_la_fin() {
        let mut decodeur = MessageDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(r#"{"type":"message_stop"}"#, &mut sorties);
        let apres = sorties.len();
        decodeur.on_data(
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text"}}"#,
            &mut sorties,
        );
        assert_eq!(sorties.len(), apres, "{sorties:?}");
    }
}

#[cfg(test)]
mod budget_tests;
