//! Renders the real browser into a `TestBackend` at several terminal sizes.

use ratatui::Terminal;
use ratatui::backend::TestBackend;

use super::{ActivePanel, StackBrowser};
use crate::stack::PullRequestRef;
use crate::test_fixtures::{app_state, stack_summary};
use crate::tui::action::Action;
use crate::tui::component::Component;
use crate::tui::components::confirm::{self, ConfirmModal};
use crate::tui::state::submit_progress::SubmitProgress;
use crate::tui::state::{AppState, Screen, layer_diff_cache_key};

const SIZES: [(u16, u16); 4] = [(60, 20), (80, 24), (100, 30), (160, 45)];
const PANELS: [ActivePanel; 5] = [
    ActivePanel::Stacks,
    ActivePanel::Layers,
    ActivePanel::Detail,
    ActivePanel::Files,
    ActivePanel::Diff,
];

fn sample_state(layers: usize) -> AppState {
    let mut stack = stack_summary("feature", layers);
    for (index, layer) in stack.layers.iter_mut().enumerate() {
        layer.pull_request = Some(PullRequestRef {
            number: 100 + u64::try_from(index).unwrap(),
            url: String::new(),
            state: "OPEN".to_string(),
            title: Some("A pull request".to_string()),
            is_draft: None,
            checks_status: None,
            review_decision: None,
        });
    }
    let mut state = app_state(vec![stack], Screen::Layers(0));
    let stack = &state.stacks[0];
    for layer in &stack.layers {
        let key = layer_diff_cache_key(stack, layer);
        state
            .layer_diffs
            .store_result(key, Ok(super::test_support::sample_diff_with_paths()));
    }
    state
}

fn browser(state: &mut AppState, panel: ActivePanel) -> StackBrowser {
    let mut browser = StackBrowser::new();
    browser.update(&Action::ShowLayers(0), state);
    browser.active_panel = panel;
    browser
}

fn draw(
    terminal: &mut Terminal<TestBackend>,
    browser: &mut StackBrowser,
    state: &AppState,
) -> String {
    terminal.draw(|frame| browser.draw(frame, state)).unwrap();
    screen_text(terminal)
}

