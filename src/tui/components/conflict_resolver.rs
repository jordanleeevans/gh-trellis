//! The conflict view (#23): shown instead of the stack browser while a
//! rebase is stopped on a conflict, whether trellis started it or it was
//! left over from an earlier session.
//!
//! It lists the conflicted files and offers, for the selected file: open it
//! in `$EDITOR` (the TUI is suspended while the editor runs, then the list
//! is reloaded) and mark it resolved (`git add`); and for the rebase:
//! continue once no files are left, or abort (confirmed first). Every key
//! is a [`KeyIntent`], so the footer, the guidance text and the `?`
//! overlay show the user's own bindings.

use crossterm::event::KeyEvent;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, List, ListItem, ListState, Paragraph, Wrap};

use crate::stack::{ConflictedFile, RebaseConflict, RebaseDriver};
use crate::theme::ui::THEME;
use crate::tui::action::Action;
use crate::tui::component::Component;
use crate::tui::components::help;
use crate::tui::components::stack_browser::{Hint, render_hinted_footer};
use crate::tui::keymap::{self, KeyIntent, Keymap, key_intent};
use crate::tui::state::AppState;
use crate::tui::widgets::{glyphs, panel_block};

/// Footer hints, most useful first.
const HINTS: &[Hint] = &[
    (&[KeyIntent::MoveDown, KeyIntent::MoveUp], "files"),
    (&[KeyIntent::ConflictEdit], "edit"),
    (&[KeyIntent::ConflictMarkResolved], "mark resolved"),
    (&[KeyIntent::RebaseContinue], "continue"),
    (&[KeyIntent::RebaseAbort], "abort"),
    (&[KeyIntent::Refresh], "reload"),
];

pub struct ConflictResolver {
    selected: usize,
    show_help: bool,
    help_scroll: u16,
}

impl ConflictResolver {
    pub fn new() -> Self {
        Self {
            selected: 0,
            show_help: false,
            help_scroll: 0,
        }
    }

    fn selected_path<'a>(&self, conflict: &'a RebaseConflict) -> Option<&'a str> {
        conflict
            .files
            .get(self.selected)
            .map(|file| file.path.as_str())
    }
}

impl Component for ConflictResolver {
    fn draw(&mut self, frame: &mut Frame, state: &AppState) {
        let Some(conflict) = &state.conflict else {
            return;
        };
        let [header, content, footer] = Layout::vertical([
            Constraint::Length(3),
            Constraint::Min(0),
            Constraint::Length(2),
        ])
        .areas(frame.area());

        render_header(frame, header, conflict);
        let [files, guide] =
            Layout::horizontal([Constraint::Percentage(50), Constraint::Percentage(50)])
                .areas(content);
        render_files(frame, files, conflict, self.selected);
        frame.render_widget(
            Paragraph::new(guidance_lines(conflict, keymap::current()))
                .wrap(Wrap { trim: false })
                .block(panel_block("resolve", false)),
            guide,
        );
        render_hinted_footer(frame, footer, state, HINTS);

        if self.show_help {
            let max_scroll = help::render(
                frame,
                frame.area(),
                keymap::current(),
                state,
                self.help_scroll,
            );
            self.help_scroll = self.help_scroll.min(max_scroll);
        }
    }

    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action> {
        if self.show_help {
            match key_intent(key) {
                Some(KeyIntent::Help | KeyIntent::Back | KeyIntent::DismissMessage) => {
                    self.show_help = false;
                    self.help_scroll = 0;
                }
                Some(KeyIntent::MoveDown) => self.help_scroll = self.help_scroll.saturating_add(1),
                Some(KeyIntent::MoveUp) => self.help_scroll = self.help_scroll.saturating_sub(1),
                _ => {}
            }
            return Vec::new();
        }

        let Some(conflict) = &state.conflict else {
            return Vec::new();
        };
        let selected = || self.selected_path(conflict).map(str::to_string);

        match key_intent(key) {
            Some(KeyIntent::Back) => vec![Action::Quit],
            Some(KeyIntent::Help) => {
                self.show_help = true;
                Vec::new()
            }
            Some(KeyIntent::MoveDown) => {
                self.selected = (self.selected + 1).min(conflict.files.len().saturating_sub(1));
                Vec::new()
            }
            Some(KeyIntent::MoveUp) => {
                self.selected = self.selected.saturating_sub(1);
                Vec::new()
            }
            Some(KeyIntent::ConflictEdit) => selected()
                .map(|path| vec![Action::EditConflictFile { path }])
                .unwrap_or_default(),
            Some(KeyIntent::ConflictMarkResolved) => selected()
                .map(|path| vec![Action::StageConflictFile { path }])
                .unwrap_or_default(),
            Some(KeyIntent::RebaseContinue) => vec![Action::ContinueRebase],
            Some(KeyIntent::RebaseAbort) => vec![Action::AbortRebase],
            Some(KeyIntent::Refresh) => vec![Action::LoadRebaseState],
            _ => Vec::new(),
        }
    }

