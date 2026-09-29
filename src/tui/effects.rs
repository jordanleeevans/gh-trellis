//! Runs every process-touching [`Action`] off the render loop.
//!
//! The reducer in [`super::app`] never awaits a shell command. When an
//! action needs one, the reducer calls a method here, which spawns a tokio
//! task against the injected [`Shell`] and sends the outcome back through
//! the app's channel as further `Action`s (a `*Loaded`/`*Finished` result,
//! a follow-up like [`Action::RefreshStacks`], or [`Action::SetError`]).

use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::mpsc;

use crate::git;
use crate::shell::Shell;
use crate::stack::{
    Layer, RebaseDriver, RebaseScope, SubmitEvent, SubmitLayerOutcome, SubmitOptions, SyncOptions,
    UnstackScope, abort_rebase, continue_rebase, hydrate_layer_detail, interrupted_rebase,
    list_stacks, rebase_stack, stage_resolved, submit_stack, sync_stack, unstack_stack,
};

use super::action::Action;
use super::messages::friendly_shell_error;
use super::state::submit_progress::LayerSubmitStatus;

pub(crate) struct Effects {
    repo: PathBuf,
    shell: Arc<dyn Shell>,
    tx: mpsc::UnboundedSender<Action>,
    in_flight: Arc<AtomicUsize>,
}

