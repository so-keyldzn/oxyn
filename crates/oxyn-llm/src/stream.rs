//! Du flux d'octets HTTP au flux d'événements, pour **tous** les protocoles.
//!
//! Ce module ne parle ni de fournisseur, ni d'authentification, ni de format de
//! trame : il assemble le décodeur SSE ([`crate::sse`]) et un décodeur de
//! protocole ([`EventDecoder`]) en un [`Stream`] annulable.
//!
//! # Pourquoi un seul pilote pour deux protocoles
//!
//! Les garanties ci-dessous sont exactement ce qui se rate à la deuxième
//! écriture : un `Done` émis deux fois, un `Done` jamais émis sur une rupture,
//! une annulation qui n'interrompt rien. Les tenir à un seul endroit est ce qui
//! rend l'invariant relisible ; deux pilotes en parallèle divergeraient en un
//! ajout de variante.
//!
//! # Ce que le flux garantit
//!
//! * **`Done` est émis exactement une fois, en dernier.** Fermeture propre,
//!   fermeture brutale, rupture de transport, annulation, tampon dépassé : les
//!   cinq sorties passent par la même émission.
//! * **L'annulation interrompt la lecture.** Le jeton est interrogé avant
//!   chaque attente *et* concurremment de celle-ci : un fournisseur qui ne
//!   répond plus ne laisse pas l'utilisateur devant un bouton sans effet.
//! * **Rien n'est repris après une interruption.** Un décodeur SSE dont on a
//!   perdu des octets est désynchronisé ; une génération se relance, elle ne se
//!   reprend pas.

use std::collections::VecDeque;
use std::pin::{Pin, pin};

use bytes::Bytes;
use futures::future::{Either, select};
use futures::stream::{BoxStream, Stream, StreamExt};
use oxyn_core::CancelToken;

use crate::sse::{SseDecoder, SseFrame};
use crate::types::ChatEvent;

/// Décrit une rupture de flux **sans** reprendre le message brut.
///
/// Le message d'une erreur de transport peut contenir l'URL, donc les
/// identifiants qu'elle porterait. On classe plutôt que de recopier.
pub(crate) fn describe_stream_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "timed out while receiving the stream".to_owned()
    } else if err.is_body() || err.is_decode() {
        "the provider interrupted the stream".to_owned()
    } else {
        "lost the connection while receiving the stream".to_owned()
    }
}

/// Flux d'octets déjà classé, tel que le décodeur le consomme.
pub(crate) type ByteStream = Pin<Box<dyn Stream<Item = std::result::Result<Bytes, String>> + Send>>;

/// Ce qu'un décodeur de protocole doit savoir faire pour être piloté ici.
///
/// Les méthodes de terminaison sont distinctes parce que les décisions le sont.
///
/// **Le pilote ne sait pas ce qu'est une fin annoncée** : c'est une trame du
/// protocole (`message_stop`, `finish_reason`, `[DONE]`), et seul le décodeur
/// la reconnaît. Le pilote ne rapporte que ce qu'il constate — le serveur a
/// fermé, le transport a rompu, l'appelant a annulé. Un décodeur qui a vu son
/// annonce de fin se déclare terminé par [`is_done`](Self::is_done), et le
/// pilote ne l'appelle plus.
pub(crate) trait EventDecoder: Send {
    /// Consomme une trame SSE complète.
    fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>);

    /// Le décodeur a-t-il déjà émis sa fin ? Plus rien ne doit lui être donné.
    fn is_done(&self) -> bool;

    /// Le serveur a fermé le flux **sans** que le décodeur se soit déclaré
    /// terminé.
    ///
    /// Une fermeture propre n'est pas une fin annoncée : un mandataire qui
    /// coupe à sa limite de durée ferme proprement. Sauf si le protocole a
    /// annoncé la fin de la génération, le décodeur répond
    /// [`StopReason::Interrupted`](crate::types::StopReason::Interrupted) et
    /// jette ce qu'il n'a pas vu se clore.
    fn finish(&mut self, out: &mut Vec<ChatEvent>);

    /// L'appelant a annulé.
    fn cancel(&mut self, out: &mut Vec<ChatEvent>);

    /// Le transport a rompu en cours de flux.
    fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>);
}

/// État porté d'un pas de décodage à l'autre.
struct StreamState<D> {
    bytes: ByteStream,
    sse: SseDecoder,
    decoder: D,
    pending: VecDeque<ChatEvent>,
    cancel: CancelToken,
    finished: bool,
}