    fn update(&mut self, _action: &Action, state: &mut AppState) {
        match &state.conflict {
            Some(conflict) => {
                self.selected = self.selected.min(conflict.files.len().saturating_sub(1));
            }
            None => {
                self.selected = 0;
                self.show_help = false;
                self.help_scroll = 0;
            }
        }
    }
}

fn render_header(frame: &mut Frame, area: Rect, conflict: &RebaseConflict) {
    let block = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Trellis ",
                Style::default()
                    .fg(THEME.colors.text_inverse)
                    .bg(THEME.colors.primary),
            ),
            Span::styled(
                " rebase conflict",
                THEME.text.heading.fg(THEME.colors.danger),
            ),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(THEME.colors.danger));

    let branch = conflict.branch.as_deref().unwrap_or("a branch");
    let by = match conflict.driver {
        RebaseDriver::GhStack => "gh stack rebase",
        RebaseDriver::Git => "git rebase",
    };
    let text = format!(
        "{} Rebase of {branch} stopped ({by}) - {} file(s) conflicted",
        glyphs().warning,
        conflict.files.len()
    );
    frame.render_widget(
        Paragraph::new(text)
            .style(
                Style::default()
                    .fg(THEME.colors.warning)
                    .add_modifier(Modifier::BOLD),
            )
            .block(block),
        area,
    );
}

fn render_files(frame: &mut Frame, area: Rect, conflict: &RebaseConflict, selected: usize) {
    let block = panel_block("conflicted files", true);
    if conflict.files.is_empty() {
        frame.render_widget(
            Paragraph::new("No conflicted files left.")
                .style(THEME.text.muted)
                .block(block),
            area,
        );
        return;
    }

    let items: Vec<ListItem> = conflict
        .files
        .iter()
        .map(|file| {
            let (note, style) = file_note(file);
            ListItem::new(Line::from(vec![
                Span::styled(file.path.clone(), THEME.text.body),
                Span::raw("  "),
                Span::styled(note, style),
            ]))
        })
        .collect();
    let mut list_state = ListState::default().with_selected(Some(selected));
    frame.render_stateful_widget(
        List::new(items)
            .block(block)
            .highlight_style(THEME.text.selected)
            .highlight_symbol("> "),
        area,
        &mut list_state,
    );
}

/// Whether `file` still needs editing or only needs marking resolved.
fn file_note(file: &ConflictedFile) -> (&'static str, Style) {
    if file.has_markers {
        ("conflict markers", Style::default().fg(THEME.colors.danger))
    } else {
        (
            "no markers left, ready to mark resolved",
            Style::default().fg(THEME.colors.success),
        )
    }
}

