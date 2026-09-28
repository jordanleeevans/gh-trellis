//! Everything the reducer owns, shared read-only with components for drawing.

pub(crate) mod layer_resource;
pub(crate) mod submit_progress;

use crate::stack::{Layer, LayerDetail, StackSummary, SubmitOptions, SyncOptions};

use super::components::confirm::ConfirmModal;
use layer_resource::LayerResourceCache;
use submit_progress::SubmitProgress;

/// Which stack is currently selected in the unified browser.
#[derive(Debug, Clone, Copy)]
pub enum Screen {
    /// No stack is currently selected.
    List,
    /// The unified browser focused on the stack at this index into [`AppState::stacks`].
    Layers(usize),
}

pub struct AppState {
    pub stacks: Vec<StackSummary>,
    pub screen: Screen,
    pub status: Option<String>,
    pub error: Option<String>,
    pub refresh_in_flight: bool,
    pub refresh_spinner_frame: usize,
    pub refresh_request_id: u64,
    pub refresh_active_request_id: Option<u64>,
    pub last_successful_stacks: Vec<StackSummary>,
    pub layer_details: LayerResourceCache<LayerDetail>,
    pub layer_diffs: LayerResourceCache<String>,
    /// The confirmation modal currently shown on top of whatever screen is
    /// active, if any. Any destructive (or otherwise confirmation-worthy)
    /// action shows one via `Action::ShowConfirm` rather than rolling its
    /// own one-off modal.
    pub confirm: Option<ConfirmModal>,
    pub submit_progress: Option<SubmitProgress>,
    pub submit_options: SubmitOptions,
    pub sync_options: SyncOptions,
    /// A `gh stack sync` is running in the background.
    pub sync_in_flight: bool,
    /// Outcome of the last successful sync, shown in the footer until
    /// dismissed. (`status` is never rendered, so it can't carry this.)
    pub sync_notice: Option<String>,
    pub should_quit: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            stacks: Vec::new(),
            screen: Screen::List,
            status: None,
            error: None,
            refresh_in_flight: false,
            refresh_spinner_frame: 0,
            refresh_request_id: 0,
            refresh_active_request_id: None,
            last_successful_stacks: Vec::new(),
            layer_details: LayerResourceCache::default(),
            layer_diffs: LayerResourceCache::default(),
            confirm: None,
            submit_progress: None,
            submit_options: SubmitOptions::default(),
            sync_options: SyncOptions::default(),
            sync_in_flight: false,
            sync_notice: None,
            should_quit: false,
        }
    }
}

pub fn layer_detail_cache_key(stack: &StackSummary, layer: &Layer) -> String {
    format!("{}::{}", stack.label, layer.branch)
}

pub fn layer_diff_cache_key(stack: &StackSummary, layer: &Layer) -> String {
    format!("{}::{}::diff", stack.label, layer.branch)
}

pub fn lower_layer_ref(stack: &StackSummary, layer_index: usize) -> String {
    stack
        .layers
        .get(layer_index.saturating_sub(1))
        .filter(|_| layer_index > 0)
        .map(|layer| layer.branch.clone())
        .unwrap_or_else(|| stack.trunk.clone())
}
