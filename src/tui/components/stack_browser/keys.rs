//! Maps key presses to [`Action`]s for whichever panel has focus.

use crossterm::event::{KeyCode, KeyEvent};

use crate::stack::UnstackScope;
use crate::tui::action::Action;
use crate::tui::components::add_layer_prompt::{AddLayerPrompt, PromptOutcome};
use crate::tui::keymap::{KeyIntent, key_intent};
use crate::tui::state::AppState;

use super::selection::{next_stack_action, previous_stack_action, selected_stack_index};
use super::{ActivePanel, StackBrowser};

impl StackBrowser {
    pub(super) fn handle_key_event(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action> {
        if self.add_layer_prompt.is_some() {
            return self.handle_add_layer_prompt_key(key);
        }

        if state.submit_progress.is_some() {
            return match key_intent(key) {
                Some(KeyIntent::Back) | Some(KeyIntent::DismissMessage) => {
                    vec![Action::DismissSubmit]
                }
                _ => Vec::new(),
            };
        }

        let selected_stack = selected_stack_index(state, self.stack_list_state.selected());
        let is_pending_g = self.pending_g;
        self.pending_g = false;

        if self.active_panel == ActivePanel::Diff {
            match key.code {
                KeyCode::Char('g') if is_pending_g => return vec![Action::ScrollDiffTop],
                KeyCode::Char('g') => {
                    self.pending_g = true;
                    return Vec::new();
                }
                _ => {}
            }
        }

        match key_intent(key) {
            Some(KeyIntent::Back) => vec![Action::Quit],
            Some(KeyIntent::FocusNext) => vec![Action::FocusNextPanel],
            Some(KeyIntent::FocusPrevious) => vec![Action::FocusPreviousPanel],
            Some(KeyIntent::MoveDown) => match self.active_panel {
                ActivePanel::Stacks => next_stack_action(state, selected_stack),
                ActivePanel::Layers => vec![Action::SelectNext],
                ActivePanel::Files => vec![Action::SelectNextDiffFile],
                ActivePanel::Diff => vec![Action::ScrollDiffLineDown],
                ActivePanel::Detail => Vec::new(),
            },
            Some(KeyIntent::MoveUp) => match self.active_panel {
                ActivePanel::Stacks => previous_stack_action(state, selected_stack),
                ActivePanel::Layers => vec![Action::SelectPrevious],
                ActivePanel::Files => vec![Action::SelectPreviousDiffFile],
                ActivePanel::Diff => vec![Action::ScrollDiffLineUp],
                ActivePanel::Detail => Vec::new(),
            },
            Some(KeyIntent::PageDown) => vec![Action::ScrollDiffDown],
            Some(KeyIntent::PageUp) => vec![Action::ScrollDiffUp],
            Some(KeyIntent::HalfPageDown) => vec![Action::ScrollDiffHalfPageDown],
            Some(KeyIntent::HalfPageUp) => vec![Action::ScrollDiffHalfPageUp],
            Some(KeyIntent::End) => vec![Action::ScrollDiffBottom],
            Some(KeyIntent::Refresh) => {
                if self.active_panel == ActivePanel::Stacks || selected_stack.is_none() {
                    vec![Action::RefreshStacks]
                } else {
                    self.list_state
                        .selected()
                        .map(|layer_index| {
                            let stack_index = selected_stack.unwrap_or_default();
                            vec![
                                Action::LoadLayerDetail {
                                    stack_index,
                                    layer_index,
                                    force: true,
                                },
                                Action::LoadLayerDiff {
                                    stack_index,
                                    layer_index,
                                    force: true,
                                },
                            ]
                        })
                        .unwrap_or_default()
                }
            }
            Some(KeyIntent::Checkout) => selected_stack
                .map(|stack_index| match self.active_panel {
                    ActivePanel::Stacks => vec![Action::CheckoutSelected {
                        stack_index,
                        layer_index: None,
                    }],
                    _ => self
                        .list_state
                        .selected()
                        .map(|layer_index| {
                            vec![Action::CheckoutSelected {
                                stack_index,
                                layer_index: Some(layer_index),
                            }]
                        })
                        .unwrap_or_default(),
                })
                .unwrap_or_default(),
            Some(KeyIntent::AddLayer) => {
                if let Some(stack_index) = selected_stack {
                    self.add_layer_prompt = Some(AddLayerPrompt::new(stack_index));
                }
                Vec::new()
            }
            Some(KeyIntent::Unstack) => selected_stack
                .map(|stack_index| {
                    vec![Action::UnstackSelected {
                        stack_index,
                        scope: UnstackScope::Local,
                    }]
                })
                .unwrap_or_default(),
            Some(KeyIntent::UnstackRemote) => selected_stack
                .map(|stack_index| {
                    vec![Action::UnstackSelected {
                        stack_index,
                        scope: UnstackScope::LocalAndRemote,
                    }]
                })
                .unwrap_or_default(),
            Some(KeyIntent::ToggleDiff) => vec![Action::ToggleDiffView],
            Some(KeyIntent::Submit) => selected_stack
                .map(|stack_index| vec![Action::SubmitStack { stack_index }])
                .unwrap_or_default(),
            Some(KeyIntent::ToggleSubmitAuto) => vec![Action::ToggleSubmitAuto],
            Some(KeyIntent::ToggleSubmitOpen) => vec![Action::ToggleSubmitOpen],
            Some(KeyIntent::Sync) => selected_stack
                .map(|stack_index| vec![Action::SyncStack { stack_index }])
                .unwrap_or_default(),
            Some(KeyIntent::ToggleSyncPrune) => vec![Action::ToggleSyncPrune],
            Some(KeyIntent::DrillIn) => match self.active_panel {
                ActivePanel::Stacks => vec![Action::FocusNextPanel],
                ActivePanel::Layers => vec![Action::FocusNextPanel],
                ActivePanel::Detail => vec![Action::FocusNextPanel],
                ActivePanel::Files => vec![Action::ToggleDiffView],
                ActivePanel::Diff => Vec::new(),
            },
            Some(KeyIntent::OpenExternal) => selected_stack
                .and_then(|stack_index| {
                    self.list_state.selected().map(|layer_index| {
                        vec![Action::OpenPullRequest {
                            stack_index,
                            layer_index,
                        }]
                    })
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        }
    }

    /// Routes a key event to the add-layer prompt while it's open.
    fn handle_add_layer_prompt_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(prompt) = self.add_layer_prompt.as_mut() else {
            return Vec::new();
        };

        match prompt.handle_key(key) {
            PromptOutcome::Editing => Vec::new(),
            PromptOutcome::Cancelled => {
                self.add_layer_prompt = None;
                Vec::new()
            }
            PromptOutcome::Submitted(action) => {
                self.add_layer_prompt = None;
                vec![action]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::UnstackScope;
    use crate::test_fixtures::{app_state, stack_summary};
    use crate::tui::component::Component;
    use crate::tui::components::add_layer_prompt::AddLayerField;
    use crate::tui::components::stack_browser::test_support::*;
    use crate::tui::state::Screen;
    use crate::tui::state::layer_diff_cache_key;
    use crate::tui::state::submit_progress::SubmitProgress;

    #[test]
    fn handle_key_maps_navigation_actions() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let quit = component.handle_key(key(KeyCode::Char('q')), &state);
        let next = component.handle_key(key(KeyCode::Down), &state);
        let previous = component.handle_key(key(KeyCode::Up), &state);
        let refresh = component.handle_key(key(KeyCode::Char('r')), &state);

        assert!(matches!(quit.as_slice(), [Action::Quit]));
        assert!(matches!(next.as_slice(), [Action::SelectNext]));
        assert!(matches!(previous.as_slice(), [Action::SelectPrevious]));
        assert!(matches!(
            refresh.as_slice(),
            [
                Action::LoadLayerDetail {
                    stack_index: 0,
                    layer_index: 0,
                    force: true,
                },
                Action::LoadLayerDiff {
                    stack_index: 0,
                    layer_index: 0,
                    force: true,
                }
            ]
        ));
    }

    #[test]
    fn handle_key_dispatches_open_pr_for_selected_layer() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let actions = component.handle_key(key(KeyCode::Char('o')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::OpenPullRequest {
                stack_index: 0,
                layer_index: 1
            }]
        ));
    }

    #[test]
    fn handle_key_dispatches_checkout_for_selected_layer() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let actions = component.handle_key(key(KeyCode::Char('c')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::CheckoutSelected {
                stack_index: 0,
                layer_index: Some(1)
            }]
        ));
    }