/// What to do next, with the user's key labels.
fn guidance_lines(conflict: &RebaseConflict, keymap: &Keymap) -> Vec<Line<'static>> {
    let key = |intent| {
        keymap
            .short_label(intent)
            .unwrap_or_else(|| "unbound".into())
    };
    let step = |keys: String, text: String| {
        Line::from(vec![
            Span::styled(format!("{keys:>3}  "), THEME.text.key),
            Span::raw(text),
        ])
    };

    let mut lines = Vec::new();
    if conflict.files.is_empty() {
        lines.push(Line::from(Span::styled(
            "All conflicts are resolved.",
            THEME.text.heading,
        )));
        lines.push(step(
            key(KeyIntent::RebaseContinue),
            "continue the rebase".to_string(),
        ));
    } else {
        lines.push(Line::from(Span::styled(
            "For each conflicted file:",
            THEME.text.heading,
        )));
        lines.push(step(
            key(KeyIntent::ConflictEdit),
            "open it in $EDITOR and fix the <<<<<<< / >>>>>>> sections".to_string(),
        ));
        lines.push(step(
            key(KeyIntent::ConflictMarkResolved),
            "mark it resolved (stages it)".to_string(),
        ));
        lines.push(Line::from(""));
        lines.push(step(
            key(KeyIntent::RebaseContinue),
            "then continue the rebase".to_string(),
        ));
    }
    let abort = match conflict.driver {
        RebaseDriver::GhStack => "abort, restoring every branch in the stack",
        RebaseDriver::Git => "abort, restoring the branch",
    };
    lines.push(step(key(KeyIntent::RebaseAbort), abort.to_string()));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        match conflict.driver {
            RebaseDriver::GhStack => {
                "Continuing runs `gh stack rebase --continue`, which also rebases the layers above."
            }
            RebaseDriver::Git => {
                "This rebase wasn't started by gh stack, so it's continued with plain git."
            }
        },
        THEME.text.muted,
    )));
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::{app_state, stack_summary};
    use crate::tui::state::Screen;
    use crossterm::event::{KeyCode, KeyModifiers};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn state_with(files: &[(&str, bool)]) -> AppState {
        let mut state = app_state(vec![stack_summary("s", 2)], Screen::Layers(0));
        state.conflict = Some(RebaseConflict {
            driver: RebaseDriver::GhStack,
            branch: Some("s-layer-1".to_string()),
            files: files
                .iter()
                .map(|(path, has_markers)| ConflictedFile {
                    path: path.to_string(),
                    has_markers: *has_markers,
                })
                .collect(),
        });
        state
    }

    fn screen(state: &AppState, resolver: &mut ConflictResolver, width: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, 20)).unwrap();
        terminal.draw(|frame| resolver.draw(frame, state)).unwrap();
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect()
    }

    #[test]
    fn keys_act_on_the_selected_file() {
        let state = state_with(&[("a.rs", true), ("b.rs", false)]);
        let mut resolver = ConflictResolver::new();

        assert!(
            resolver
                .handle_key(key(KeyCode::Char('j')), &state)
                .is_empty()
        );
        let edit = resolver.handle_key(key(KeyCode::Char('e')), &state);
        let mark = resolver.handle_key(key(KeyCode::Char('m')), &state);
        // Can't move past the last file.
        resolver.handle_key(key(KeyCode::Char('j')), &state);
        let edit_again = resolver.handle_key(key(KeyCode::Char('e')), &state);

        assert!(matches!(edit.as_slice(), [Action::EditConflictFile { path }] if path == "b.rs"));
        assert!(matches!(mark.as_slice(), [Action::StageConflictFile { path }] if path == "b.rs"));
        assert!(
            matches!(edit_again.as_slice(), [Action::EditConflictFile { path }] if path == "b.rs")
        );
    }

    #[test]
    fn continue_abort_reload_and_quit_keys() {
        let state = state_with(&[("a.rs", true)]);
        let mut resolver = ConflictResolver::new();
        let mut press = |c| resolver.handle_key(key(KeyCode::Char(c)), &state);

        assert!(matches!(press('C').as_slice(), [Action::ContinueRebase]));
        assert!(matches!(press('A').as_slice(), [Action::AbortRebase]));
        assert!(matches!(press('r').as_slice(), [Action::LoadRebaseState]));
        assert!(matches!(press('q').as_slice(), [Action::Quit]));
    }

    #[test]
    fn edit_does_nothing_without_files() {
        let state = state_with(&[]);
        let mut resolver = ConflictResolver::new();

        assert!(
            resolver
                .handle_key(key(KeyCode::Char('e')), &state)
                .is_empty()
        );
        assert!(
            resolver
                .handle_key(key(KeyCode::Char('m')), &state)
                .is_empty()
        );
    }

    #[test]
    fn selection_is_clamped_when_the_list_shrinks() {
        let mut state = state_with(&[("a.rs", true), ("b.rs", true)]);
        let mut resolver = ConflictResolver::new();
        resolver.handle_key(key(KeyCode::Char('j')), &state);
        assert_eq!(resolver.selected, 1);

        state.conflict.as_mut().unwrap().files.pop();
        resolver.update(&Action::LoadRebaseState, &mut state);

        assert_eq!(resolver.selected, 0);
    }

    #[test]
    fn renders_files_guidance_and_footer_hints() {
        let state = state_with(&[("src/a.rs", true), ("b.rs", false)]);
        let text = screen(&state, &mut ConflictResolver::new(), 140);

        assert!(text.contains("rebase conflict"));
        assert!(text.contains("Rebase of s-layer-1 stopped (gh stack rebase)"));
        assert!(text.contains("src/a.rs  conflict markers"));
        assert!(text.contains("b.rs  no markers left"));
        assert!(text.contains("e edit"));
        assert!(text.contains("m mark resolved"));
        assert!(text.contains("C continue"));
        assert!(text.contains("A abort"));
        assert!(text.contains("? help"));
    }

    #[test]
    fn with_everything_resolved_it_offers_continue() {
        let state = state_with(&[]);
        let text = screen(&state, &mut ConflictResolver::new(), 120);

        assert!(text.contains("No conflicted files left."));
        assert!(text.contains("All conflicts are resolved."));
    }

    #[test]
    fn help_overlay_lists_the_conflict_keys() {
        let state = state_with(&[("a.rs", true)]);
        let mut resolver = ConflictResolver::new();
        resolver.handle_key(key(KeyCode::Char('?')), &state);

        let text = screen(&state, &mut resolver, 120);

        assert!(text.contains("Rebase conflict"));
        assert!(text.contains("open file in $EDITOR"));
    }
}
