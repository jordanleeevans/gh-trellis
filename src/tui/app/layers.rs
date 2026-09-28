//! Per-layer detail and diff loading, cached by layer.

use crate::tui::action::Action;
use crate::tui::effects::Effects;
use crate::tui::state::{Screen, layer_detail_cache_key, layer_diff_cache_key, lower_layer_ref};

use super::App;

impl App {
    pub(super) fn reduce_layers(&mut self, action: &Action, effects: &Effects) -> Vec<Action> {
        match action {
            Action::ShowLayers(index) => {
                if *index < self.state.stacks.len() {
                    self.state.screen = Screen::Layers(*index);
                    vec![
                        Action::LoadLayerDetail {
                            stack_index: *index,
                            layer_index: 0,
                            force: false,
                        },
                        Action::LoadLayerDiff {
                            stack_index: *index,
                            layer_index: 0,
                            force: false,
                        },
                    ]
                } else {
                    self.state.screen = Screen::List;
                    self.state.status = Some("selected stack is no longer available".to_string());
                    Vec::new()
                }
            }
            Action::LoadLayerDetail {
                stack_index,
                layer_index,
                force,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };
                let Some(layer) = stack.layers.get(*layer_index) else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return Vec::new();
                };

                let cache_key = layer_detail_cache_key(stack, layer);
                if self.state.layer_details.should_load(&cache_key, *force) {
                    let layer = layer.clone();
                    self.state.layer_details.mark_loading(cache_key.clone());
                    effects.load_detail(cache_key, layer);
                }
                Vec::new()
            }
            Action::LayerDetailLoaded { cache_key, result } => {
                self.state
                    .layer_details
                    .store_result(cache_key.clone(), result.clone());
                match result {
                    Err(error) => vec![Action::SetError(error.clone())],
                    Ok(_) => Vec::new(),
                }
            }
            Action::LoadLayerDiff {
                stack_index,
                layer_index,
                force,
            } => {
                let Some(stack) = self.state.stacks.get(*stack_index) else {
                    self.state.status = Some("selected stack is no longer available".to_string());
                    return Vec::new();
                };
                let Some(layer) = stack.layers.get(*layer_index) else {
                    self.state.status = Some("selected layer is no longer available".to_string());
                    return Vec::new();
                };

                let cache_key = layer_diff_cache_key(stack, layer);
                if self.state.layer_diffs.should_load(&cache_key, *force) {
                    let lower = lower_layer_ref(stack, *layer_index);
                    let branch = layer.branch.clone();
                    self.state.layer_diffs.mark_loading(cache_key.clone());
                    effects.load_diff(cache_key, lower, branch);
                }
                Vec::new()
            }
            Action::LayerDiffLoaded { cache_key, result } => {
                self.state
                    .layer_diffs
                    .store_result(cache_key.clone(), result.clone());
                match result {
                    Err(error) => vec![Action::SetError(error.clone())],
                    Ok(_) => Vec::new(),
                }
            }
            _ => unreachable!("routed to reduce_layers by App::apply_action"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::app::test_support::*;

    #[tokio::test]
    async fn show_layers_loads_first_layer_detail() {
        let shell = MockShell::new();
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let follow_ups = app.settle(&Action::ShowLayers(0), &shell).await;

        assert!(matches!(app.state.screen, Screen::Layers(0)));
        assert!(matches!(
            follow_ups.as_slice(),
            [
                Action::LoadLayerDetail {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                },
                Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                }
            ]
        ));
    }

    /// A failed load used to raise the error without recording it, leaving
    /// the cache entry stuck in `Loading` ("Loading diff..." forever).
    #[tokio::test]
    async fn failed_layer_diff_load_stops_loading_and_reports_error() {
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 1)];
        let key = layer_diff_cache_key(&app.state.stacks[0], &app.state.stacks[0].layers[0]);
        app.state.layer_diffs.mark_loading(key.clone());

        let follow_ups = app.apply(&Action::LayerDiffLoaded {
            cache_key: key.clone(),
            result: Err("load layer diff: boom".to_string()),
        });

        assert!(!app.state.layer_diffs.is_loading(&key));
        assert!(matches!(follow_ups.as_slice(), [Action::SetError(_)]));
    }

    #[tokio::test]
    async fn load_layer_diff_diffs_bottom_layer_against_trunk() {
        let shell = MockShell::new().when(
            "git",
            &[
                "diff",
                "--color=never",
                "--find-renames",
                "main..stack-a-layer-0",
            ],
            Ok(crate::shell::ShellOutput {
                stdout: "+bottom".to_string(),
                stderr: String::new(),
                exit_code: 0,
            }),
        );
        let mut app = App::new();
        app.state.stacks = vec![stack_summary("stack-a", 2)];

        let results = app
            .settle(
                &Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: false,
                },
                &shell,
            )
            .await;
        for action in &results {
            app.apply(action);
        }

        let key = layer_diff_cache_key(&app.state.stacks[0], &app.state.stacks[0].layers[0]);
        assert_eq!(app.state.layer_diffs.get(&key).unwrap(), "+bottom");
    }
}
