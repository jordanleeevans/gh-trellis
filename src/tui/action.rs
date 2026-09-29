//! The message type every part of the TUI communicates through.
//!
//! Components turn key presses into `Action`s, the reducer in
//! [`super::app`] applies them to [`super::state::AppState`], and
//! [`super::effects`] turns the ones that need a process into background
//! tasks whose results come back as further `Action`s.

use crate::stack::{
    LayerDetail, MergeMethod, MergeOutcome, RebaseConflict, RebaseOutcome, RebaseScope,
    StackSummary, StageOutcome, SyncOutcome, UnstackScope,
};

use super::components::confirm::ConfirmModal;
use super::state::external_command::ExternalCommand;
use super::state::submit_progress::LayerSubmitStatus;

#[derive(Debug, Clone)]
pub enum Action {
    Quit,
    RefreshStacks,
    SelectNext,
    SelectPrevious,
    FocusNextPanel,
    FocusPreviousPanel,
    SelectNextDiffFile,
    SelectPreviousDiffFile,
    ScrollDiffLineDown,
    ScrollDiffLineUp,
    ScrollDiffDown,
    ScrollDiffUp,
    ScrollDiffHalfPageDown,
    ScrollDiffHalfPageUp,
    ScrollDiffTop,
    ScrollDiffBottom,
    ToggleDiffView,
    ShowLayers(usize),
    CheckoutSelected {
        stack_index: usize,
        layer_index: Option<usize>,
    },
    AddLayer {
        stack_index: usize,
        branch: String,
        message: Option<String>,
    },
    /// User intent: unstack the stack at `stack_index`. Shows a confirm modal
    /// whose confirmation dispatches [`Action::RunUnstack`].
    UnstackSelected {
        stack_index: usize,
        scope: UnstackScope,
    },
    /// Confirmed unstack; runs `gh stack unstack` via effects.
    RunUnstack {
        stack_index: usize,
        scope: UnstackScope,
    },
    OpenPullRequest {
        stack_index: usize,
        layer_index: usize,
    },
    LoadLayerDetail {
        stack_index: usize,
        layer_index: usize,
        force: bool,
    },
    LoadLayerDiff {
        stack_index: usize,
        layer_index: usize,
        force: bool,
    },
    LayerDetailLoaded {
        cache_key: String,
        result: Result<LayerDetail, String>,
    },
    LayerDiffLoaded {
        cache_key: String,
        result: Result<String, String>,
    },
    StackRefreshStarted {
        request_id: u64,
    },
    StackRefreshSucceeded {
        request_id: u64,
        result: Result<Vec<StackSummary>, String>,
    },
    Tick,
    StacksLoaded(Option<usize>),
    SetError(String),
    ClearError,
    ClearStatus,
    /// Shows a confirmation modal (see [`crate::tui::confirm::ConfirmModal`]),
    /// replacing any modal already shown.
    ShowConfirm(ConfirmModal),
    /// Accepts the currently shown confirm modal, dispatching its attached
    /// action if [`ConfirmModal::can_confirm`] allows it, or leaving it open
    /// (waiting for the danger phrase to be completed) otherwise.
    ConfirmAccept,
    /// Dismisses the currently shown confirm modal without dispatching its
    /// attached action.
    ConfirmCancel,
    /// Appends a typed character to a `danger` confirm modal's input.
    ConfirmInput(char),
    /// Removes the last typed character from a `danger` confirm modal's
    /// input.
    ConfirmBackspace,
    SubmitStack {
        stack_index: usize,
    },
    ToggleSubmitAuto,
    ToggleSubmitOpen,
    SubmitStarted {
        stack_index: usize,
    },
    SubmitLayerProgress {
        stack_index: usize,
        layer_index: usize,
        status: LayerSubmitStatus,
    },
    SubmitFinished {
        stack_index: usize,
    },
    DismissSubmit,
    /// The user's intent to sync a stack; shows the confirmation modal.
    SyncStack {
        stack_index: usize,
    },
    ToggleSyncPrune,
    /// The user's intent to restructure a stack with `gh stack modify`;
    /// shows the confirmation modal.
    ModifyStack {
        stack_index: usize,
    },
    /// Modify was confirmed: hands the terminal to `gh stack modify`.
    ModifyStarted {
        stack_index: usize,
    },
    /// Sync was confirmed: starts `gh stack sync` in the background.
    SyncStarted {
        stack_index: usize,
    },
    SyncFinished {
        /// The error is already user-facing (see `friendly_shell_error`).
        result: Result<SyncOutcome, String>,
    },
    /// The user's intent to merge a stack; shows the danger confirmation
    /// naming every PR that will merge, or refuses.
    MergeStack {
        stack_index: usize,
    },
    /// Cycles the merge method (merge, squash, rebase).
    CycleMergeMethod,
    /// The merge was confirmed for exactly `prs` (bottom to top) with
    /// `method`. The reducer re-checks both against current state before
    /// running `gh stack merge`.
    MergeStarted {
        stack_index: usize,
        prs: Vec<u64>,
        method: MergeMethod,
    },
    MergeFinished {
        /// The error is already user-facing (see `tui::messages`).
        result: Result<MergeOutcome, String>,
    },
    /// User intent: rebase the stack at `stack_index`, which must be
    /// checked out. Shows a confirm modal whose confirmation dispatches
    /// [`Action::RebaseStarted`].
    RebaseStack {
        stack_index: usize,
        scope: RebaseScope,
    },
    /// Rebase was confirmed: starts `gh stack rebase` in the background.
    RebaseStarted {
        stack_index: usize,
        scope: RebaseScope,
    },
    /// A rebase or `--continue` ended. Stopping on a conflict is an `Ok`
    /// outcome; the error is already user-facing.
    RebaseFinished {
        result: Result<RebaseOutcome, String>,
    },
    /// Asks git whether a rebase is stopped: on startup, on refresh, and
    /// after anything that may have changed the conflicted files.
    LoadRebaseState,
    RebaseStateLoaded {
        result: Result<Option<RebaseConflict>, String>,
    },
    /// Opens a conflicted file in `$EDITOR`, suspending the TUI.
    EditConflictFile {
        path: String,
    },
    /// Marks a conflicted file resolved (`git add`).
    StageConflictFile {
        path: String,
    },
    ConflictFileStaged {
        path: String,
        result: Result<StageOutcome, String>,
    },
    /// Continues the interrupted rebase once every file is resolved.
    ContinueRebase,
    /// User intent: abort the interrupted rebase. Shows a confirm modal
    /// whose confirmation dispatches [`Action::RunAbortRebase`].
    AbortRebase,
    RunAbortRebase,
    RebaseAborted {
        result: Result<(), String>,
    },
    /// Hands the terminal to another program; see [`ExternalCommand`].
    RunExternal(ExternalCommand),
    /// Sent by the event loop once an external command has exited and the
    /// TUI is back. The error is already user-facing; `then` is dispatched
    /// either way.
    ExternalCommandFinished {
        result: Result<(), String>,
        then: Box<Action>,
    },
}
