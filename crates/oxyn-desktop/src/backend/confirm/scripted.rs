//! [`HostConfirm`] answered by a script: the tests' dialog, which opens no
//! window.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;

use super::{Confirmation, HostConfirm, HostReply};

/// What the scripted dialog answers.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Answer {
    Confirm,
    Refuse,
    /// Confirms once this much time has passed.
    ConfirmAfter(Duration),
    /// Never answers, like a dialog left on screen.
    Never,
}

/// Records every confirmation it is shown, and answers from its script.
pub(crate) struct ScriptedConfirm {
    answer: Mutex<Answer>,
    shown: Mutex<Vec<Confirmation>>,
    asked: Notify,
}

impl ScriptedConfirm {
    pub(crate) fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            shown: Mutex::new(Vec::new()),
            asked: Notify::new(),
        })
    }

    /// Changes the answer to the next confirmations.
    pub(crate) fn answer(&self, answer: Answer) {
        *self.answer.lock() = answer;
    }

    /// Every confirmation shown so far.
    pub(crate) fn shown(&self) -> Vec<Confirmation> {
        self.shown.lock().clone()
    }

    /// Waits until at least `count` confirmations were shown.
    pub(crate) async fn wait_shown(&self, count: usize) {
        loop {
            let notified = self.asked.notified();
            if self.shown.lock().len() >= count {
                return;
            }
            notified.await;
        }
    }
}

impl HostConfirm for ScriptedConfirm {
    fn confirm(&self, confirmation: Confirmation) -> HostReply {
        self.shown.lock().push(confirmation);
        self.asked.notify_waiters();
        match *self.answer.lock() {
            Answer::Confirm => Box::pin(async { true }),
            Answer::Refuse => Box::pin(async { false }),
            Answer::ConfirmAfter(delay) => Box::pin(async move {
                tokio::time::sleep(delay).await;
                true
            }),
            Answer::Never => Box::pin(std::future::pending()),
        }
    }
}
