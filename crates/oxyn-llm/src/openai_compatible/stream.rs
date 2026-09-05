//! Du flux d'octets HTTP au flux d'événements.
//!
//! Ce module ne parle ni de fournisseur ni d'authentification : il assemble le
//! décodeur SSE ([`crate::sse`]) et le décodeur de trames
//! ([`super::decode`]) en un [`Stream`] annulable.
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

use super::decode::ChunkDecoder;
use crate::sse::SseDecoder;
use crate::types::ChatEvent;

/// Décrit une rupture de flux **sans** reprendre le message brut.
///
/// Le message d'une erreur de transport peut contenir l'URL, donc les
/// identifiants qu'elle porterait. On classe plutôt que de recopier.
pub(crate) fn describe_stream_error(err: &reqwest::Error) -> String {
    if err.is_timeout() {
        "délai dépassé pendant la réception du flux".to_owned()
    } else if err.is_body() || err.is_decode() {
        "flux interrompu par le fournisseur".to_owned()
    } else {
        "connexion perdue pendant la réception du flux".to_owned()
    }
}

/// Flux d'octets déjà classé, tel que le décodeur le consomme.
pub(crate) type ByteStream = Pin<Box<dyn Stream<Item = std::result::Result<Bytes, String>> + Send>>;

/// État porté d'un pas de décodage à l'autre.
struct StreamState {
    bytes: ByteStream,
    sse: SseDecoder,
    decoder: ChunkDecoder,
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
pub(crate) fn events_stream(
    bytes: ByteStream,
    cancel: CancelToken,
) -> BoxStream<'static, ChatEvent> {
    let etat = StreamState {
        bytes,
        sse: SseDecoder::new(),
        decoder: ChunkDecoder::new(),
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

/// Vide le décodeur SSE dans le décodeur de trames.
fn drain(etat: &mut StreamState, sorties: &mut Vec<ChatEvent>) {
    loop {
        // `let … else` plutôt que `while let` : l'emprunt du décodeur SSE se
        // termine à la fin de l'instruction, avant que le décodeur de trames
        // ne soit emprunté à son tour.
        let Some(trame) = etat.sse.next_frame() else {
            return;
        };
        etat.decoder.on_data(&trame.data, sorties);
        if etat.decoder.is_done() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::StopReason;

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

    // ── Flux ───────────────────────────────────────────────────────────────

    #[test]
    fn un_flux_complet_devient_des_evenements() {
        let flux = events_stream(
            morceaux(&[
                "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ]),
            CancelToken::new(),
        );
        let evenements = collecter(flux);
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("SELECT ".to_owned()),
                ChatEvent::TextDelta("1".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn une_trame_coupee_entre_deux_morceaux_reseau_se_recolle() {
        let flux = events_stream(
            morceaux(&[
                "data: {\"choices\":[{\"delta\":{\"con",
                "tent\":\"bonjour\"}}]}\n\ndata: [DONE]\n\n",
            ]),
            CancelToken::new(),
        );
        let evenements = collecter(flux);
        assert!(
            evenements.contains(&ChatEvent::TextDelta("bonjour".to_owned())),
            "{evenements:?}"
        );
    }

    #[test]
    fn un_jeton_deja_annule_ne_lit_aucun_morceau() {
        let jeton = CancelToken::new();
        jeton.cancel();
        let flux = events_stream(
            morceaux(&["data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n"]),
            jeton,
        );
        assert_eq!(
            collecter(flux),
            vec![ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            }]
        );
    }

    #[test]
    fn une_annulation_en_cours_de_flux_arrete_la_lecture() {
        let jeton = CancelToken::new();
        let declencheur = jeton.clone();
        let octets = futures::stream::iter(vec![
            "data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"b\"}}]}\n\n",
        ])
        .map(move |morceau| -> std::result::Result<Bytes, String> {
            declencheur.cancel();
            Ok(Bytes::from_static(morceau.as_bytes()))
        });

        let evenements = collecter(events_stream(Box::pin(octets), jeton));
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
            Ok(Bytes::from_static(
                b"data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n",
            )),
            Err("connexion perdue pendant la réception du flux".to_owned()),
        ]);
        let evenements = collecter(events_stream(Box::pin(octets), CancelToken::new()));
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
    fn un_flux_ferme_sans_sentinelle_se_termine_quand_meme() {
        let flux = events_stream(
            morceaux(&["data: {\"choices\":[{\"delta\":{\"content\":\"a\"}}]}\n\n"]),
            CancelToken::new(),
        );
        let evenements = collecter(flux);
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Unspecified
            })
        );
    }

    #[test]
    fn un_flux_vide_produit_tout_de_meme_une_fin() {
        let evenements = collecter(events_stream(morceaux(&[]), CancelToken::new()));
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
    }

    #[test]
    fn done_n_est_emis_qu_une_fois_meme_avec_des_trames_apres() {
        let flux = events_stream(
            morceaux(&[
                "data: [DONE]\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"fantome\"}}]}\n\n",
            ]),
            CancelToken::new(),
        );
        let evenements = collecter(flux);
        assert_eq!(evenements.len(), 1, "{evenements:?}");
        assert!(evenements[0].is_terminal());
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
            CancelToken::new(),
        ));
        assert!(
            evenements.iter().any(|e| matches!(e, ChatEvent::Error(_))),
            "{evenements:?}"
        );
        assert!(evenements.last().is_some_and(ChatEvent::is_terminal));
    }
}
