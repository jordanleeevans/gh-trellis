//! Periodic background refresh, driven by the `refresh_interval_secs`
//! config option (0 disables it).

use std::time::{Duration, Instant};

use crate::tui::state::AppState;

pub(super) struct AutoRefresh {
    interval: Option<Duration>,
    /// When the most recent refresh (manual or automatic) started.
    last_started: Instant,
    seen_request_id: u64,
}

impl AutoRefresh {
    pub(super) fn new(interval_secs: u64, now: Instant) -> Self {
        Self {
            interval: (interval_secs > 0).then(|| Duration::from_secs(interval_secs)),
            last_started: now,
            seen_request_id: 0,
        }
    }

    /// Whether to start a refresh now.
    ///
    /// Any refresh, including a manual one, restarts the countdown. It
    /// never overlaps a refresh in flight, and it waits while a confirm
    /// modal is open, a submit, sync or rebase is running, or the conflict
    /// view is open: a refresh can reorder the stack list, pending actions
    /// refer to stacks by index, and mid-rebase the stack list can't be
    /// read reliably (the conflict view reloads with its own key).
    pub(super) fn due(&mut self, now: Instant, state: &AppState) -> bool {
        let Some(interval) = self.interval else {
            return false;
        };

        if state.refresh_request_id != self.seen_request_id {
            self.seen_request_id = state.refresh_request_id;
            self.last_started = now;
            return false;
        }

        let busy = state.refresh_in_flight
            || state.confirm.is_some()
            || state.sync_in_flight
            || state.rebase_in_flight.is_some()
            || state.conflict.is_some()
            || state
                .submit_progress
                .as_ref()
                .is_some_and(|progress| !progress.finished);

        !busy && now.duration_since(self.last_started) >= interval
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::components::confirm::ConfirmModal;

    const SECOND: Duration = Duration::from_secs(1);

    #[test]
    fn disabled_when_interval_is_zero() {
        let start = Instant::now();
        let mut auto = AutoRefresh::new(0, start);
        assert!(!auto.due(start + 3600 * SECOND, &AppState::default()));
    }

    #[test]
    fn due_once_the_interval_has_passed() {
        let start = Instant::now();
        let mut auto = AutoRefresh::new(30, start);
        let state = AppState::default();
        assert!(!auto.due(start + 29 * SECOND, &state));
        assert!(auto.due(start + 30 * SECOND, &state));
    }

    #[test]
    fn any_refresh_restarts_the_countdown() {
        let start = Instant::now();
        let mut auto = AutoRefresh::new(30, start);
        // A manual refresh 20s in.
        let state = AppState {
            refresh_request_id: 1,
            ..AppState::default()
        };
        assert!(!auto.due(start + 20 * SECOND, &state));

        assert!(!auto.due(start + 40 * SECOND, &state));
        assert!(auto.due(start + 50 * SECOND, &state));
    }

    #[test]
    fn waits_while_busy() {
        let start = Instant::now();
        let later = start + 60 * SECOND;
        let mut auto = AutoRefresh::new(30, start);

        let in_flight = AppState {
            refresh_in_flight: true,
            ..AppState::default()
        };
        assert!(!auto.due(later, &in_flight));

        let confirming = AppState {
            confirm: Some(ConfirmModal::new("t", "b", "ok", false)),
            ..AppState::default()
        };
        assert!(!auto.due(later, &confirming));

        let syncing = AppState {
            sync_in_flight: true,
            ..AppState::default()
        };
        assert!(!auto.due(later, &syncing));

        let rebasing = AppState {
            rebase_in_flight: Some(crate::tui::state::RebaseOp::Rebase),
            ..AppState::default()
        };
        assert!(!auto.due(later, &rebasing));

        assert!(auto.due(later, &AppState::default()));
    }
}