    #[test]
    fn handle_key_dispatches_checkout_for_selected_stack_from_stack_panel() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('c')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::CheckoutSelected {
                stack_index: 0,
                layer_index: None
            }]
        ));
    }

    #[test]
    fn a_opens_add_layer_prompt_on_branch_field() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('a')), &state);

        assert!(actions.is_empty());
        let prompt = component.add_layer_prompt.as_ref().expect("prompt open");
        assert_eq!(prompt.stack_index, 0);
        assert_eq!(prompt.field, AddLayerField::Branch);
    }

    #[test]
    fn add_layer_prompt_types_into_branch_then_advances_on_enter() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);

        // Global shortcut letters are captured as text, not intents, while
        // the prompt is open.
        component.handle_key(key(KeyCode::Char('c')), &state);
        component.handle_key(key(KeyCode::Char('q')), &state);
        assert_eq!(component.add_layer_prompt.as_ref().unwrap().branch, "cq");

        // Enter with an empty branch is a no-op.
        let mut empty_component = StackBrowser::new();
        empty_component.update(&Action::ShowLayers(0), &mut state);
        empty_component.handle_key(key(KeyCode::Char('a')), &state);
        empty_component.handle_key(key(KeyCode::Enter), &state);
        assert_eq!(
            empty_component.add_layer_prompt.as_ref().unwrap().field,
            AddLayerField::Branch
        );

        let advance = component.handle_key(key(KeyCode::Enter), &state);
        assert!(advance.is_empty());
        assert_eq!(
            component.add_layer_prompt.as_ref().unwrap().field,
            AddLayerField::Message
        );
    }

    #[test]
    fn add_layer_prompt_backspace_removes_last_character() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        component.handle_key(key(KeyCode::Char('x')), &state);
        component.handle_key(key(KeyCode::Char('y')), &state);

        component.handle_key(key(KeyCode::Backspace), &state);

        assert_eq!(component.add_layer_prompt.as_ref().unwrap().branch, "x");
    }

    #[test]
    fn escape_cancels_add_layer_prompt_without_dispatching_actions() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        component.handle_key(key(KeyCode::Char('x')), &state);

        let actions = component.handle_key(key(KeyCode::Esc), &state);

        assert!(actions.is_empty());
        assert!(component.add_layer_prompt.is_none());
    }

    #[test]
    fn enter_on_message_field_submits_add_layer_with_message_and_closes_prompt() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        for c in "feature/new".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }
        component.handle_key(key(KeyCode::Enter), &state);
        for c in "add a layer".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }

        let actions = component.handle_key(key(KeyCode::Enter), &state);

        assert!(component.add_layer_prompt.is_none());
        assert!(matches!(
            actions.as_slice(),
            [Action::AddLayer {
                stack_index: 0,
                branch,
                message: Some(message),
            }] if branch == "feature/new" && message == "add a layer"
        ));
    }

    #[test]
    fn enter_on_blank_message_field_submits_add_layer_without_message() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);
        for c in "feature/new".chars() {
            component.handle_key(key(KeyCode::Char(c)), &state);
        }
        component.handle_key(key(KeyCode::Enter), &state);

        let actions = component.handle_key(key(KeyCode::Enter), &state);

        assert!(component.add_layer_prompt.is_none());
        assert!(matches!(
            actions.as_slice(),
            [Action::AddLayer {
                stack_index: 0,
                branch,
                message: None,
            }] if branch == "feature/new"
        ));
    }

    #[test]
    fn d_toggles_diff_and_jk_scroll_it() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let toggle = component.handle_key(key(KeyCode::Char('d')), &state);
        assert!(matches!(toggle.as_slice(), [Action::ToggleDiffView]));
        component.update(&toggle[0], &mut state);

        let diff_down = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(diff_down.as_slice(), [Action::ScrollDiffLineDown]));
        component.update(&diff_down[0], &mut state);
        assert_eq!(component.diff_scroll, 1);
    }

    #[test]
    fn stack_panel_navigation_switches_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(
            vec![stack_summary("a", 1), stack_summary("b", 1)],
            Screen::Layers(0),
        );
        component.update(&Action::ShowLayers(0), &mut state);

        let next = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(next.as_slice(), [Action::ShowLayers(1)]));

        state.screen = Screen::Layers(1);
        component.update(&next[0], &mut state);
        assert_eq!(component.stack_list_state.selected(), Some(1));

        let refresh = component.handle_key(key(KeyCode::Char('r')), &state);
        assert!(matches!(refresh.as_slice(), [Action::RefreshStacks]));
    }

    #[test]
    fn enter_advances_focus_and_space_opens_diff_from_files() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let stack = state.stacks[0].clone();
        let layer = stack.layers[0].clone();
        state.layer_diffs.store_result(
            layer_diff_cache_key(&stack, &layer),
            Ok(sample_diff_with_paths()),
        );
        component.update(&Action::ShowLayers(0), &mut state);

        let stack_open = component.handle_key(key(KeyCode::Enter), &state);
        assert!(matches!(stack_open.as_slice(), [Action::FocusNextPanel]));
        component.update(&stack_open[0], &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        assert_eq!(component.active_panel, ActivePanel::Files);

        let open_diff = component.handle_key(key(KeyCode::Char(' ')), &state);
        assert!(matches!(open_diff.as_slice(), [Action::ToggleDiffView]));
    }

    #[test]
    fn files_panel_navigation_changes_selected_file() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        let stack = state.stacks[0].clone();
        let layer = stack.layers[0].clone();
        state.layer_diffs.store_result(
            layer_diff_cache_key(&stack, &layer),
            Ok(sample_diff_with_paths()),
        );
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);
        component.update(&Action::FocusNextPanel, &mut state);

        let next = component.handle_key(key(KeyCode::Down), &state);
        assert!(matches!(next.as_slice(), [Action::SelectNextDiffFile]));
        component.update(&next[0], &mut state);
        assert_eq!(component.selected_diff_file, 1);
    }

    #[test]
    fn diff_panel_supports_vim_navigation() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::ToggleDiffView, &mut state);

        let first_g = component.handle_key(key(KeyCode::Char('g')), &state);
        assert!(first_g.is_empty());
        let second_g = component.handle_key(key(KeyCode::Char('g')), &state);
        assert!(matches!(second_g.as_slice(), [Action::ScrollDiffTop]));
        let end = component.handle_key(key(KeyCode::Char('G')), &state);
        assert!(matches!(end.as_slice(), [Action::ScrollDiffBottom]));
        let half_down = component.handle_key(ctrl_key(KeyCode::Char('d')), &state);
        assert!(matches!(
            half_down.as_slice(),
            [Action::ScrollDiffHalfPageDown]
        ));
        let half_up = component.handle_key(ctrl_key(KeyCode::Char('u')), &state);
        assert!(matches!(half_up.as_slice(), [Action::ScrollDiffHalfPageUp]));
    }

    #[test]
    fn handle_key_dispatches_submit_for_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('s')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::SubmitStack { stack_index: 0 }]
        ));
    }

    #[test]
    fn handle_key_dispatches_unstack_scopes_for_selected_stack() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let local = component.handle_key(key(KeyCode::Char('D')), &state);
        assert!(matches!(
            local.as_slice(),
            [Action::UnstackSelected {
                stack_index: 0,
                scope: UnstackScope::Local
            }]
        ));
        let remote = component.handle_key(key(KeyCode::Char('U')), &state);
        assert!(matches!(
            remote.as_slice(),
            [Action::UnstackSelected {
                stack_index: 0,
                scope: UnstackScope::LocalAndRemote
            }]
        ));
    }

    #[test]
    fn handle_key_toggles_submit_auto_and_open() {
        let mut component = StackBrowser::new();
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));

        let auto = component.handle_key(key(KeyCode::Char('t')), &state);
        assert!(matches!(auto.as_slice(), [Action::ToggleSubmitAuto]));

        let open = component.handle_key(key(KeyCode::Char('p')), &state);
        assert!(matches!(open.as_slice(), [Action::ToggleSubmitOpen]));
    }

    #[test]
    fn handle_key_dispatches_sync_for_selected_stack_and_toggles_prune() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let sync = component.handle_key(key(KeyCode::Char('S')), &state);
        assert!(matches!(
            sync.as_slice(),
            [Action::SyncStack { stack_index: 0 }]
        ));

        let prune = component.handle_key(key(KeyCode::Char('P')), &state);
        assert!(matches!(prune.as_slice(), [Action::ToggleSyncPrune]));
    }

    #[test]
    fn handle_key_while_submitting_only_allows_dismissal() {
        let mut component = StackBrowser::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        state.submit_progress = Some(SubmitProgress::new(
            0,
            vec!["a-layer-0".to_string(), "a-layer-1".to_string()],
        ));

        let navigate = component.handle_key(key(KeyCode::Down), &state);
        assert!(navigate.is_empty());

        let dismiss = component.handle_key(key(KeyCode::Esc), &state);
        assert!(matches!(dismiss.as_slice(), [Action::DismissSubmit]));

        let dismiss_q = component.handle_key(key(KeyCode::Char('q')), &state);
        assert!(matches!(dismiss_q.as_slice(), [Action::DismissSubmit]));
    }
}
