//! The message type every part of the TUI communicates through.
//!
//! Components turn key presses into `Action`s, the reducer in
//! [`super::app`] applies them to [`super::state::AppState`], and
//! [`super::effects`] turns the ones that need a process into background
//! tasks whose results come back as further `Action`s.

use crate::stack::{LayerDetail, StackSummary};

use super::components::confirm::ConfirmModal;
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
}
