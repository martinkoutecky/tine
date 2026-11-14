//! Monotonic eligibility for the git worker's timed commits. O(1), no I/O.

use std::time::{Duration, Instant};

pub(crate) const IDLE: Duration = Duration::from_secs(60);

#[derive(Default)]
pub(crate) struct CommitCadence {
    changed_at: Option<Instant>,
    pending_since: Option<Instant>,
}

impl CommitCadence {
    pub(crate) fn changed(&mut self, now: Instant) {
        self.changed_at = Some(now);
        self.pending_since.get_or_insert(now);
    }

    pub(crate) fn due(
        &self,
        now: Instant,
        last_commit: Option<Instant>,
        interval: Duration,
    ) -> bool {
        self.has_changes()
            && (self
                .changed_at
                .is_some_and(|at| now.duration_since(at) >= IDLE)
                || self.pending_since.is_some_and(|at| {
                    now.duration_since(at) >= interval.max(Duration::from_secs(60 * 60))
                }))
            && last_commit.is_none_or(|at| now.duration_since(at) >= interval)
    }

    pub(crate) fn has_changes(&self) -> bool {
        self.changed_at.is_some()
    }

    pub(crate) fn succeeded(&mut self, committed: bool) {
        self.changed_at = None;
        if committed {
            self.pending_since = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continuous_saves_receive_one_commit_in_61_minutes() {
        let start = Instant::now();
        let mut cadence = CommitCadence::default();
        let mut last = Some(start);
        let mut commits = 0;
        for seconds in (30..=61 * 60).step_by(15) {
            let now = start + Duration::from_secs(seconds);
            if seconds % 30 == 0 {
                cadence.changed(now);
            }
            if cadence.due(now, last, Duration::from_secs(600)) {
                commits += 1;
                cadence.succeeded(true);
                last = Some(now);
            }
        }
        assert_eq!(
            commits, 1,
            "continuous saves must receive one recovery commit"
        );
    }

    #[test]
    fn idle_still_waits_for_both_quiet_and_interval() {
        let start = Instant::now();
        let mut cadence = CommitCadence::default();
        let interval = Duration::from_secs(600);
        cadence.changed(start);
        assert!(!cadence.due(start + IDLE, Some(start), interval));
        cadence.changed(start + interval);
        assert!(!cadence.due(start + interval, Some(start), interval));
        assert!(cadence.due(start + interval + IDLE, Some(start), interval));
    }

    #[test]
    fn continuous_edit_deadlines_never_add_more_than_one_commit_per_hour() {
        for minutes in [1, 10, 120] {
            let start = Instant::now();
            let interval = Duration::from_secs(minutes * 60);
            let mut cadence = CommitCadence::default();
            let mut last = Some(start);
            let mut commits = 0;
            for seconds in (30..=4 * 60 * 60 + 30).step_by(15) {
                let now = start + Duration::from_secs(seconds);
                if seconds % 30 == 0 {
                    cadence.changed(now);
                }
                if cadence.due(now, last, interval) {
                    assert!(
                        now.duration_since(last.unwrap())
                            >= interval.max(Duration::from_secs(3600))
                    );
                    cadence.succeeded(true);
                    last = Some(now);
                    commits += 1;
                }
            }
            // Each next first change occurs 30 seconds after the preceding
            // commit, so the fourth (or second two-hour) deadline is later.
            assert_eq!(commits, if minutes == 120 { 1 } else { 3 });
        }
    }

    #[test]
    fn longer_interval_and_only_a_real_commit_restart_the_deadline() {
        let start = Instant::now();
        let interval = Duration::from_secs(7200);
        let mut cadence = CommitCadence::default();
        cadence.changed(start);
        let hour = start + Duration::from_secs(3600);
        cadence.changed(hour);
        assert!(!cadence.due(hour, None, interval));
        let deadline = start + interval;
        cadence.changed(deadline);
        assert!(cadence.due(deadline, None, interval));
        // A refusal never calls succeeded; another save must not postpone retry.
        cadence.changed(deadline + IDLE);
        assert!(cadence.due(deadline + IDLE, None, interval));
        // Nothing to commit (e.g. only ignored files changed): stop attempts
        // until another revision, retaining the first-change deadline.
        cadence.succeeded(false);
        assert!(!cadence.due(deadline + IDLE, None, interval));
        cadence.changed(deadline + IDLE);
        assert!(cadence.due(deadline + IDLE, None, interval));
        cadence.succeeded(true);
        cadence.changed(deadline + IDLE);
        assert!(!cadence.due(deadline + IDLE, Some(deadline), interval));
    }
}
