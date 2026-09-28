use crate::stack::{Layer, StackSummary};
use crate::tui::state::{AppState, Screen};

pub fn layer(name: &str) -> Layer {
    Layer {
        branch: name.to_string(),
        head: None,
        base: "main".to_string(),
        is_current: false,
        is_merged: false,
        is_queued: false,
        needs_rebase: false,
        pull_request: None,
        commits: Vec::new(),
        position: 0,
    }
}

pub fn stack_summary(label: &str, layer_count: usize) -> StackSummary {
    StackSummary {
        label: label.to_string(),
        trunk: "main".to_string(),
        layers: (0..layer_count)
            .map(|index| layer(&format!("{label}-layer-{index}")))
            .collect(),
        is_current: false,
    }
}

pub fn app_state(stacks: Vec<StackSummary>, screen: Screen) -> AppState {
    AppState {
        stacks,
        screen,
        ..AppState::default()
    }
}
