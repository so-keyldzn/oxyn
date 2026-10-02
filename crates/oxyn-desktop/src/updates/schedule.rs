//! When the scheduler checks. Pure: the clock is passed in.
//!
//! A 24 h `sleep` is not used: the monotonic clock stops while the machine
//! sleeps, so a laptop opened every morning would never reach it. The
//! scheduler wakes every hour instead and reads the wall clock.

use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};

/// The first check, after the launch has settled: the workspace, the
/// windows and the user's first query come first.
pub(crate) const FIRST_CHECK: Duration = Duration::from_secs(60);

/// How often the scheduler reads the wall clock.
pub(crate) const TICK: Duration = Duration::from_secs(60 * 60);

/// How old the last successful check may be.
pub(crate) const INTERVAL: TimeDelta = TimeDelta::hours(24);

/// Whether a check is due. The first one always is; a clock moved back
/// before the last check makes one due too, rather than none for days.
pub(crate) fn due(last_success: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    match last_success {
        None => true,
        Some(last) => now < last || now.signed_duration_since(last) >= INTERVAL,
    }
}
