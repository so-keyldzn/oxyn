//! Le flux d'un fournisseur compatible OpenAI, de bout en bout.
//!
//! Le pilote est commun à tous les protocoles ([`crate::stream`]) ; ce module
//! n'ajoute que le branchement du décodeur de trames de ce protocole-ci, et les
//! tests qui vérifient l'assemblage sur des flux réels — découpés comme le
//! réseau les découpe, c'est-à-dire n'importe où.

use futures::stream::BoxStream;
use oxyn_core::CancelToken;

use super::decode::ChunkDecoder;
use crate::stream::{ByteStream, events_stream};
use crate::types::ChatEvent;

/// Branche le décodeur compatible OpenAI sur le pilote commun.
pub(crate) fn openai_events(
    bytes: ByteStream,
    cancel: CancelToken,
) -> BoxStream<'static, ChatEvent> {
    events_stream(bytes, ChunkDecoder::new(), cancel)
}

#[cfg(test)]
mod tests {
    use bytes::Bytes;
    use futures::stream::StreamExt;

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

    #[test]
    fn un_flux_complet_devient_des_evenements() {
        let flux = openai_events(
            morceaux(&[
                "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ]),
            CancelToken::new(),
        );
        assert_eq!(
            collecter(flux),
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
        let flux = openai_events(
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
    fn un_flux_octet_par_octet_donne_le_meme_resultat() {
        // Le réseau ne respecte pas les frontières de trame : le seul découpage
        // qui les couvre tous est celui qui n'en respecte aucune.
        const BRUT: &str = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"café\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let octets: Vec<std::result::Result<Bytes, String>> = BRUT
            .as_bytes()
            .iter()
            .map(|octet| Ok(Bytes::copy_from_slice(&[*octet])))
            .collect();
        let evenements = collecter(openai_events(
            Box::pin(futures::stream::iter(octets)),
            CancelToken::new(),
        ));
        assert_eq!(
            evenements,
            vec![
                ChatEvent::TextDelta("café".to_owned()),
                ChatEvent::Done {
                    stop_reason: StopReason::EndTurn
                },
            ]
        );
    }

    #[test]
    fn une_annulation_au_milieu_d_un_appel_d_outil_ne_propose_rien() {
        let jeton = CancelToken::new();
        let declencheur = jeton.clone();
        let octets = futures::stream::iter(vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"drop_table\",\"arguments\":\"{\\\"nom\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"audit\\\"}\"}}]}}]}\n\n",
        ])
        .map(move |morceau| -> std::result::Result<Bytes, String> {
            declencheur.cancel();
            Ok(Bytes::from_static(morceau.as_bytes()))
        });

        let evenements = collecter(openai_events(Box::pin(octets), jeton));
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "des arguments tronqués ne sont pas des arguments : {evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }
}
