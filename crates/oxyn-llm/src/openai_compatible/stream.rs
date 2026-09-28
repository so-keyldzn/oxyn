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

    fn chunks(parts: &[&'static str]) -> ByteStream {
        let items: Vec<std::result::Result<Bytes, String>> = parts
            .iter()
            .map(|p| Ok(Bytes::from_static(p.as_bytes())))
            .collect();
        Box::pin(futures::stream::iter(items))
    }

    fn collect(flux: BoxStream<'static, ChatEvent>) -> Vec<ChatEvent> {
        futures::executor::block_on(flux.collect())
    }

    #[test]
    fn a_complete_stream_becomes_events() {
        let flux = openai_events(
            chunks(&[
                "data: {\"choices\":[{\"delta\":{\"content\":\"SELECT \"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{\"content\":\"1\"}}]}\n\n",
                "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
                "data: [DONE]\n\n",
            ]),
            CancelToken::new(),
            None,
        );
        assert_eq!(
            collect(flux),
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
            chunks(&[
                "data: {\"choices\":[{\"delta\":{\"con",
                "tent\":\"hello\"}}]}\n\ndata: [DONE]\n\n",
            ]),
            CancelToken::new(),
            None,
        );
        let events = collect(flux);
        assert!(
            events.contains(&ChatEvent::TextDelta("hello".to_owned())),
            "{events:?}"
        );
    }

    #[test]
    fn a_byte_by_byte_stream_gives_the_same_result() {
        // The network does not respect frame boundaries: the only split that
        // covers them all is the one that respects none.
        const RAW: &str = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"café\"}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n",
        );
        let bytes: Vec<std::result::Result<Bytes, String>> = RAW
            .as_bytes()
            .iter()
            .map(|byte| Ok(Bytes::copy_from_slice(&[*byte])))
            .collect();
        let events = collect(openai_events(
            Box::pin(futures::stream::iter(bytes)),
            CancelToken::new(),
            None,
        ));
        assert_eq!(
            events,
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
        let token = CancelToken::new();
        let trigger = token.clone();
        let bytes = futures::stream::iter(vec![
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"c\",\"function\":{\"name\":\"drop_table\",\"arguments\":\"{\\\"name\\\":\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"audit\\\"}\"}}]}}]}\n\n",
        ])
        .map(move |chunk| -> std::result::Result<Bytes, String> {
            trigger.cancel();
            Ok(Bytes::from_static(chunk.as_bytes()))
        });

        let events = collect(openai_events(Box::pin(bytes), token, None));
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ChatEvent::ToolCallComplete(_))),
            "truncated arguments are not arguments: {events:?}"
        );
        assert_eq!(
            events.last(),
            Some(&ChatEvent::Done {
                stop_reason: StopReason::Cancelled
            })
        );
    }
}
