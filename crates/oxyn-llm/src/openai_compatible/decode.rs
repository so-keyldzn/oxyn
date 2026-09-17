//! De la trame SSE aux événements du domaine.
//!
//! Ce module tient l'état d'un flux de complétion. Il est **pur** : aucune
//! entrée-sortie, aucun réseau. C'est ce qui permet de le tester sur les
//! séquences réelles des serveurs, y compris leurs bizarreries, sans en lancer
//! un.
//!
//! # Le recollement des appels d'outils
//!
//! Un appel d'outil n'arrive pas d'un bloc : le nom vient dans une trame, les
//! arguments en une dizaine de fragments, et les fragments de plusieurs appels
//! sont **entrelacés**. La seule chose qui les relie est le champ `index`. Un
//! décodeur qui concatènerait dans l'ordre d'arrivée produirait un JSON mêlant
//! deux appels — et le modèle, lui, aurait demandé deux actions distinctes.
//!
//! # Quand `Done` est émis
//!
//! `finish_reason` marque la fin de la génération, mais **pas** la fin du
//! flux : OpenAI envoie la consommation dans une trame ultérieure. On mémorise
//! donc la raison, on clôt les appels d'outils, et on n'émet `Done` qu'à la
//! sentinelle `[DONE]` ou à la fermeture du flux. `Done` est émis exactement une
//! fois.
//!
//! # Une fin constatée n'est pas une fin annoncée
//!
//! Ce protocole a **deux** annonces de fin, et elles ne disent pas la même
//! chose : `finish_reason` sur le dernier fragment de contenu dit que la
//! génération est finie ; `data: [DONE]` dit que le flux l'est. OpenAI
//! documente les deux ([RESEARCH-NOTES](../../../../docs/RESEARCH-NOTES.md)),
//! mais un serveur compatible peut omettre la sentinelle.
//!
//! La règle retenue ne dépend donc pas de la sentinelle :
//!
//! * **`finish_reason` reçu, puis fermeture** : la génération a été annoncée
//!   finie, le contenu est entier ; seule la trame de consommation a pu se
//!   perdre. La raison annoncée est conservée ;
//! * **ni `finish_reason` ni `[DONE]`, puis fermeture** — propre ou non : c'est
//!   une coupure, [`StopReason::Interrupted`], et les appels d'outils en cours
//!   sont **jetés**. Un mandataire qui ferme proprement en pleine génération ne
//!   produit aucune erreur de transport, et des arguments qui se lisent par
//!   chance ne sont pas des arguments que le modèle a fini d'écrire.

use std::collections::BTreeMap;

use super::wire::{self, ChatChunk, DeltaToolCall};
use crate::sse::SseFrame;
use crate::stream::EventDecoder;
use crate::types::{ChatEvent, StopReason};

/// Sentinelle de fin des protocoles compatibles OpenAI.
pub(crate) const DONE_SENTINEL: &str = "[DONE]";

/// Nombre de trames illisibles tolérées avant d'abandonner le flux.
///
/// Une trame illisible isolée arrive (une passerelle qui insère un objet non
/// documenté) ; une série signifie qu'on ne parle pas le même protocole, et
/// continuer ne ferait qu'inonder l'interface d'erreurs.
pub(crate) const MAX_DECODE_ERRORS: usize = 8;

/// Un appel d'outil en cours de reconstruction.
#[derive(Debug, Default)]
struct PartialCall {
    id: Option<String>,
    name: String,
    arguments: String,
    started: bool,
}

/// État d'un flux de complétion compatible OpenAI.
#[derive(Debug, Default)]
pub(crate) struct ChunkDecoder {
    calls: BTreeMap<u32, PartialCall>,
    stop: Option<StopReason>,
    done: bool,
    errors: usize,
}