/// Issue d'une attente : un morceau, une fin, ou une annulation.
enum Step {
    Cancelled,
    Chunk(Option<std::result::Result<Bytes, String>>),
}

/// Transforme un flux d'octets SSE en flux d'événements du domaine.
///
/// Le flux rendu est `'static` et `Send` : il se transmet à une tâche. Il émet
/// exactement un [`ChatEvent::Done`], en dernier, y compris en cas
/// d'annulation, d'erreur de transport ou de fermeture brutale.
pub(crate) fn events_stream<D: EventDecoder + 'static>(
    bytes: ByteStream,
    decoder: D,
    cancel: CancelToken,
) -> BoxStream<'static, ChatEvent> {
    let etat = StreamState {
        bytes,
        sse: SseDecoder::new(),
        decoder,
        pending: VecDeque::new(),
        cancel,
        finished: false,
    };

    futures::stream::unfold(etat, |mut etat| async move {
        loop {
            if let Some(evenement) = etat.pending.pop_front() {
                return Some((evenement, etat));
            }
            if etat.finished {
                return None;
            }

            let mut sorties = Vec::new();

            // Vérification avant l'attente : un jeton déjà annulé ne doit pas
            // faire lire un morceau de plus.
            if etat.cancel.is_cancelled() {
                etat.decoder.cancel(&mut sorties);
                etat.finished = true;
                etat.pending.extend(sorties);
                continue;
            }

            let pas = {
                let attente = pin!(etat.cancel.cancelled());
                let suivant = pin!(etat.bytes.next());
                match select(attente, suivant).await {
                    Either::Left(((), _)) => Step::Cancelled,
                    Either::Right((morceau, _)) => Step::Chunk(morceau),
                }
            };

            match pas {
                Step::Cancelled => {
                    // Le futur de lecture est abandonné ici. Aucune reprise
                    // n'est tentée : un décodeur SSE dont on a perdu des octets
                    // est désynchronisé, et une génération se relance, elle ne
                    // se reprend pas.
                    etat.decoder.cancel(&mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(None) => {
                    etat.sse.finish();
                    drain(&mut etat, &mut sorties);
                    etat.decoder.finish(&mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(Some(Err(detail))) => {
                    etat.decoder.transport_error(detail, &mut sorties);
                    etat.finished = true;
                }
                Step::Chunk(Some(Ok(morceau))) => {
                    // L'accumulation est sortie du `match` : un emprunt pris
                    // dans l'expression jugée y resterait vivant pendant les
                    // bras, qui réempruntent l'état.
                    let accumulation = etat.sse.push(&morceau);
                    match accumulation {
                        Ok(()) => {
                            drain(&mut etat, &mut sorties);
                            if etat.decoder.is_done() {
                                etat.finished = true;
                            }
                        }
                        Err(depassement) => {
                            etat.decoder
                                .transport_error(depassement.to_string(), &mut sorties);
                            etat.finished = true;
                        }
                    }
                }
            }

            etat.pending.extend(sorties);
        }
    })
    .boxed()
}

/// Vide le décodeur SSE dans le décodeur de protocole.
fn drain<D: EventDecoder>(etat: &mut StreamState<D>, sorties: &mut Vec<ChatEvent>) {
    loop {
        // `let … else` plutôt que `while let` : l'emprunt du décodeur SSE se
        // termine à la fin de l'instruction, avant que le décodeur de protocole
        // ne soit emprunté à son tour.
        let Some(trame) = etat.sse.next_frame() else {
            return;
        };
        etat.decoder.on_frame(&trame, sorties);
        if etat.decoder.is_done() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;

    /// Décodeur d'essai : chaque trame `data:` devient un fragment de texte.
    ///
    /// Il ne connaît aucun protocole réel — c'est le pilote qu'on éprouve ici,
    /// et les décodeurs réels sont éprouvés dans leur propre module.
    #[derive(Default)]
    struct Echo {
        done: bool,
        stop: Option<StopReason>,
    }

    impl Echo {
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

    impl EventDecoder for Echo {
        fn on_frame(&mut self, frame: &SseFrame, out: &mut Vec<ChatEvent>) {
            if self.done {
                return;
            }
            if frame.data == "fin" {
                self.stop = Some(StopReason::EndTurn);
                self.emit_done(out);
                return;
            }
            out.push(ChatEvent::TextDelta(frame.data.clone()));
        }

        fn is_done(&self) -> bool {
            self.done
        }

        fn finish(&mut self, out: &mut Vec<ChatEvent>) {
            self.emit_done(out);
        }

        fn cancel(&mut self, out: &mut Vec<ChatEvent>) {
            self.stop = Some(StopReason::Cancelled);
            self.emit_done(out);
        }

        fn transport_error(&mut self, detail: String, out: &mut Vec<ChatEvent>) {
            if self.done {
                return;
            }
            out.push(ChatEvent::Error(detail));
            self.stop = Some(StopReason::Interrupted);
            self.emit_done(out);
        }
    }

    fn morceaux(parts: &[&'static str]) -> ByteStream {
        let items: Vec<std::result::Result<Bytes, String>> = parts
            .iter()
            .map(|p| Ok(Bytes::from_static(p.as_bytes())))
            .collect();
        Box::pin(futures::stream::iter(items))
    }

    fn collecter(flux: BoxStream<'static, ChatEvent>) -> Vec<ChatEvent> {
        futures::executor::block_on(flux.collect())
    }

    fn jouer(parts: &[&'static str], jeton: CancelToken) -> Vec<ChatEvent> {
        collecter(events_stream(morceaux(parts), Echo::default(), jeton))
    }

    #[test]
    fn un_flux_complet_devient_des_evenements() {
        let evenements = jouer(
            &["data: a\n\n", "data: b\n\n", "data: fin\n\n"],
            CancelToken::new(),
        );
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("a".to_owned()),
                ChatEvent::TextDelta("b".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    /// Le pilote garantit la présence de `Done` ; la raison est l'affaire du
    /// décodeur, éprouvée dans chaque module de protocole.
    #[test]
    fn un_flux_ferme_sans_marqueur_de_fin_se_termine_quand_meme() {
        let evenements = jouer(&["data: a\n\n"], CancelToken::new());
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Unspecified
            })
        );
    }

    #[test]
    fn un_flux_vide_produit_tout_de_meme_une_fin() {
        let evenements = jouer(&[], CancelToken::new());
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn done_n_est_emis_qu_une_fois_meme_avec_des_trames_apres() {
        let evenements = jouer(&["data: fin\n\n", "data: fantome\n\n"], CancelToken::new());
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn un_jeton_deja_annule_ne_lit_aucun_morceau() {
        let jeton = CancelToken::new();
        jeton.cancel();
        assert_eq!(
            jouer(&["data: a\n\n"], jeton),
            vec![ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            }]
        );
    }

    #[test]
    fn une_annulation_en_cours_de_flux_arrete_la_lecture() {
        let jeton = CancelToken::new();
        let declencheur = jeton.clone();
        let octets = futures::stream::iter(vec!["data: a\n\n", "data: b\n\n"]).map(
            move |morceau| -> std::result::Result<Bytes, String> {
                declencheur.cancel();
                Ok(Bytes::from_static(morceau.as_bytes()))
            },
        );

        let evenements = collecter(events_stream(Box::pin(octets), Echo::default(), jeton));
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("a".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::Cancelled
                },
            ],
            "le second morceau ne doit jamais être lu"
        );
    }

    #[test]
    fn une_rupture_de_transport_termine_proprement() {
        let octets = futures::stream::iter(vec![
            Ok(Bytes::from_static(b"data: a\n\n")),
            Err("lost the connection while receiving the stream".to_owned()),
        ]);
        let evenements = collecter(events_stream(
            Box::pin(octets),
            Echo::default(),
            CancelToken::new(),
        ));
        assert_eq!(
            evenements.first(),
            Some(&ChatEvent::TextDelta("a".to_owned()))
        );
        assert!(
            evenements.last().is_some_and(ChatEvent::is_terminal),
            "{evenements:?}"
        );
    }

    #[test]
    fn un_flux_sans_fin_de_ligne_est_borne_et_se_termine() {
        // Un serveur défaillant qui n'envoie jamais de fin de ligne ne doit pas
        // faire gonfler la mémoire sans limite : le décodeur borne, et le flux
        // se termine sur une erreur suivie de `Done`.
        let deluge: Vec<std::result::Result<Bytes, String>> = vec![Ok(Bytes::from(vec![
            b'x';
            crate::sse::DEFAULT_BUFFER_LIMIT
                + 1
        ]))];
        let evenements = collecter(events_stream(
            Box::pin(futures::stream::iter(deluge)),
            Echo::default(),
            CancelToken::new(),
        ));
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
        assert!(evenements.last().is_some_and(ChatEvent::is_terminal));
    }
}
