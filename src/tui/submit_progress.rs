//! In-progress and completed state for a `submit` run (see
//! [`crate::stack::submit_layer`] and `Action::SubmitStack` in
//! `tui::app`), tracked per layer so a partial failure on one layer never
//! hides the result of the others.

/// The state of one layer's submission, updated as its push and pull
/// request steps complete.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LayerSubmitStatus {
    /// Not yet started.
    Pending,
    /// The branch is being pushed.
    InProgress,
    /// The branch pushed successfully; its pull request is being synced.
    Pushed,
    /// A new pull request was created for this layer.
    PullRequestCreated { number: u64 },
    /// This layer's existing pull request was brought up to date.
    PullRequestUpdated { number: u64 },
    /// The push or the pull request step failed; the message is a
    /// user-facing summary (already run through `friendly_shell_error`).
    Failed(String),
}

impl LayerSubmitStatus {
    /// Whether this layer has reached a final state (succeeded or failed) —
    /// as opposed to still being pending or in progress.
    pub(crate) fn is_terminal(&self) -> bool {
        matches!(
            self,
            LayerSubmitStatus::PullRequestCreated { .. }
                | LayerSubmitStatus::PullRequestUpdated { .. }
                | LayerSubmitStatus::Failed(_)
        )
    }

    pub(crate) fn is_failed(&self) -> bool {
        matches!(self, LayerSubmitStatus::Failed(_))
    }
}

/// One layer's branch and its current submit status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubmitLayerProgress {
    pub(crate) branch: String,
    pub(crate) status: LayerSubmitStatus,
}

/// Progress for submitting every layer of one stack.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SubmitProgress {
    pub(crate) stack_index: usize,
    pub(crate) layers: Vec<SubmitLayerProgress>,
    /// Set once every layer has reached a terminal status.
    pub(crate) finished: bool,
}

impl SubmitProgress {
    pub(crate) fn new(stack_index: usize, branches: Vec<String>) -> Self {
        Self {
            stack_index,
            layers: branches
                .into_iter()
                .map(|branch| SubmitLayerProgress {
                    branch,
                    status: LayerSubmitStatus::Pending,
                })
                .collect(),
            finished: false,
        }
    }

    pub(crate) fn set_status(&mut self, layer_index: usize, status: LayerSubmitStatus) {
        if let Some(layer) = self.layers.get_mut(layer_index) {
            layer.status = status;
        }
    }

    pub(crate) fn total(&self) -> usize {
        self.layers.len()
    }

    pub(crate) fn completed_count(&self) -> usize {
        self.layers
            .iter()
            .filter(|layer| layer.status.is_terminal())
            .count()
    }

    pub(crate) fn failed_count(&self) -> usize {
        self.layers
            .iter()
            .filter(|layer| layer.status.is_failed())
            .count()
    }

    /// Fraction of layers that have reached a terminal status, for driving a
    /// progress gauge.
    pub(crate) fn ratio(&self) -> f64 {
        if self.total() == 0 {
            1.0
        } else {
            self.completed_count() as f64 / self.total() as f64
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_progress_starts_every_layer_pending() {
        let progress = SubmitProgress::new(0, vec!["a".to_string(), "b".to_string()]);

        assert_eq!(progress.total(), 2);
        assert_eq!(progress.completed_count(), 0);
        assert_eq!(progress.ratio(), 0.0);
        assert!(!progress.finished);
    }

    #[test]
    fn set_status_updates_the_named_layer_only() {
        let mut progress = SubmitProgress::new(0, vec!["a".to_string(), "b".to_string()]);

        progress.set_status(0, LayerSubmitStatus::PullRequestCreated { number: 1 });

        assert_eq!(
            progress.layers[0].status,
            LayerSubmitStatus::PullRequestCreated { number: 1 }
        );
        assert_eq!(progress.layers[1].status, LayerSubmitStatus::Pending);
        assert_eq!(progress.completed_count(), 1);
        assert_eq!(progress.ratio(), 0.5);
    }

    #[test]
    fn partial_failure_is_reported_per_layer_not_as_a_single_outcome() {
        let mut progress =
            SubmitProgress::new(0, vec!["a".to_string(), "b".to_string(), "c".to_string()]);

        progress.set_status(0, LayerSubmitStatus::PullRequestCreated { number: 1 });
        progress.set_status(1, LayerSubmitStatus::Failed("push rejected".to_string()));
        progress.set_status(2, LayerSubmitStatus::PullRequestUpdated { number: 2 });

        assert_eq!(progress.completed_count(), 3);
        assert_eq!(progress.failed_count(), 1);
        assert!(!progress.layers[0].status.is_failed());
        assert!(progress.layers[1].status.is_failed());
        assert!(!progress.layers[2].status.is_failed());
    }

    #[test]
    fn set_status_on_an_out_of_range_layer_is_a_no_op() {
        let mut progress = SubmitProgress::new(0, vec!["a".to_string()]);

        progress.set_status(5, LayerSubmitStatus::InProgress);

        assert_eq!(progress.layers[0].status, LayerSubmitStatus::Pending);
    }
}