impl ChunkDecoder {
    /// Décodeur pour un flux neuf.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// Le flux est-il terminé ? Plus rien ne doit être décodé après.
    pub(crate) fn is_done(&self) -> bool {
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
        if data == DONE_SENTINEL {
            self.flush_calls(out);
            self.emit_done(out);
            return;
        }

        let chunk: ChatChunk = match serde_json::from_str(data) {
            Ok(chunk) => chunk,
            Err(err) => {
                // La trame fautive n'est pas recopiée : on ne sait pas ce qu'un
                // point d'accès tiers y met.
                self.errors += 1;
                out.push(ChatEvent::Error(format!(
                    "unreadable stream frame ({}, line {}, column {})",
                    wire::classify_label(&err),
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

        // Une erreur transportée dans le flux est terminale : la passerelle ne
        // produira plus rien après.
        if let Some(erreur) = &chunk.error {
            out.push(ChatEvent::Error(erreur.describe()));
            self.stop = Some(StopReason::ProviderError);
            // Jetés, pas clos : un appel interrompu par une erreur n'est pas une
            // proposition d'action.
            self.discard_calls(out);
            self.emit_done(out);
            return;
        }

        if let Some(usage) = &chunk.usage {
            out.push(ChatEvent::Usage {
                prompt_tokens: usage.prompt(),
                completion_tokens: usage.completion(),
                // Ce protocole ne distingue pas l'écriture de cache : il ne
                // rapporte que les jetons **lus**. Déclarer `Some(0)` en
                // écriture laisserait croire qu'aucun préfixe n'a été mis en
                // cache, alors qu'on n'en sait rien.
                cache_write_tokens: None,
                cache_read_tokens: usage.cache_read(),
                reasoning_tokens: usage.reasoning(),
            });
        }

        for choix in chunk.choices {
            if let Some(texte) = choix.delta.content
                && !texte.is_empty()
            {
                out.push(ChatEvent::TextDelta(texte));
            }
            // Un refus est une réponse, pas une erreur : la requête a abouti et
            // le modèle a dit qu'il ne répondrait pas. Le confondre avec du
            // texte le ferait présenter comme une réponse.
            if let Some(refus) = choix.delta.refusal
                && !refus.is_empty()
            {
                out.push(ChatEvent::RefusalDelta(refus));
            }
            for appel in choix.delta.tool_calls {
                self.accumulate(appel, out);
            }
            if let Some(raison) = choix.finish_reason {
                // La génération est finie ; le flux, pas forcément.
                self.stop = Some(stop_reason(&raison));
                self.flush_calls(out);
            }
        }
    }

    /// Signale la fermeture du flux par le serveur, sans `[DONE]`.
    ///
    /// Voir la note du module : seul un `finish_reason` déjà reçu fait de cette
    /// fermeture une fin ordinaire.
    pub(crate) fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        if self.stop.is_some() {
            // Les appels ont été clos au `finish_reason` ; un fragment arrivé
            // après n'a pas d'annonce de fin.
            self.discard_calls(out);
            self.emit_done(out);
            return;
        }
        out.push(ChatEvent::Error(
            "the stream ended before the provider announced the end of the generation".to_owned(),
        ));
        self.discard_calls(out);
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
    }

    /// Signale une annulation demandée par l'appelant.
    ///
    /// Les appels d'outils en cours ne sont **pas** clos : des arguments
    /// tronqués ne sont pas des arguments, et proposer une action à moitié
    /// décrite serait pire que ne rien proposer.
    pub(crate) fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        self.calls.clear();
        self.stop = Some(StopReason::Cancelled);
        self.emit_done(out);
    }

    /// Signale une rupture de transport en cours de flux.
    pub(crate) fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        if self.done {
            return;
        }
        out.push(ChatEvent::Error(detail));
        self.calls.clear();
        self.stop = Some(StopReason::Interrupted);
        self.emit_done(out);
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

    /// Range un fragment d'appel d'outil et émet ce qui est devenu certain.
    fn accumulate(&mut self, brut: DeltaToolCall, out: &mut Vec<ChatEvent>) {
        let index = brut.index;
        let entree = self.calls.entry(index).or_default();

        if let Some(id) = brut.id.filter(|valeur| !valeur.is_empty())
            && entree.id.is_none()
        {
            entree.id = Some(id);
        }

        let mut fragment = None;
        if let Some(fonction) = brut.function {
            if let Some(nom) = fonction.name.filter(|valeur| !valeur.is_empty()) {
                // Concaténation et non affectation : quelques serveurs
                // fragmentent aussi le nom.
                entree.name.push_str(&nom);
            }
            if let Some(arguments) = fonction.arguments.filter(|valeur| !valeur.is_empty()) {
                entree.arguments.push_str(&arguments);
                fragment = Some(arguments);
            }
        }

        if !entree.started && !entree.name.is_empty() {
            let id = entree.id.clone().unwrap_or_else(|| synthetic_id(index));
            entree.id = Some(id.clone());
            entree.started = true;
            let name = entree.name.clone();
            out.push(ChatEvent::ToolCallStarted { index, id, name });
        }

        if let Some(arguments) = fragment {
            out.push(ChatEvent::ToolCallDelta { index, arguments });
        }
    }

    /// Jette les appels d'outils sans annonce de fin, en le signalant.
    ///
    /// Un appel qui disparaît en silence laisserait l'utilisateur croire que le
    /// modèle n'a rien demandé.
    fn discard_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, partiel) in std::mem::take(&mut self.calls) {
            if !partiel.name.is_empty() || !partiel.arguments.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} was not finished by the provider and was dropped"
                )));
            }
        }
    }

    /// Clôt tous les appels d'outils rassemblés, sur une annonce de fin.
    fn flush_calls(&mut self, out: &mut Vec<ChatEvent>) {
        for (index, partiel) in std::mem::take(&mut self.calls) {
            if partiel.name.is_empty() {
                out.push(ChatEvent::Error(format!(
                    "tool call #{index} has no name and was dropped"
                )));
                continue;
            }
            let id = partiel.id.unwrap_or_else(|| synthetic_id(index));
            if !partiel.started {
                out.push(ChatEvent::ToolCallStarted {
                    index,
                    id: id.clone(),
                    name: partiel.name.clone(),
                });
            }
            match wire::build_tool_call(id, partiel.name, &partiel.arguments) {
                Ok(appel) => out.push(ChatEvent::ToolCallComplete(appel)),
                Err(message) => out.push(ChatEvent::Error(message)),
            }
        }
    }
}

