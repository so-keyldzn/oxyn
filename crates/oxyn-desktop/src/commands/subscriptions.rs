//! One live subscription per stream, whatever the webview does.
//!
//! A reload of the webview drops its callbacks but not the tasks that feed
//! them: `Channel::send` keeps returning `Ok` while the JavaScript side logs
//! « Couldn't find callback id ». A task that waits for `send` to fail never
//! ends. The page subscribes again on load; that subscription ends the
//! previous one at once.

use std::sync::OnceLock;

use tokio::sync::watch;

/// The generation counter of one stream.
pub(crate) type Stream = OnceLock<watch::Sender<u64>>;

/// Ends when a newer subscription to the same stream starts.
pub(crate) struct Superseded(watch::Receiver<u64>);

impl Superseded {
    pub(crate) async fn wait(&mut self) {
        // The counter is a process-wide static: its sender never drops, so an
        // error cannot happen and would mean « no subscriber left » anyway.
        let _ = self.0.changed().await;
    }
}

/// Starts a new generation of `stream`, ending the previous subscriber.
pub(crate) fn supersede(stream: &Stream) -> Superseded {
    let sender = stream.get_or_init(|| watch::channel(0).0);
    sender.send_modify(|generation| *generation = generation.wrapping_add(1));
    // Subscribed after the bump: this generation is already seen.
    Superseded(sender.subscribe())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_subscription_ends_the_previous_one_only() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            static STREAM: Stream = OnceLock::new();
            let mut first = supersede(&STREAM);
            let mut second = supersede(&STREAM);
            tokio::time::timeout(std::time::Duration::from_secs(1), first.wait())
                .await
                .expect("the first subscription is superseded");
            assert!(
                tokio::time::timeout(std::time::Duration::from_millis(50), second.wait())
                    .await
                    .is_err(),
                "the latest subscription stays live"
            );
        });
    }
}
