//! One live subscription per window and per stream, whatever the webview
//! does.
//!
//! A reload of the webview drops its callbacks but not the tasks that feed
//! them: `Channel::send` keeps returning `Ok` while the JavaScript side logs
//! « Couldn't find callback id ». A task that waits for `send` to fail never
//! ends. The page subscribes again on load; that subscription ends the
//! previous one of **the same window** at once, and no other window's
//! ([ADR-0043](../../../../docs/adr/0043-multi-fenetre.md)). A window's close
//! ends all of its subscriptions.

use tokio::sync::watch;

use crate::backend::Backend;
use crate::backend::WindowKey;
use crate::backend::windows::Stream;
use crate::ipc::IpcError;

/// Ends when a newer subscription of the same window to the same stream
/// starts, or when the window closes.
pub(crate) struct Superseded(watch::Receiver<u64>);

impl Superseded {
    /// Starts a new generation of `stream` for `window`, ending its previous
    /// subscriber.
    ///
    /// # Errors
    /// A window that is gone subscribes to nothing.
    pub(crate) fn start(
        backend: &Backend,
        window: WindowKey,
        stream: Stream,
    ) -> Result<Self, IpcError> {
        backend
            .inner
            .windows
            .supersede(window, stream)
            .map(Self)
            .ok_or_else(|| IpcError::invalid("This window is closed"))
    }

    pub(crate) async fn wait(&mut self) {
        // An error means the counter is gone with its window: the
        // subscription ends too.
        let _ = self.0.changed().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_subscription_ends_the_previous_one_of_its_window_only() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        runtime.block_on(async {
            let backend = Backend::open_temporary().expect("temporary backend");
            let left = backend.reserve_window(true).expect("a first window");
            let right = backend.reserve_window(false).expect("a second window");
            let mut first = Superseded::start(&backend, left, Stream::Events).expect("open");
            let mut other = Superseded::start(&backend, right, Stream::Events).expect("open");
            let mut second = Superseded::start(&backend, left, Stream::Events).expect("open");
            tokio::time::timeout(std::time::Duration::from_secs(1), first.wait())
                .await
                .expect("the first subscription is superseded");
            let short = std::time::Duration::from_millis(50);
            assert!(
                tokio::time::timeout(short, second.wait()).await.is_err(),
                "the latest subscription stays live"
            );
            assert!(
                tokio::time::timeout(short, other.wait()).await.is_err(),
                "another window's subscription is untouched"
            );
            // Closing a window ends its subscriptions.
            let _ = backend.inner.windows.forget(right);
            tokio::time::timeout(std::time::Duration::from_secs(1), other.wait())
                .await
                .expect("a closed window's subscription ends");
            assert!(Superseded::start(&backend, right, Stream::Events).is_err());
        });
    }
}