impl EventDecoder for ChunkDecoder {
    /// Ce protocole ne nomme pas ses trames : seul le champ `data` compte, et
    /// un éventuel champ `event` est ignoré plutôt que d'inventer un sens.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
        self.on_data(&frame.data, out);
    }

    fn is_done(&self) -> bool {
        Self::is_done(self)
    }

    fn finish(&mut self, out: &mut Vec<ChatEvent>) {
        Self::finish(self, out);
    }

    fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
        Self::cancel(self, out);
    }

    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
        Self::transport_error(self, detail, out);
    }
}

/// Traduit le `finish_reason` des protocoles compatibles OpenAI.
///
/// Fonction de ce module et non méthode de [`StopReason`] : le type vit dans
/// `oxyn-core`, qui ne connaît aucun protocole. Une raison inconnue se
/// **conserve** plutôt que d'être rabattue sur `EndTurn`, qui ferait passer une
/// réponse incomplète pour une réponse finie.
pub(crate) fn stop_reason(raw: &str) -> StopReason {
    match raw {
        "stop" => StopReason::EndTurn,
        "length" => StopReason::MaxTokens,
        "tool_calls" | "function_call" => StopReason::ToolCalls,
        "content_filter" => StopReason::ContentFilter,
        autre => StopReason::Other(autre.to_owned()),
    }
}