fn screen_text(terminal: &Terminal<TestBackend>) -> String {
    let buffer = terminal.backend().buffer();
    let width = usize::from(buffer.area.width);
    buffer
        .content()
        .chunks(width.max(1))
        .map(|row| row.iter().map(|cell| cell.symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Text that only shows while `panel` is the focused (and so visible) one.
fn focused_content(panel: ActivePanel) -> &'static str {
    match panel {
        ActivePanel::Stacks => "trunk: main",
        ActivePanel::Layers => "feature-layer-1",
        ActivePanel::Detail => "A pull request",
        ActivePanel::Files => "changed files",
        ActivePanel::Diff => "src/a.rs",
    }
}

#[test]
fn every_panel_is_readable_at_every_size() {
    for (width, height) in SIZES {
        for panel in PANELS {
            let mut state = sample_state(3);
            let mut browser = browser(&mut state, panel);
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            let screen = draw(&mut terminal, &mut browser, &state);
            let context = format!("{panel:?} at {width}x{height}:\n{screen}");
            assert!(screen.contains(focused_content(panel)), "{context}");
            assert!(screen.contains("? help"), "{context}");
            assert!(screen.contains("q quit"), "{context}");
        }
    }
}

#[test]
fn narrow_terminal_shows_one_column_and_says_which() {
    let mut state = sample_state(3);
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();

    let mut nav = browser(&mut state, ActivePanel::Layers);
    let screen = draw(&mut terminal, &mut nav, &state);
    assert!(screen.contains("1/2 navigator"), "{screen}");
    assert!(!screen.contains("changed files"), "{screen}");

    let mut files = browser(&mut state, ActivePanel::Files);
    let screen = draw(&mut terminal, &mut files, &state);
    assert!(screen.contains("2/2 details"), "{screen}");
    assert!(screen.contains("changed files"), "{screen}");
    assert!(!screen.contains("navigator"), "{screen}");
}

#[test]
fn wide_terminal_shows_both_columns_without_a_view_label() {
    let mut state = sample_state(3);
    let mut browser = browser(&mut state, ActivePanel::Layers);
    let mut terminal = Terminal::new(TestBackend::new(100, 30)).unwrap();
    let screen = draw(&mut terminal, &mut browser, &state);
    assert!(screen.contains("navigator"), "{screen}");
    assert!(screen.contains("changed files"), "{screen}");
    assert!(!screen.contains("1/2"), "{screen}");
}

#[test]
fn resizing_between_frames_re_lays_out() {
    let mut state = sample_state(3);
    let mut browser = browser(&mut state, ActivePanel::Layers);
    let mut terminal = Terminal::new(TestBackend::new(160, 45)).unwrap();

    let wide = draw(&mut terminal, &mut browser, &state);
    assert!(wide.contains("navigator") && wide.contains("changed files"));

    terminal.backend_mut().resize(60, 20);
    let narrow = draw(&mut terminal, &mut browser, &state);
    assert!(narrow.contains("1/2 navigator"), "{narrow}");
    assert!(!narrow.contains("changed files"), "{narrow}");
    assert!(narrow.contains("? help") && narrow.contains("q quit"));

    terminal.backend_mut().resize(120, 40);
    let wide_again = draw(&mut terminal, &mut browser, &state);
    assert!(wide_again.contains("navigator") && wide_again.contains("changed files"));
    assert!(!wide_again.contains("1/2"));
}

#[test]
fn navigator_scrolls_to_keep_the_selected_layer_visible() {
    let mut state = sample_state(15);
    let mut browser = browser(&mut state, ActivePanel::Layers);
    browser.list_state.select(Some(13));
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let screen = draw(&mut terminal, &mut browser, &state);
    assert!(screen.contains("feature-layer-13"), "{screen}");
}

#[test]
fn empty_repository_renders_at_small_sizes() {
    for panel in PANELS {
        let mut state = app_state(Vec::new(), Screen::List);
        let mut browser = StackBrowser::new();
        browser.active_panel = panel;
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        let screen = draw(&mut terminal, &mut browser, &state);
        assert!(screen.contains("? help"), "{screen}");
        state.error = None;
    }
}

#[test]
fn tiny_terminals_do_not_panic() {
    for (width, height) in [(1, 1), (10, 3), (20, 6), (30, 8)] {
        for panel in PANELS {
            let mut state = sample_state(3);
            let mut browser = browser(&mut state, panel);
            browser.show_help = true;
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            draw(&mut terminal, &mut browser, &state);
        }
    }
}

#[test]
fn help_overlay_is_readable_at_60x20() {
    let mut state = sample_state(3);
    let mut browser = browser(&mut state, ActivePanel::Layers);
    browser.show_help = true;
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let screen = draw(&mut terminal, &mut browser, &state);
    assert!(screen.contains("keybindings"), "{screen}");
    assert!(screen.contains("Navigation"), "{screen}");
}

#[test]
fn add_layer_prompt_is_readable_at_60x20() {
    let mut state = sample_state(3);
    let mut browser = browser(&mut state, ActivePanel::Layers);
    browser.add_layer_prompt = Some(super::AddLayerPrompt::new(0));
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let screen = draw(&mut terminal, &mut browser, &state);
    assert!(screen.contains("add layer"), "{screen}");
    assert!(screen.contains("New branch name"), "{screen}");
    assert!(screen.contains("enter continue  esc cancel"), "{screen}");
}

#[test]
fn submit_progress_is_readable_at_60x20() {
    let mut state = sample_state(3);
    state.submit_progress = Some(SubmitProgress::new(
        0,
        vec!["feature-layer-0".into(), "feature-layer-1".into()],
    ));
    let mut browser = browser(&mut state, ActivePanel::Layers);
    let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
    let screen = draw(&mut terminal, &mut browser, &state);
    assert!(screen.contains("submitting stack"), "{screen}");
    assert!(screen.contains("0/2 layers"), "{screen}");
    assert!(screen.contains("feature-layer-1"), "{screen}");
    assert!(screen.contains("please wait"), "{screen}");
}

#[test]
fn confirm_modal_is_readable_at_60x20() {
    for danger in [false, true] {
        let modal = ConfirmModal::new(
            "Unstack",
            "Remove the stack from GitHub?",
            "Unstack",
            danger,
        );
        let mut terminal = Terminal::new(TestBackend::new(60, 20)).unwrap();
        terminal
            .draw(|frame| confirm::render(frame, frame.area(), &modal))
            .unwrap();
        let screen = screen_text(&terminal);
        assert!(screen.contains("Unstack"), "{screen}");
        assert!(screen.contains("Remove the stack from GitHub?"), "{screen}");
    }
}
