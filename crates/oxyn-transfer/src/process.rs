//! Blocking, owned Unix process groups; a leader is never signalled after reaping.
use crate::{Result, TransferError};
use oxyn_core::CancelToken;
use std::process::Command;
use std::time::Duration;

/// Only inspection captures stdout. Archive/data streams go to an owned file/FD.
pub(crate) struct Observation {
    pub code: Option<i32>,
    pub cancelled: bool,
    pub deadline: bool,
    pub cleanup: bool,
    pub stdout: Vec<u8>,
    pub overflow: bool,
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
mod unix {
    use super::*;
    use rustix::process::{Pid, Signal, WaitId, WaitidOptions};
    use std::io::Read;
    use std::os::fd::AsFd;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Stdio};
    use std::time::Instant;

    const POLL: Duration = Duration::from_millis(10);
    const SIGNAL_GRACE: Duration = Duration::from_secs(2);
    const PIPE_GRACE: Duration = Duration::from_secs(2);

    fn poll_pause() {
        #[expect(
            clippy::disallowed_methods,
            reason = "ADR-0083: bounded process/pipe polling on the owned blocking worker, never the UI or async executor thread"
        )]
        std::thread::sleep(POLL);
    }

    struct Owned {
        child: Child,
        pid: Pid,
        reserved: bool,
    }
    impl Owned {
        fn signal(&mut self, signal: Signal) -> bool {
            if !self.reserved {
                return false;
            }
            match rustix::process::kill_process_group(self.pid, signal) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => true,
                Err(_) => false,
            }
        }
        fn finish_group(&mut self, pipes_closed: bool) -> bool {
            if !self.reserved {
                return false;
            }
            match rustix::process::kill_process_group(self.pid, Signal::Kill) {
                Ok(()) | Err(rustix::io::Errno::SRCH) => true,
                // XNU excludes zombies when signalling a group, then returns
                // EPERM when no live permitted target remains. This path is
                // reached only after waitid observed our still-reserved leader
                // exit and all inherited pipe writers closed. Never accept
                // EPERM while the leader or a pipe-owning descendant is live.
                #[cfg(target_os = "macos")]
                Err(rustix::io::Errno::PERM) if pipes_closed => true,
                Err(_) => {
                    let _ = pipes_closed;
                    false
                }
            }
        }
        fn reap(&mut self) -> std::io::Result<std::process::ExitStatus> {
            let status = self.child.wait();
            // Never signal the numerical group again after wait: PID reuse is possible.
            self.reserved = false;
            status
        }
    }
    impl Drop for Owned {
        fn drop(&mut self) {
            if self.reserved {
                let _ = self.signal(Signal::Kill);
                let _ = self.child.kill();
                let _ = self.reap();
            }
        }
    }
    fn nonblocking(fd: impl AsFd) -> Result<()> {
        let flags = rustix::fs::fcntl_getfl(&fd).map_err(|_| TransferError::Cleanup)?;
        rustix::fs::fcntl_setfl(fd, flags | rustix::fs::OFlags::NONBLOCK)
            .map_err(|_| TransferError::Cleanup)
    }
    fn drain(
        reader: &mut impl Read,
        mut capture: Option<&mut Vec<u8>>,
        cap: usize,
        overflow: &mut bool,
    ) -> std::io::Result<bool> {
        let mut buffer = [0u8; 8192];
        // Even infinite output gives the cancellation/deadline loop a turn.
        for _ in 0..8 {
            match reader.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(count) => {
                    if let Some(output) = capture.as_deref_mut() {
                        let keep = count.min(cap.saturating_sub(output.len()));
                        if let Some(bytes) = buffer.get(..keep) {
                            output.extend_from_slice(bytes);
                        }
                        *overflow |= keep != count;
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }
    pub(super) fn run(
        mut command: Command,
        cancel: &CancelToken,
        source_closed: &CancelToken,
        deadline: Duration,
        capture: Option<usize>,
        tick: &mut dyn FnMut(bool),
    ) -> Result<Observation> {
        if cancel.is_cancelled() || source_closed.is_cancelled() {
            return Err(TransferError::Cancelled);
        }
        command.process_group(0).stderr(Stdio::piped());
        if capture.is_some() {
            command.stdout(Stdio::piped());
        }
        let mut child = command.spawn().map_err(|_| TransferError::Tool)?;
        let pid = i32::try_from(child.id())
            .ok()
            .filter(|pid| *pid > 1)
            .and_then(Pid::from_raw);
        let Some(pid) = pid else {
            let _ = child.kill();
            let _ = child.wait();
            return Err(TransferError::Cleanup);
        };
        let mut owned = Owned {
            child,
            pid,
            reserved: true,
        };
        let mut stderr = owned.child.stderr.take().ok_or(TransferError::Cleanup)?;
        nonblocking(&stderr)?;
        let mut stdout = owned.child.stdout.take();
        if let Some(stdout) = &stdout {
            nonblocking(stdout)?;
        }
        let started = Instant::now();
        let mut stopping: Option<Instant> = None;
        let mut forced = false;
        let mut cleanup = true;
        let mut result = Observation {
            code: None,
            cancelled: false,
            deadline: false,
            cleanup: true,
            stdout: Vec::new(),
            overflow: false,
        };
        let mut stderr_eof;
        let mut stdout_eof = stdout.is_none();
        loop {
            stderr_eof = drain(&mut stderr, None, 0, &mut result.overflow)
                .map_err(|_| TransferError::Cleanup)?;
            if let Some(stdout) = &mut stdout {
                stdout_eof = drain(
                    stdout,
                    Some(&mut result.stdout),
                    capture.unwrap_or(0),
                    &mut result.overflow,
                )
                .map_err(|_| TransferError::Cleanup)?;
            }
            let status = match rustix::process::waitid(
                WaitId::Pid(pid),
                WaitidOptions::EXITED | WaitidOptions::NOWAIT | WaitidOptions::NOHANG,
            ) {
                Ok(status) => status,
                Err(rustix::io::Errno::INTR) => continue,
                Err(rustix::io::Errno::CHILD) => {
                    // Another component reaped it: ownership can no longer be proven.
                    owned.reserved = false;
                    return Err(TransferError::Cleanup);
                }
                Err(_) => return Err(TransferError::Cleanup),
            };
            if status.is_some()
                && (stopping.is_none()
                    || stderr_eof && stdout_eof
                    || stopping.is_some_and(|at| at.elapsed() >= SIGNAL_GRACE))
            {
                cleanup &= owned.finish_group(stderr_eof && stdout_eof);
                break;
            }
            if stopping.is_none()
                && (cancel.is_cancelled()
                    || source_closed.is_cancelled()
                    || started.elapsed() >= deadline
                    || result.overflow)
            {
                result.cancelled = cancel.is_cancelled() || source_closed.is_cancelled();
                result.deadline = started.elapsed() >= deadline;
                stopping = Some(Instant::now());
                cleanup &= owned.signal(Signal::Int);
                tick(true);
            }
            if stopping.is_some_and(|at| at.elapsed() >= SIGNAL_GRACE) && !forced {
                cleanup &= owned.signal(Signal::Kill);
                let _ = owned.child.kill();
                forced = true;
            }
            tick(stopping.is_some());
            poll_pause();
        }
        let status = owned.reap().map_err(|_| TransferError::Cleanup)?;
        result.code = status.code();
        let draining = Instant::now();
        while !(stderr_eof && stdout_eof) && draining.elapsed() < PIPE_GRACE {
            stderr_eof = drain(&mut stderr, None, 0, &mut result.overflow)
                .map_err(|_| TransferError::Cleanup)?;
            if let Some(stdout) = &mut stdout {
                stdout_eof = drain(
                    stdout,
                    Some(&mut result.stdout),
                    capture.unwrap_or(0),
                    &mut result.overflow,
                )
                .map_err(|_| TransferError::Cleanup)?;
            }
            if !(stderr_eof && stdout_eof) {
                poll_pause();
            }
        }
        result.cleanup = cleanup && stderr_eof && stdout_eof;
        Ok(result)
    }
}

/// Called only on an owned blocking worker; callers retain its handle until cleanup.
pub(crate) fn run(
    command: Command,
    cancel: &CancelToken,
    source_closed: &CancelToken,
    deadline: Duration,
    capture: Option<usize>,
    tick: &mut dyn FnMut(bool),
) -> Result<Observation> {
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        unix::run(command, cancel, source_closed, deadline, capture, tick)
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = (command, cancel, source_closed, deadline, capture, tick);
        Err(TransferError::Unavailable(
            "Native process-tree cleanup is not implemented on this platform",
        ))
    }
}

#[cfg(all(test, any(target_os = "macos", target_os = "linux")))]
mod tests;