/// Fabrique un identifiant d'appel quand le serveur n'en donne pas.
///
/// Ollama et `llama.cpp` omettent régulièrement `id`. Sans identifiant, le tour
/// suivant ne peut pas rattacher le résultat de l'outil à sa demande : le
/// fabriquer à partir de l'index est le seul recours, et il est stable pour un
/// tour donné.
fn synthetic_id(index: u32) -> String {
    format!("call_{index}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn les_raisons_d_arret_d_openai_sont_traduites() {
        assert_eq!(stop_reason("stop"), StopReason::EndTurn);
        assert_eq!(stop_reason("length"), StopReason::MaxTokens);
        assert_eq!(stop_reason("tool_calls"), StopReason::ToolCalls);
        assert_eq!(stop_reason("function_call"), StopReason::ToolCalls);
        assert_eq!(stop_reason("content_filter"), StopReason::ContentFilter);
        assert_eq!(
            stop_reason("guardrail_intervened"),
            StopReason::Other("guardrail_intervened".to_owned()),
            "une raison inconnue se conserve, elle ne se rabat pas sur EndTurn"
        );
    }

    /// Joue une suite de champs `data` et rend tous les événements produits.
    fn jouer(trames: &[&str]) -> Vec<ChatEvent> {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        for trame in trames {
            decodeur.on_data(trame, &mut sorties);
        }
        if !decodeur.is_done() {
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

    fn appels(evenements: &[ChatEvent]) -> Vec<&crate::types::ToolCall> {
        evenements
            .iter()
            .filter_map(|e| match e {
                ChatEvent::ToolCallComplete(appel) => Some(appel),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn un_flux_de_texte_se_recolle_dans_l_ordre() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"role":"assistant","content":""}}]}"#,
            r#"{"choices":[{"delta":{"content":"SELECT "}}]}"#,
            r#"{"choices":[{"delta":{"content":"1"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            DONE_SENTINEL,
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
    fn done_est_emis_exactement_une_fois() {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            &mut sorties,
        );
        decodeur.on_data(DONE_SENTINEL, &mut sorties);
        // Le serveur ferme après la sentinelle : `finish` ne doit rien ajouter.
        decodeur.finish(&mut sorties);
        let fins = sorties.iter().filter(|e| e.is_terminal()).count();
        assert_eq!(fins, 1, "{sorties:?}");
    }

    #[test]
    fn la_consommation_arrivant_apres_la_fin_de_generation_est_conservee() {
        // Le piège : émettre `Done` sur `finish_reason` perdrait cette trame.
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"content":"ok"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
            r#"{"choices":[],"usage":{"prompt_tokens":120,"completion_tokens":8}}"#,
            DONE_SENTINEL,
        ]);
        assert!(
            evenements.contains(&ChatEvent::Usage {
                prompt_tokens: 120,
                completion_tokens: 8,
                cache_write_tokens: None,
                cache_read_tokens: None,
                reasoning_tokens: None,
            }),
            "{evenements:?}"
        );
        assert!(
            evenements.last().is_some_and(ChatEvent::is_terminal),
            "Done doit rester le dernier événement"
        );
    }

    #[test]
    fn un_appel_d_outil_fragmente_se_recolle() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_abc","type":"function","function":{"name":"execute_query","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"\"SELECT 1\"}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
            DONE_SENTINEL,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1, "{evenements:?}");
        assert_eq!(complets[0].id, "call_abc");
        assert_eq!(complets[0].name, "execute_query");
        assert_eq!(complets[0].arguments["sql"], "SELECT 1");

        assert!(evenements.contains(&ChatEvent::ToolCallStarted {
            index: 0,
            id: "call_abc".to_owned(),
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
    fn deux_appels_entrelaces_ne_se_melangent_pas() {
        // Le défaut que ce module existe pour éviter : concaténer dans l'ordre
        // d'arrivée produirait un seul JSON incohérent.
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"lire"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"id":"b","function":{"name":"ecrire"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"t\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"{\"u\":"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"1}"}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":1,"function":{"arguments":"2}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);

        let complets = appels(&evenements);
        assert_eq!(complets.len(), 2, "{evenements:?}");
        assert_eq!(complets[0].name, "lire");
        assert_eq!(complets[0].arguments, serde_json::json!({"t": 1}));
        assert_eq!(complets[1].name, "ecrire");
        assert_eq!(complets[1].arguments, serde_json::json!({"u": 2}));
    }

    #[test]
    fn un_appel_sans_identifiant_en_recoit_un() {
        // Ollama et llama.cpp omettent `id`.
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"name":"ping","arguments":"{}"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        let complets = appels(&evenements);
        assert_eq!(complets.len(), 1);
        assert!(
            !complets[0].id.is_empty(),
            "sans identifiant, le résultat de l'outil ne peut pas être rattaché"
        );
    }

    #[test]
    fn un_outil_sans_argument_recoit_un_objet_vide() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"lister"}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert_eq!(appels(&evenements)[0].arguments, serde_json::json!({}));
    }

    #[test]
    fn des_arguments_invalides_produisent_une_erreur_et_pas_un_appel() {
        // Un modèle produit parfois du JSON cassé. Émettre un appel avec des
        // arguments nuls serait un mensonge silencieux.
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"execute","arguments":"{\"sql\": "}}]}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"tool_calls"}]}"#,
        ]);
        assert!(appels(&evenements).is_empty(), "{evenements:?}");
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("execute"))),
            "{evenements:?}"
        );
    }

    #[test]
    fn un_flux_ferme_apres_finish_reason_sans_sentinelle_est_complet() {
        // La génération a été annoncée finie : l'absence de `[DONE]` ne coûte
        // que la trame de consommation.
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"choices":[{"delta":{},"finish_reason":"stop"}]}"#,
        ]);
        assert_eq!(textes(&evenements), "a");
        assert!(
            !evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        );
    }

    #[test]
    fn un_flux_ferme_proprement_sans_aucune_annonce_est_interrompu() {
        // Ni `finish_reason` ni `[DONE]` : un mandataire a fermé en pleine
        // génération. Rien n'a échoué côté transport, et pourtant la réponse
        // est coupée.
        let evenements = jouer(&[r#"{"choices":[{"delta":{"content":"a"}}]}"#]);
        let Some(ChatEvent::Done { stop_reason }) = evenements.last() else {
            panic!("le flux doit se terminer : {evenements:?}");
        };
        assert_eq!(*stop_reason, StopReason::Interrupted);
        assert!(stop_reason.is_ambiguous());
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "la coupure doit se voir : {evenements:?}"
        );
    }

    #[test]
    fn un_appel_d_outil_lisible_mais_sans_annonce_de_fin_n_est_jamais_propose() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":""}}]}}]}"#,
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"function":{"arguments":"{\"sql\":\"DELETE FROM t\"}"}}]}}]}"#,
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
            "{evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Interrupted
            })
        );
    }

    #[test]
    fn une_erreur_dans_le_flux_jette_les_appels_en_cours() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"call_1","function":{"name":"execute","arguments":"{}"}}]}}]}"#,
            r#"{"error":{"message":"overloaded"}}"#,
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
                stop_reason: StopReason::ProviderError
            })
        );
    }

    #[test]
    fn une_trame_illisible_isolee_ne_tue_pas_le_flux() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            "{ceci n'est pas du json",
            r#"{"choices":[{"delta":{"content":"b"}}]}"#,
            DONE_SENTINEL,
        ]);
        assert_eq!(textes(&evenements), "ab");
        assert!(evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))));
    }

    #[test]
    fn une_serie_de_trames_illisibles_abandonne_le_flux() {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        for _ in 0..MAX_DECODE_ERRORS {
            decodeur.on_data("pas du json", &mut sorties);
        }
        assert!(decodeur.is_done(), "{sorties:?}");
        assert!(sorties.last().is_some_and(ChatEvent::is_terminal));
    }

    #[test]
    fn une_erreur_transportee_dans_le_flux_est_terminale() {
        let evenements = jouer(&[
            r#"{"choices":[{"delta":{"content":"a"}}]}"#,
            r#"{"error":{"message":"upstream refused","code":502}}"#,
        ]);
        assert!(
            evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::Error(m) if m.contains("upstream refused"))),
            "{evenements:?}"
        );
        assert_eq!(
            evenements.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{evenements:?}"
        );
    }

    #[test]
    fn une_annulation_n_invente_pas_d_appel_a_moitie_decrit() {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(
            r#"{"choices":[{"delta":{"tool_calls":[{"index":0,"id":"c","function":{"name":"drop_table","arguments":"{\"nom\":"}}]}}]}"#,
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
    fn une_rupture_de_transport_termine_le_flux() {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(r#"{"choices":[{"delta":{"content":"a"}}]}"#, &mut sorties);
        decodeur.transport_error("connexion réinitialisée".to_owned(), &mut sorties);
        assert!(decodeur.is_done());
        assert!(sorties.last().is_some_and(ChatEvent::is_terminal));
    }

    #[test]
    fn rien_n_est_decode_apres_la_fin() {
        let mut decodeur = ChunkDecoder::new();
        let mut sorties = Vec::new();
        decodeur.on_data(DONE_SENTINEL, &mut sorties);
        let apres = sorties.len();
        decodeur.on_data(
            r#"{"choices":[{"delta":{"content":"fantome"}}]}"#,
            &mut sorties,
        );
        assert_eq!(sorties.len(), apres, "{sorties:?}");
    }
}
