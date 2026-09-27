//! The stream of an OpenAI-compatible provider, end to end.
//!
//! The driver is shared by every protocol ([`crate::stream`]); this module
//! only adds the plugging of this protocol's frame decoder, and the tests
//! that check the assembly on real streams — split the way the network
//! splits them, that is, anywhere.

use futures::stream::BoxStream;
use oxyn_core::CancelToken;

use super::decode::ChunkDecoder;
use crate::secret::ApiKey;
use crate::stream::{ByteStream, events_stream};
use crate::types::ChatEvent;

/// Plugs the OpenAI-compatible decoder into the shared driver.
pub(crate) fn openai_events(
    bytes: ByteStream,
    cancel: CancelToken,
    key: Option<ApiKey>,
) -> BoxStream<'static, ChatEvent> {
    events_stream(bytes, ChunkDecoder::new(), cancel, key)
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
    fn a_complete_stream_becomes_events() {
        let flux = openai_events(
            morceaux(&[
                "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ]),
            CancelToken::new(),
            None,
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
    fn a_frame_cut_between_two_network_chunks_is_reassembled() {
        let flux = openai_events(
            morceaux(&[
                "data: {\"choices\":[{\"delta\":{\"con",
                "tent\":\"bonjour\"}}]}\n\ndata: [DONE]\n\n",
            ]),
            CancelToken::new(),
            None,
        );
        let evenements = collecter(flux);
        assert!(
            evenements.contains(&ChatEvent::TextDelta("bonjour".to_owned())),
            "{evenements:?}"
        );
    }

    #[test]
    fn a_byte_by_byte_stream_gives_the_same_result() {
        // The network does not respect frame boundaries: the only split that
        // covers them all is the one that respects none.
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
            None,
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
    fn a_cancellation_in_the_middle_of_a_tool_call_proposes_nothing() {
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

        let evenements = collecter(openai_events(Box::pin(octets), jeton, None));
        assert!(
            !evenements
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "truncated arguments are not arguments: {evenements:?}"
        );
        assert_eq!(
            evenements.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }
}
