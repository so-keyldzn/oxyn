//! [`HostConfirm`] answered by a script: the tests' dialog, which opens no
//! window.

use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use tokio::sync::Notify;
use tokio::time::Instant;

use super::{Confirmation, HostConfirm, HostReply, Reply};

/// What the scripted dialog answers, counted from when it is shown.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Answer {
    Confirm,
    Refuse,
    /// Confirms once it has been on screen this long.
    ConfirmAfter(Duration),
    /// Never answers, and leaves the screen once nobody awaits it: the
    /// dialog left on screen is [`Answer::ConfirmAfter`] past the deadline.
    Never,
}

/// Records every confirmation it is asked, and answers from its script.
///
/// Like the native dialog, it shows one dialog at a time: a dialog stays on
/// screen until its scripted answer, even once the backend stopped waiting
/// for it, and the next one is shown only then.
pub(crate) struct ScriptedConfirm {
    answer: Mutex<Answer>,
    shown: Mutex<Vec<Confirmation>>,
    /// When the dialog on screen closes.
    screen_free: Mutex<Instant>,
    asked: Notify,
}

impl ScriptedConfirm {
    pub(crate) fn new(answer: Answer) -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(answer),
            shown: Mutex::new(Vec::new()),
            screen_free: Mutex::new(Instant::now()),
            asked: Notify::new(),
        })
    }

    /// Changes the answer to the next confirmations.
    pub(crate) fn answer(&self, answer: Answer) {
        *self.answer.lock() = answer;
    }

    /// Every confirmation asked so far, shown or still waiting for the screen.
    pub(crate) fn shown(&self) -> Vec<Confirmation> {
        self.shown.lock().clone()
    }

    /// Waits until at least `count` confirmations were asked.
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
    fn confirm(&self, confirmation: Confirmation, deadline: Instant) -> HostReply {
        self.shown.lock().push(confirmation);
        self.asked.notify_waiters();
        let answer = *self.answer.lock();
        // Decided now, for the whole queue: the dialog on screen closes at its
        // scripted time whether or not anyone still awaits its reply.
        let (shown, closed) = {
            let mut screen_free = self.screen_free.lock();
            let shown = (*screen_free).max(Instant::now());
            if shown >= deadline {
                // Its turn comes too late: it is never drawn.
                return Box::pin(std::future::pending());
            }
            let closed = match answer {
                Answer::Confirm | Answer::Refuse => shown,
                Answer::ConfirmAfter(delay) => shown + delay,
                Answer::Never => return Box::pin(std::future::pending()),
            };
            *screen_free = closed;
            (shown, closed)
        };
        Box::pin(async move {
            tokio::time::sleep_until(closed).await;
            match answer {
                Answer::Refuse => Reply::Refused,
                _ => Reply::Confirmed { shown },
            }
        })
    }
}