/// Decrements the in-flight count when a task ends, including by panic.
struct InFlightGuard(Arc<AtomicUsize>);

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl Effects {
    pub(crate) fn new(
        repo: PathBuf,
        shell: Arc<dyn Shell>,
        tx: mpsc::UnboundedSender<Action>,
    ) -> Self {
        Self {
            repo,
            shell,
            tx,
            in_flight: Arc::new(AtomicUsize::new(0)),
        }
    }

    /// Spawns `task` and sends every `Action` it returns back to the app.
    fn spawn<F, Fut>(&self, task: F)
    where
        F: FnOnce(Arc<dyn Shell>, PathBuf, mpsc::UnboundedSender<Action>) -> Fut,
        Fut: Future<Output = Vec<Action>> + Send + 'static,
    {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
        let guard = InFlightGuard(self.in_flight.clone());
        let tx = self.tx.clone();
        let future = task(self.shell.clone(), self.repo.clone(), self.tx.clone());
        tokio::spawn(async move {
            let _guard = guard;
            for action in future.await {
                let _ = tx.send(action);
            }
        });
    }

    pub(crate) fn load_stacks(&self, request_id: u64) {
        self.spawn(|shell, repo, _| async move {
            let result = list_stacks(&shell, &repo)
                .await
                .map_err(|error| error.to_string());
            vec![Action::StackRefreshSucceeded { request_id, result }]
        });
    }

    pub(crate) fn load_detail(&self, cache_key: String, layer: Layer) {
        self.spawn(|shell, repo, _| async move {
            let result = hydrate_layer_detail(&shell, &repo, &layer)
                .await
                .map_err(|error| friendly_shell_error("load layer detail", &error));
            vec![Action::LayerDetailLoaded { cache_key, result }]
        });
    }

    pub(crate) fn load_diff(&self, cache_key: String, lower: String, branch: String) {
        self.spawn(|shell, repo, _| async move {
            let result = git::diff(&shell, &repo, &lower, &branch)
                .await
                .map_err(|error| friendly_shell_error("load layer diff", &error));
            vec![Action::LayerDiffLoaded { cache_key, result }]
        });
    }

    pub(crate) fn checkout(&self, branch: String) {
        self.run_gh("checkout stack", vec!["stack", "checkout", &branch], true);
    }

    /// Runs `gh stack unstack` (with `--local` for [`UnstackScope::Local`]),
    /// refreshing stacks on success and reporting failure via `SetError`.
    pub(crate) fn unstack(&self, scope: UnstackScope) {
        self.spawn(move |shell, repo, _| async move {
            match unstack_stack(&shell, &repo, scope).await {
                Ok(()) => vec![Action::RefreshStacks],
                Err(error) => vec![Action::SetError(friendly_shell_error("unstack", &error))],
            }
        });
    }

    pub(crate) fn add_layer(&self, branch: String, message: Option<String>) {
        let mut args = vec!["stack", "add", branch.as_str()];
        if let Some(message) = &message {
            args.extend(["-m", message.as_str()]);
        }
        self.run_gh("add layer", args, true);
    }

    pub(crate) fn open_pull_request(&self, number: u64) {
        let number = number.to_string();
        self.run_gh(
            "open pull request",
            vec!["pr", "view", &number, "--web"],
            false,
        );
    }

    /// Runs one `gh` command, reporting failure via [`Action::SetError`]
    /// and, on success, refreshing stacks when `refresh` is set.
    fn run_gh(&self, context: &'static str, args: Vec<&str>, refresh: bool) {
        let args: Vec<String> = args.into_iter().map(str::to_string).collect();
        self.spawn(move |shell, repo, _| async move {
            let args: Vec<&str> = args.iter().map(String::as_str).collect();
            match shell.run(&repo, "gh", &args).await {
                Err(error) => vec![Action::SetError(friendly_shell_error(context, &error))],
                Ok(_) if refresh => vec![Action::RefreshStacks],
                Ok(_) => Vec::new(),
            }
        });
    }

    /// Submits every layer of the stack at `stack_index`, sending an
    /// [`Action::SubmitLayerProgress`] as each layer's push and pull request
    /// step completes, then [`Action::SubmitFinished`].
    pub(crate) fn submit_stack(
        &self,
        stack_index: usize,
        layers: Vec<Layer>,
        remote: String,
        options: SubmitOptions,
    ) {
        self.spawn(move |shell, repo, tx| async move {
            submit_stack(
                &shell,
                &repo,
                &remote,
                &layers,
                options,
                |layer_index, event| {
                    let _ = tx.send(Action::SubmitLayerProgress {
                        stack_index,
                        layer_index,
                        status: layer_submit_status(event),
                    });
                },
            )
            .await;
            vec![Action::SubmitFinished { stack_index }]
        });
    }

    /// Runs `gh stack sync` for the currently checked-out stack, then sends
    /// [`Action::SyncFinished`] with the outcome (or a user-facing error).
    pub(crate) fn sync_stack(&self, remote: String, options: SyncOptions) {
        self.spawn(move |shell, repo, _| async move {
            let result = sync_stack(&shell, &repo, &remote, options)
                .await
                .map_err(|error| friendly_shell_error("sync stack", &error));
            vec![Action::SyncFinished { result }]
        });
    }

    /// Runs `gh stack rebase` (with `run_long`: a cascade can take well over
    /// the default timeout) on the checked-out stack, then sends
    /// [`Action::RebaseFinished`].
    pub(crate) fn rebase_stack(&self, remote: String, scope: RebaseScope) {
        self.spawn(move |shell, repo, _| async move {
            let result = rebase_stack(&shell, &repo, &remote, scope)
                .await
                .map_err(|error| friendly_shell_error("rebase stack", &error));
            vec![Action::RebaseFinished { result }]
        });
    }

    /// Continues an interrupted rebase, then sends [`Action::RebaseFinished`].
    pub(crate) fn continue_rebase(&self, driver: RebaseDriver) {
        self.spawn(move |shell, repo, _| async move {
            let result = continue_rebase(&shell, &repo, driver)
                .await
                .map_err(|error| friendly_shell_error("continue rebase", &error));
            vec![Action::RebaseFinished { result }]
        });
    }

    /// Aborts an interrupted rebase, then sends [`Action::RebaseAborted`].
    pub(crate) fn abort_rebase(&self, driver: RebaseDriver) {
        self.spawn(move |shell, repo, _| async move {
            let result = abort_rebase(&shell, &repo, driver)
                .await
                .map_err(|error| friendly_shell_error("abort rebase", &error));
            vec![Action::RebaseAborted { result }]
        });
    }

    /// Asks git whether a rebase is stopped, sending
    /// [`Action::RebaseStateLoaded`].
    pub(crate) fn load_rebase_state(&self) {
        self.spawn(|shell, repo, _| async move {
            let result = interrupted_rebase(&shell, &repo)
                .await
                .map_err(|error| friendly_shell_error("check rebase state", &error));
            vec![Action::RebaseStateLoaded { result }]
        });
    }

    /// Stages a resolved file, sending [`Action::ConflictFileStaged`].
    pub(crate) fn stage_conflict_file(&self, path: String) {
        self.spawn(move |shell, repo, _| async move {
            let result = stage_resolved(&shell, &repo, &path)
                .await
                .map_err(|error| friendly_shell_error("mark resolved", &error));
            vec![Action::ConflictFileStaged { path, result }]
        });
    }

    /// Waits until every spawned task has finished and sent its results.
    #[cfg(test)]
    pub(crate) async fn wait_idle(&self) {
        while self.in_flight.load(Ordering::SeqCst) > 0 {
            tokio::task::yield_now().await;
        }
    }
}

fn layer_submit_status(event: SubmitEvent) -> LayerSubmitStatus {
    match event {
        SubmitEvent::Pushing => LayerSubmitStatus::InProgress,
        SubmitEvent::Pushed => LayerSubmitStatus::Pushed,
        SubmitEvent::Completed(SubmitLayerOutcome::PullRequestCreated { number }) => {
            LayerSubmitStatus::PullRequestCreated { number }
        }
        SubmitEvent::Completed(SubmitLayerOutcome::PullRequestUpdated { number }) => {
            LayerSubmitStatus::PullRequestUpdated { number }
        }
        SubmitEvent::PushFailed(error) => {
            LayerSubmitStatus::Failed(friendly_shell_error("push layer", &error))
        }
        SubmitEvent::PullRequestFailed(error) => {
            LayerSubmitStatus::Failed(friendly_shell_error("submit pull request", &error))
        }
    }
}
