use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Gauge, ListState, Paragraph, Wrap};

use crate::stack::{Layer, LayerDetail, StackSummary};
use crate::theme::glyphs::{GlyphSet, NERD_FONT};
use crate::theme::ui::THEME;
use crate::tui::app::{
    Action, AppState, Component, Screen, layer_detail_cache_key, layer_diff_cache_key,
    lower_layer_ref, spinner_frame,
};

use super::keymap::{KeyIntent, key_intent};
use super::panel::panel_block;
use super::submit_progress::{LayerSubmitStatus, SubmitLayerProgress, SubmitProgress};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivePanel {
    Stacks,
    Layers,
    Detail,
    Files,
    Diff,
}

/// Which field of the "add layer" prompt is currently being typed into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AddLayerField {
    Branch,
    Message,
}

/// Inline prompt state for `gh stack add`, opened by [`KeyIntent::AddLayer`].
///
/// The flow is two sequential single-line fields: the new branch name first,
/// then an optional commit message (`-m`). Enter on the branch field moves
/// to the message field (only once the branch name is non-empty); Enter on
/// the message field submits an [`Action::AddLayer`] (with `message: None`
/// if left blank) and closes the prompt. Escape cancels at either step
/// without dispatching anything.
#[derive(Debug, Clone)]
struct AddLayerPrompt {
    stack_index: usize,
    field: AddLayerField,
    branch: String,
    message: String,
}

impl AddLayerPrompt {
    fn new(stack_index: usize) -> Self {
        Self {
            stack_index,
            field: AddLayerField::Branch,
            branch: String::new(),
            message: String::new(),
        }
    }
}

pub struct StackLayers {
    stack_list_state: ListState,
    list_state: ListState,
    active_stack_label: Option<String>,
    active_panel: ActivePanel,
    last_non_diff_panel: ActivePanel,
    selected_diff_file: usize,
    pending_g: bool,
    diff_scroll: u16,
    active_layer_key: Option<String>,
    add_layer_prompt: Option<AddLayerPrompt>,
}

impl StackLayers {
    pub fn new() -> Self {
        Self {
            stack_list_state: ListState::default(),
            list_state: ListState::default(),
            active_stack_label: None,
            active_panel: ActivePanel::Stacks,
            last_non_diff_panel: ActivePanel::Detail,
            selected_diff_file: 0,
            pending_g: false,
            diff_scroll: 0,
            active_layer_key: None,
            add_layer_prompt: None,
        }
    }

    pub fn selected_index(&self) -> Option<usize> {
        self.list_state.selected()
    }
}

fn render(
    frame: &mut Frame,
    state: &AppState,
    stack_list_state: &mut ListState,
    list_state: &mut ListState,
    active_panel: ActivePanel,
    selected_diff_file: usize,
    diff_scroll: u16,
) {
    let [header_area, content_area, footer_area] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(0),
        Constraint::Length(2),
    ])
    .areas(frame.area());

    let selected_stack = selected_stack_index(state, stack_list_state.selected());
    let selected_layer = selected_stack
        .and_then(|index| state.stacks.get(index))
        .and_then(|stack| {
            list_state
                .selected()
                .and_then(|layer_index| stack.layers.get(layer_index))
        });
    render_header(frame, header_area, state, selected_stack, selected_layer);
    render_stack(
        frame,
        content_area,
        state,
        stack_list_state,
        list_state,
        selected_stack,
        active_panel,
        selected_diff_file,
        diff_scroll,
    );
    render_footer(frame, footer_area, state);

    if let Some(progress) = &state.submit_progress {
        render_submit_progress(frame, content_area, progress);
    }
}

fn render_submit_progress(frame: &mut Frame, area: Rect, progress: &SubmitProgress) {
    let area = centered_rect(area, 70, 60);

    let failed = progress.failed_count();
    let title = if !progress.finished {
        " submitting stack ".to_string()
    } else if failed > 0 {
        format!(" submit finished — {failed} failed ")
    } else {
        " submit finished ".to_string()
    };
    let border_color = if failed > 0 {
        THEME.colors.danger
    } else if progress.finished {
        THEME.colors.success
    } else {
        THEME.colors.primary
    };

    let block = Block::default()
        .title(title)
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(border_color));
    let inner = block.inner(area);

    frame.render_widget(Clear, area);
    frame.render_widget(block, area);

    let [gauge_area, list_area, footer_area] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(0),
        Constraint::Length(1),
    ])
    .areas(inner);

    let gauge_color = if failed > 0 {
        THEME.colors.danger
    } else if progress.finished {
        THEME.colors.success
    } else {
        THEME.colors.warning
    };
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(gauge_color).bg(THEME.colors.surface))
        .label(format!(
            "{}/{} layers",
            progress.completed_count(),
            progress.total()
        ))
        .ratio(progress.ratio().clamp(0.0, 1.0));
    frame.render_widget(gauge, gauge_area);

    let lines = progress
        .layers
        .iter()
        .map(submit_layer_line)
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), list_area);

    let footer_text = if !progress.finished {
        "submitting… please wait"
    } else if failed > 0 {
        "some layers failed — esc/q to dismiss"
    } else {
        "all layers submitted — esc/q to dismiss"
    };
    frame.render_widget(
        Paragraph::new(footer_text).style(THEME.text.muted),
        footer_area,
    );
}

fn submit_layer_line(layer: &SubmitLayerProgress) -> Line<'static> {
    let (glyph, style, text) = match &layer.status {
        LayerSubmitStatus::Pending => (glyphs().pending, THEME.text.muted, "pending".to_string()),
        LayerSubmitStatus::InProgress => (
            glyphs().running,
            Style::default().fg(THEME.colors.warning),
            "pushing…".to_string(),
        ),
        LayerSubmitStatus::Pushed => (
            glyphs().running,
            Style::default().fg(THEME.colors.warning),
            "pushed, syncing pull request…".to_string(),
        ),
        LayerSubmitStatus::PullRequestCreated { number } => (
            glyphs().check,
            Style::default().fg(THEME.colors.success),
            format!("pushed, PR #{number} created"),
        ),
        LayerSubmitStatus::PullRequestUpdated { number } => (
            glyphs().check,
            Style::default().fg(THEME.colors.success),
            format!("pushed, PR #{number} updated"),
        ),
        LayerSubmitStatus::Failed(message) => (
            glyphs().cross,
            Style::default().fg(THEME.colors.danger),
            format!("failed: {message}"),
        ),
    };

    Line::from(vec![
        Span::styled(format!("{glyph} "), style),
        Span::styled(format!("{:<28}", layer.branch), style),
        Span::styled(text, style),
    ])
}

/// A rect centered within `area`, `percent_x`/`percent_y` of its size.
fn centered_rect(area: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let [_, vertical, _] = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .areas(area);
    let [_, horizontal, _] = Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .areas(vertical);
    horizontal
}

fn render_header(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<&Layer>,
) {
    let header = Block::default()
        .title(Line::from(vec![
            Span::styled(
                " Trellis ",
                Style::default()
                    .fg(THEME.colors.text_inverse)
                    .bg(THEME.colors.primary),
            ),
            Span::styled(" stacks", THEME.text.heading.fg(THEME.colors.secondary)),
        ]))
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(THEME.primary_border());

    let content = if let Some(layer) = selected_layer {
        header_summary_line(area, layer.branch.clone(), layer_status_text(layer))
    } else {
        Line::from(Span::raw(
            selected_stack
                .and_then(|index| state.stacks.get(index))
                .map(|stack| format!("{} (trunk: {})", stack_name(stack), stack.trunk))
                .unwrap_or_else(|| "Browse locally tracked stacks and layer status".to_string()),
        ))
    };

    frame.render_widget(
        Paragraph::new(content)
            .style(
                Style::default()
                    .fg(THEME.colors.secondary)
                    .add_modifier(Modifier::BOLD),
            )
            .block(header),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, state: &AppState) {
    let content = footer_line(state);

    frame.render_widget(
        Paragraph::new(content).block(
            Block::default()
                .borders(Borders::TOP)
                .border_style(Style::default().fg(THEME.colors.text_muted)),
        ),
        area,
    );
}

fn footer_line(state: &AppState) -> Line<'static> {
    if let Some(error) = &state.error {
        Line::from(vec![
            Span::styled("error: ", THEME.text.key.fg(THEME.colors.danger)),
            Span::raw(error.clone()),
            Span::raw("  "),
            Span::styled("x", THEME.text.key.fg(THEME.colors.warning)),
            Span::raw(" dismiss"),
        ])
    } else if state.refresh_in_flight {
        Line::from(vec![
            Span::styled(
                format!("{} ", spinner_frame(state.refresh_spinner_frame)),
                THEME.text.key.fg(THEME.colors.secondary),
            ),
            Span::raw("Refreshing stacks in background"),
        ])
    } else {
        Line::from(vec![
            Span::styled("j/k", THEME.text.key),
            Span::raw(" navigate  "),
            Span::styled("space/enter", THEME.text.key),
            Span::raw(" open  "),
            Span::styled("tab/shift-tab", THEME.text.key),
            Span::raw(" focus  "),
            Span::styled("o", THEME.text.key),
            Span::raw(" PR  "),
            Span::styled("d", THEME.text.key),
            Span::raw(" diff  "),
            Span::styled("c", THEME.text.key),
            Span::raw(" checkout  "),
            Span::styled("a", THEME.text.key),
            Span::raw(" add layer  "),
            Span::styled("gg/G ^u/^d", THEME.text.key),
            Span::raw(" diff jump  "),
            Span::styled("s", THEME.text.key),
            Span::raw(" submit  "),
            Span::styled("t", THEME.text.key),
            Span::raw(format!(
                " auto:{}  ",
                toggle_label(state.submit_options.auto)
            )),
            Span::styled("p", THEME.text.key),
            Span::raw(format!(
                " open:{}  ",
                toggle_label(state.submit_options.open)
            )),
            Span::styled("r", THEME.text.key),
            Span::raw(" refresh  "),
            Span::styled("q", THEME.text.key.fg(THEME.colors.danger)),
            Span::raw(" quit"),
        ])
    }
}

/// Renders the inline `gh stack add` prompt as a small bordered overlay
/// centered over the rest of the screen.
fn render_add_layer_prompt(frame: &mut Frame, area: Rect, prompt: &AddLayerPrompt) {
    let popup_area = centered_rect(area, 60, 8);
    frame.render_widget(Clear, popup_area);

    let (label, value, hint) = match prompt.field {
        AddLayerField::Branch => (
            "New branch name",
            prompt.branch.as_str(),
            "enter continue  esc cancel",
        ),
        AddLayerField::Message => (
            "Commit message (optional, -m)",
            prompt.message.as_str(),
            "enter add layer  esc cancel",
        ),
    };

    let lines = vec![
        Line::from(Span::styled(label, THEME.text.label)),
        Line::from(Span::raw(format!("{value}\u{2588}"))),
        Line::from(""),
        Line::from(Span::styled(hint, THEME.text.muted)),
    ];

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block("add layer", true))
            .wrap(Wrap { trim: false }),
        popup_area,
    );
}

fn toggle_label(enabled: bool) -> &'static str {
    if enabled { "on" } else { "off" }
}

fn render_stack(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    stack_list_state: &mut ListState,
    list_state: &mut ListState,
    selected_stack: Option<usize>,
    active_panel: ActivePanel,
    selected_diff_file: usize,
    diff_scroll: u16,
) {
    let [stack_area, detail_area] = Layout::new(
        Direction::Horizontal,
        [Constraint::Percentage(32), Constraint::Percentage(68)],
    )
    .areas(area);

    render_navigator(
        frame,
        stack_area,
        state,
        stack_list_state.selected(),
        list_state.selected(),
        active_panel,
    );

    let Some(stack) = selected_stack.and_then(|index| state.stacks.get(index)) else {
        frame.render_widget(
            Paragraph::new("No stacks found in this repository.")
                .style(THEME.text.muted)
                .block(panel_block(
                    "navigator",
                    matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
                )),
            stack_area,
        );
        frame.render_widget(
            Paragraph::new("Select a stack to view its layers.")
                .style(THEME.text.muted)
                .block(panel_block("details", active_panel == ActivePanel::Detail)),
            detail_area,
        );
        return;
    };

    render_layer_detail(
        frame,
        detail_area,
        state,
        stack,
        list_state.selected(),
        active_panel,
        selected_diff_file,
        diff_scroll,
    );
}

fn render_navigator(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
    active_panel: ActivePanel,
) {
    if state.stacks.is_empty() {
        frame.render_widget(
            Paragraph::new("No stacks found in this repository.")
                .style(THEME.text.muted)
                .block(panel_block(
                    "navigator",
                    matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
                )),
            area,
        );
        return;
    }

    let lines = navigator_lines(state, selected_stack, selected_layer, active_panel);
    frame.render_widget(
        Paragraph::new(lines).block(panel_block(
            "navigator",
            matches!(active_panel, ActivePanel::Stacks | ActivePanel::Layers),
        )),
        area,
    );
}

fn render_layer_detail(
    frame: &mut Frame,
    area: Rect,
    state: &AppState,
    stack: &StackSummary,
    selected: Option<usize>,
    active_panel: ActivePanel,
    selected_diff_file: usize,
    diff_scroll: u16,
) {
    let Some(selected) = selected else {
        frame.render_widget(
            Paragraph::new("No layer selected")
                .style(THEME.text.muted)
                .block(panel_block("details", active_panel == ActivePanel::Detail)),
            area,
        );
        return;
    };

    let Some(layer) = stack.layers.get(selected) else {
        return;
    };

    let rebase_status = if layer.needs_rebase {
        Span::styled(
            "Needs rebase",
            Style::default()
                .fg(THEME.colors.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "Up to date",
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD),
        )
    };

    let detail = state
        .layer_details
        .get(&layer_detail_cache_key(stack, layer));
    let diff_key = layer_diff_cache_key(stack, layer);
    let diff = state.layer_diffs.get(&diff_key).map(String::as_str);
    let diff_loading = state.layer_diffs.is_loading(&diff_key);

    if active_panel == ActivePanel::Diff {
        render_diff(
            frame,
            area,
            stack,
            selected,
            diff,
            diff_loading,
            true,
            selected_diff_file,
            diff_scroll,
        );
        return;
    }

    let lines = detail_lines(layer, rebase_status, detail);
    let [summary_area, files_area] =
        Layout::vertical([Constraint::Length(10), Constraint::Min(0)]).areas(area);

    frame.render_widget(
        Paragraph::new(lines)
            .block(panel_block(
                layer_title(layer),
                active_panel == ActivePanel::Detail,
            ))
            .wrap(Wrap { trim: false }),
        summary_area,
    );

    render_diff_files(
        frame,
        files_area,
        diff,
        diff_loading,
        active_panel == ActivePanel::Files,
        selected_diff_file,
    );
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DiffFile {
    path: String,
    status: char,
    additions: usize,
    deletions: usize,
    start: usize,
    end: usize,
}

fn parse_diff_files(diff: &str) -> Vec<DiffFile> {
    let lines: Vec<&str> = diff.lines().collect();
    let mut files: Vec<DiffFile> = Vec::new();

    for (index, line) in lines.iter().enumerate() {
        if let Some(path) = line.strip_prefix("diff --git a/") {
            if let Some(previous) = files.last_mut() {
                previous.end = index;
            }

            let path = path.split(" b/").nth(1).unwrap_or(path).to_string();
            files.push(DiffFile {
                path,
                status: 'M',
                additions: 0,
                deletions: 0,
                start: index,
                end: lines.len(),
            });
        } else if let Some(file) = files.last_mut() {
            if line.starts_with("new file mode") {
                file.status = 'A';
            } else if line.starts_with("deleted file mode") {
                file.status = 'D';
            } else if line.starts_with("rename from") || line.starts_with("rename to") {
                file.status = 'R';
            } else if line.starts_with('+') && !line.starts_with("+++") {
                file.additions += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                file.deletions += 1;
            }
        }
    }

    files
}

fn render_diff_files(
    frame: &mut Frame,
    area: Rect,
    diff: Option<&str>,
    is_loading: bool,
    is_active: bool,
    selected_diff_file: usize,
) {
    let block = panel_block("changed files", is_active);

    let Some(diff) = diff else {
        let message = if is_loading {
            "Loading diff..."
        } else {
            "Diff not loaded."
        };
        frame.render_widget(
            Paragraph::new(message).style(THEME.text.muted).block(block),
            area,
        );
        return;
    };

    let files = parse_diff_files(diff);
    if files.is_empty() {
        frame.render_widget(
            Paragraph::new("No changes in this layer.")
                .style(THEME.text.muted)
                .block(block),
            area,
        );
        return;
    }

    let lines = build_file_tree_lines(&files, area.width, selected_diff_file, is_active);
    frame.render_widget(Paragraph::new(lines).block(block), area);
}

fn render_diff(
    frame: &mut Frame,
    area: Rect,
    stack: &StackSummary,
    selected_layer: usize,
    diff: Option<&str>,
    is_loading: bool,
    is_active: bool,
    selected_diff_file: usize,
    diff_scroll: u16,
) {
    let Some(layer) = stack.layers.get(selected_layer) else {
        return;
    };
    let files = diff.map(parse_diff_files).unwrap_or_default();
    let selected_file = files.get(selected_diff_file.min(files.len().saturating_sub(1)));
    let title = selected_file
        .map(|file| format!(" diff {} ", file.path))
        .unwrap_or_else(|| {
            format!(
                " diff {}..{} ",
                lower_layer_ref(stack, selected_layer),
                layer.branch
            )
        });
    let block = panel_block(title, is_active);

    let Some(diff) = diff else {
        let message = if is_loading {
            "Loading diff..."
        } else {
            "Diff unavailable."
        };
        frame.render_widget(
            Paragraph::new(message).style(THEME.text.muted).block(block),
            area,
        );
        return;
    };

    let lines = diff.lines().collect::<Vec<_>>();
    let visible_height = area.height.saturating_sub(2) as usize;
    let (line_start, line_end) = selected_file
        .map(|file| (file.start, file.end))
        .unwrap_or((0, lines.len()));
    let max_start = line_end.saturating_sub(visible_height);
    let start =
        (line_start + usize::from(diff_scroll)).clamp(line_start, max_start.max(line_start));
    let end = (start + visible_height).min(line_end);
    let rendered = lines[start..end]
        .iter()
        .map(|line| diff_line(line))
        .collect::<Vec<_>>();

    frame.render_widget(Paragraph::new(rendered).block(block), area);
}

fn diff_line(text: &str) -> Line<'static> {
    let style = if text.starts_with("+++") || text.starts_with("---") {
        Style::default().fg(THEME.colors.text_muted)
    } else if text.starts_with('+') {
        Style::default().fg(THEME.colors.success)
    } else if text.starts_with('-') {
        Style::default().fg(THEME.colors.danger)
    } else if text.starts_with("@@") {
        Style::default()
            .fg(THEME.colors.primary)
            .add_modifier(Modifier::BOLD)
    } else if text.starts_with("diff --git") {
        Style::default()
            .fg(THEME.colors.secondary)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    Line::from(Span::styled(text.to_string(), style))
}

fn detail_lines(
    layer: &Layer,
    rebase_status: Span<'static>,
    detail: Option<&LayerDetail>,
) -> Vec<Line<'static>> {
    let pr = layer.pull_request.as_ref();
    let title = detail
        .map(|detail| detail.pull_request.title.clone())
        .filter(|title| !title.is_empty())
        .or_else(|| pr.and_then(|pr| pr.title.clone()))
        .unwrap_or_else(|| "Not submitted".to_string());
    let author = detail
        .and_then(|detail| detail.commits.first())
        .and_then(|commit| commit.author.clone())
        .unwrap_or_else(|| "-".to_string());
    let mut lines = vec![
        Line::from(vec![Span::styled(
            pr.map(|pr| format!("PR #{}", pr.number))
                .unwrap_or_else(|| "No PR".to_string()),
            THEME.text.heading,
        )]),
        Line::from(Span::styled(title, THEME.text.body)),
        Line::from(vec![
            label_span("status"),
            rebase_status,
            Span::raw("  "),
            checks_span(detail),
        ]),
        Line::from(""),
        labeled_line("branch", layer.branch.clone()),
        labeled_line("base", layer.base.clone()),
        labeled_line("author", author),
    ];

    if let Some(detail) = detail
        && let Some(snippet) = &detail.pull_request.description_snippet
    {
        lines.push(labeled_line("summary", snippet.clone()));
    }

    lines
}

fn labeled_line(label: &str, value: String) -> Line<'static> {
    Line::from(vec![label_span(label), Span::raw(value)])
}

fn label_span(label: &str) -> Span<'static> {
    Span::styled(format!("{label:<10}"), THEME.text.label)
}

fn glyphs() -> &'static GlyphSet {
    &NERD_FONT
}

impl Component for StackLayers {
    fn draw(&mut self, frame: &mut Frame, state: &AppState) {
        render(
            frame,
            state,
            &mut self.stack_list_state,
            &mut self.list_state,
            self.active_panel,
            self.selected_diff_file,
            self.diff_scroll,
        );

        if let Some(prompt) = &self.add_layer_prompt {
            render_add_layer_prompt(frame, frame.area(), prompt);
        }
    }

    fn handle_key(&mut self, key: KeyEvent, state: &AppState) -> Vec<Action> {
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
            Some(KeyIntent::ToggleDiff) => vec![Action::ToggleDiffView],
            Some(KeyIntent::Submit) => selected_stack
                .map(|stack_index| vec![Action::SubmitStack { stack_index }])
                .unwrap_or_default(),
            Some(KeyIntent::ToggleSubmitAuto) => vec![Action::ToggleSubmitAuto],
            Some(KeyIntent::ToggleSubmitOpen) => vec![Action::ToggleSubmitOpen],
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

    fn update(&mut self, action: &Action, state: &mut AppState) {
        match action {
            Action::ShowLayers(stack_index) => {
                self.stack_list_state.select(Some(*stack_index));
                let Some(stack) = state.stacks.get(*stack_index) else {
                    return;
                };
                let count = stack.layers.len();
                let stack_label = Some(stack.label.clone());
                let preserve_selection =
                    self.active_stack_label.as_deref() == stack_label.as_deref();
                let next_selection =
                    clamped_selection(count, self.list_state.selected(), preserve_selection);
                self.list_state.select(next_selection);
                self.active_stack_label = stack_label;
                self.reset_diff_view_for_selection(state);
            }
            Action::SelectNext if matches!(state.screen, Screen::Layers(_)) => {
                select_next(&mut self.list_state, active_layer_count(state));
                self.reset_diff_view_for_selection(state);
            }
            Action::SelectPrevious if matches!(state.screen, Screen::Layers(_)) => {
                select_previous(&mut self.list_state, active_layer_count(state));
                self.reset_diff_view_for_selection(state);
            }
            Action::FocusNextPanel => {
                self.active_panel = next_panel(self.active_panel);
            }
            Action::FocusPreviousPanel => {
                self.active_panel = previous_panel(self.active_panel);
            }
            Action::SelectNextDiffFile => {
                self.select_next_diff_file(state);
            }
            Action::SelectPreviousDiffFile => {
                self.select_previous_diff_file(state);
            }
            Action::ScrollDiffDown => {
                self.scroll_diff_down();
            }
            Action::ScrollDiffUp => {
                self.scroll_diff_up();
            }
            Action::ScrollDiffHalfPageDown => {
                self.scroll_diff_half_page_down();
            }
            Action::ScrollDiffHalfPageUp => {
                self.scroll_diff_half_page_up();
            }
            Action::ScrollDiffLineDown => {
                self.scroll_diff_line_down();
            }
            Action::ScrollDiffLineUp => {
                self.scroll_diff_line_up();
            }
            Action::ScrollDiffTop => {
                self.scroll_diff_top();
            }
            Action::ScrollDiffBottom => {
                self.scroll_diff_bottom();
            }
            Action::ToggleDiffView => {
                if self.active_panel == ActivePanel::Diff {
                    self.active_panel = self.last_non_diff_panel;
                } else {
                    self.last_non_diff_panel = self.active_panel;
                    self.active_panel = ActivePanel::Diff;
                }
            }
            Action::StacksLoaded(selected_stack) => {
                let selected_stack = selected_stack
                    .or_else(|| selected_stack_index(state, self.stack_list_state.selected()));
                self.stack_list_state.select(selected_stack);
                let active_stack =
                    selected_stack.and_then(|stack_index| state.stacks.get(stack_index));
                let layer_count = active_stack.map(|stack| stack.layers.len()).unwrap_or(0);
                let active_label = active_stack.map(|stack| stack.label.clone());
                let preserve_selection =
                    self.active_stack_label.as_deref() == active_label.as_deref();
                let next_selection =
                    clamped_selection(layer_count, self.list_state.selected(), preserve_selection);
                self.list_state.select(next_selection);
                self.active_stack_label = active_label;
                self.reset_diff_view_for_selection(state);
            }
            _ => {}
        }
    }
}

impl StackLayers {
    /// Routes a key event while the add-layer prompt is open, so its own
    /// text-entry keys (including letters that are otherwise global
    /// shortcuts, like `q` or `c`) are never mistaken for a `KeyIntent`.
    fn handle_add_layer_prompt_key(&mut self, key: KeyEvent) -> Vec<Action> {
        let Some(prompt) = self.add_layer_prompt.as_mut() else {
            return Vec::new();
        };

        match key.code {
            KeyCode::Esc => {
                self.add_layer_prompt = None;
            }
            KeyCode::Enter => match prompt.field {
                AddLayerField::Branch => {
                    if !prompt.branch.trim().is_empty() {
                        prompt.field = AddLayerField::Message;
                    }
                }
                AddLayerField::Message => {
                    let stack_index = prompt.stack_index;
                    let branch = prompt.branch.trim().to_string();
                    let message = prompt.message.trim();
                    let message = if message.is_empty() {
                        None
                    } else {
                        Some(message.to_string())
                    };
                    self.add_layer_prompt = None;
                    return vec![Action::AddLayer {
                        stack_index,
                        branch,
                        message,
                    }];
                }
            },
            KeyCode::Backspace => {
                let field = match prompt.field {
                    AddLayerField::Branch => &mut prompt.branch,
                    AddLayerField::Message => &mut prompt.message,
                };
                field.pop();
            }
            KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL) => {
                let field = match prompt.field {
                    AddLayerField::Branch => &mut prompt.branch,
                    AddLayerField::Message => &mut prompt.message,
                };
                field.push(c);
            }
            _ => {}
        }

        Vec::new()
    }

    fn reset_diff_view_for_selection(&mut self, state: &AppState) {
        let next_key = active_layer_key(
            state,
            selected_stack_index(state, self.stack_list_state.selected()),
            self.list_state.selected(),
        );
        if self.active_layer_key != next_key {
            self.selected_diff_file = 0;
            self.diff_scroll = 0;
            self.active_layer_key = next_key;
        }
    }

    fn select_next_diff_file(&mut self, state: &AppState) {
        let count = active_diff_file_count(state, self.list_state.selected());
        if count == 0 {
            self.selected_diff_file = 0;
            return;
        }

        self.selected_diff_file = (self.selected_diff_file + 1) % count;
        self.diff_scroll = 0;
    }

    fn select_previous_diff_file(&mut self, state: &AppState) {
        let count = active_diff_file_count(state, self.list_state.selected());
        if count == 0 {
            self.selected_diff_file = 0;
            return;
        }

        self.selected_diff_file = if self.selected_diff_file == 0 {
            count - 1
        } else {
            self.selected_diff_file - 1
        };
        self.diff_scroll = 0;
    }

    fn scroll_diff_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(12);
    }

    fn scroll_diff_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(12);
    }

    fn scroll_diff_half_page_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(10);
    }

    fn scroll_diff_half_page_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(10);
    }

    fn scroll_diff_line_down(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_add(1);
    }

    fn scroll_diff_line_up(&mut self) {
        self.diff_scroll = self.diff_scroll.saturating_sub(1);
    }

    fn scroll_diff_top(&mut self) {
        self.diff_scroll = 0;
    }

    fn scroll_diff_bottom(&mut self) {
        self.diff_scroll = u16::MAX;
    }
}

fn next_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Layers,
        ActivePanel::Layers => ActivePanel::Detail,
        ActivePanel::Detail => ActivePanel::Files,
        ActivePanel::Files => ActivePanel::Diff,
        ActivePanel::Diff => ActivePanel::Stacks,
    }
}

fn previous_panel(panel: ActivePanel) -> ActivePanel {
    match panel {
        ActivePanel::Stacks => ActivePanel::Diff,
        ActivePanel::Layers => ActivePanel::Stacks,
        ActivePanel::Detail => ActivePanel::Layers,
        ActivePanel::Files => ActivePanel::Detail,
        ActivePanel::Diff => ActivePanel::Files,
    }
}

fn active_layer_key(
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
) -> Option<String> {
    let stack_index = selected_stack?;
    let stack = state.stacks.get(stack_index)?;
    let layer = stack.layers.get(selected_layer?)?;
    Some(layer_diff_cache_key(stack, layer))
}

fn active_layer_count(state: &AppState) -> usize {
    selected_stack_index(state, None)
        .and_then(|stack_index| state.stacks.get(stack_index))
        .map(|stack| stack.layers.len())
        .unwrap_or(0)
}

fn active_diff_file_count(state: &AppState, selected_layer: Option<usize>) -> usize {
    let Some(stack_index) = selected_stack_index(state, None) else {
        return 0;
    };
    let Some(stack) = state.stacks.get(stack_index) else {
        return 0;
    };
    let Some(layer) = selected_layer.and_then(|index| stack.layers.get(index)) else {
        return 0;
    };

    state
        .layer_diffs
        .get(&layer_diff_cache_key(stack, layer))
        .map(|diff| parse_diff_files(diff).len())
        .unwrap_or(0)
}

fn clamped_selection(
    count: usize,
    selected: Option<usize>,
    preserve_selection: bool,
) -> Option<usize> {
    if count == 0 {
        None
    } else if preserve_selection {
        match selected {
            Some(index) if index < count => Some(index),
            _ => Some(0),
        }
    } else {
        Some(0)
    }
}

fn selected_stack_index(state: &AppState, fallback: Option<usize>) -> Option<usize> {
    match state.screen {
        Screen::Layers(index) if index < state.stacks.len() => Some(index),
        _ => fallback
            .filter(|index| *index < state.stacks.len())
            .or_else(|| state.stacks.iter().position(|stack| stack.is_current))
            .or(if state.stacks.is_empty() {
                None
            } else {
                Some(0)
            }),
    }
}

fn next_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let next = match selected {
        Some(index) if index + 1 < state.stacks.len() => index + 1,
        _ => 0,
    };

    vec![Action::ShowLayers(next)]
}

fn previous_stack_action(state: &AppState, selected: Option<usize>) -> Vec<Action> {
    if state.stacks.is_empty() {
        return Vec::new();
    }

    let previous = match selected {
        Some(0) | None => state.stacks.len() - 1,
        Some(index) => index - 1,
    };

    vec![Action::ShowLayers(previous)]
}

fn navigator_lines(
    state: &AppState,
    selected_stack: Option<usize>,
    selected_layer: Option<usize>,
    active_panel: ActivePanel,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();

    for (stack_index, stack) in state.stacks.iter().enumerate() {
        let expanded = Some(stack_index) == selected_stack;
        let stack_style = if expanded && active_panel == ActivePanel::Stacks {
            THEME.text.selected
        } else if stack.is_current {
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        let symbol = if expanded { "▼" } else { "▶" };
        lines.push(Line::from(Span::styled(
            format!("{symbol} {}", stack_name(stack)),
            stack_style,
        )));

        if expanded {
            lines.push(Line::from(Span::styled(
                format!("  trunk: {}", stack.trunk),
                THEME.text.muted,
            )));
            lines.push(Line::from(""));

            for (layer_index, layer) in stack.layers.iter().enumerate() {
                let branch_marker = if layer_index + 1 == stack.layers.len() {
                    "└─"
                } else {
                    "├─"
                };
                let style =
                    if selected_layer == Some(layer_index) && active_panel == ActivePanel::Layers {
                        THEME.text.selected
                    } else if layer.is_current {
                        Style::default()
                            .fg(THEME.colors.success)
                            .add_modifier(Modifier::BOLD)
                    } else {
                        Style::default()
                    };
                lines.push(Line::from(Span::styled(
                    format!(
                        "  {branch_marker} {:<14} {}",
                        layer_title(layer),
                        layer_badge(layer)
                    ),
                    style,
                )));
            }

            lines.push(Line::from(""));
        }
    }

    lines
}

fn header_summary_line(area: Rect, left: String, right: (String, Style)) -> Line<'static> {
    let width = area.width.saturating_sub(4) as usize;
    let right_len = right.0.chars().count();
    let left_len = left.chars().count();
    let spacer_len = width.saturating_sub(left_len + right_len).max(1);
    Line::from(vec![
        Span::raw(left),
        Span::raw(" ".repeat(spacer_len)),
        Span::styled(right.0, right.1),
    ])
}

fn layer_status_text(layer: &Layer) -> (String, Style) {
    if layer.needs_rebase {
        (
            format!("{} needs rebase", glyphs().warning),
            Style::default()
                .fg(THEME.colors.danger)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        (
            format!("{} up to date", glyphs().up),
            Style::default()
                .fg(THEME.colors.success)
                .add_modifier(Modifier::BOLD),
        )
    }
}

fn checks_span(detail: Option<&LayerDetail>) -> Span<'static> {
    let Some(detail) = detail else {
        return Span::styled("loading details", THEME.text.muted);
    };

    let checks = detail.pull_request.checks;
    if checks.total == 0 {
        Span::styled("no checks", THEME.text.muted)
    } else if checks.failing > 0 {
        Span::styled(
            format!(
                "{} {} failing, {} pending of {}",
                glyphs().cross,
                checks.failing,
                checks.pending,
                checks.total
            ),
            Style::default().fg(THEME.colors.danger),
        )
    } else if checks.pending > 0 {
        Span::styled(
            format!(
                "{} {} passing, {} pending of {}",
                glyphs().pending,
                checks.passing,
                checks.pending,
                checks.total
            ),
            Style::default().fg(THEME.colors.warning),
        )
    } else {
        Span::styled(
            format!(
                "{} {}/{} checks passing",
                glyphs().check,
                checks.passing,
                checks.total
            ),
            Style::default().fg(THEME.colors.success),
        )
    }
}

fn stack_name(stack: &StackSummary) -> String {
    let Some(first_prefix) = stack
        .layers
        .first()
        .and_then(|layer| layer.branch.rsplit_once('/').map(|(prefix, _)| prefix))
    else {
        return stack.label.clone();
    };

    if stack
        .layers
        .iter()
        .all(|layer| layer.branch.rsplit_once('/').map(|(prefix, _)| prefix) == Some(first_prefix))
    {
        first_prefix.to_string()
    } else {
        stack.label.clone()
    }
}

fn layer_title(layer: &Layer) -> String {
    layer
        .branch
        .rsplit('/')
        .next()
        .unwrap_or(layer.branch.as_str())
        .to_string()
}

fn layer_badge(layer: &Layer) -> String {
    match &layer.pull_request {
        Some(pr) if layer.is_merged => format!("#{} {}", pr.number, glyphs().check),
        Some(pr) if pr.is_draft == Some(true) => format!("#{} {}", pr.number, glyphs().pending),
        Some(pr) if layer.needs_rebase => format!("#{} {}", pr.number, glyphs().warning),
        Some(pr) => format!("#{} {}", pr.number, glyphs().current),
        None => "unsubmitted".to_string(),
    }
}

fn build_file_tree_lines(
    files: &[DiffFile],
    width: u16,
    selected_index: usize,
    is_active: bool,
) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let mut previous_dirs: Vec<&str> = Vec::new();

    for (file_index, file) in files.iter().enumerate() {
        let parts = file.path.split('/').collect::<Vec<_>>();
        let dirs = &parts[..parts.len().saturating_sub(1)];
        let common_prefix = previous_dirs
            .iter()
            .zip(dirs.iter())
            .take_while(|(left, right)| left == right)
            .count();

        for (depth, dir) in dirs.iter().enumerate().skip(common_prefix) {
            lines.push(folder_line(dir, depth, depth + 1 == dirs.len()));
        }

        lines.push(diff_file_line(
            file,
            width,
            dirs.len(),
            file_index == selected_index,
            is_active,
        ));
        previous_dirs = dirs.to_vec();
    }

    lines
}

fn folder_line(name: &str, depth: usize, is_leaf: bool) -> Line<'static> {
    let branch = if is_leaf { "└─" } else { "├─" };
    Line::from(vec![
        Span::raw(format!("{}{} ", "  ".repeat(depth), branch)),
        Span::styled(glyphs().folder_open, THEME.text.muted),
        Span::raw(" "),
        Span::styled(name.to_string(), THEME.text.muted),
    ])
}

fn diff_file_line(
    file: &DiffFile,
    width: u16,
    depth: usize,
    is_selected: bool,
    is_active: bool,
) -> Line<'static> {
    let stats = format!("+{}  -{}", file.additions, file.deletions);
    let branch = "└─";
    let prefix = format!("{}{} {} ", "  ".repeat(depth), branch, glyphs().file);
    let available = width.saturating_sub(4) as usize;
    let stats_len = stats.chars().count();
    let gap = 2usize;
    let max_path_len = available.saturating_sub(prefix.chars().count() + stats_len + gap);
    let file_name = file
        .path
        .rsplit('/')
        .next()
        .unwrap_or(file.path.as_str())
        .to_string();
    let path = truncate_text(&file_name, max_path_len.max(1));
    let spacer = " ".repeat(
        available.saturating_sub(prefix.chars().count() + path.chars().count() + stats_len),
    );
    let style = if is_selected && is_active {
        THEME.text.selected
    } else {
        Style::default()
    };

    Line::from(vec![
        Span::styled(prefix, style),
        Span::styled(path, style),
        Span::styled(spacer, style),
        Span::styled(stats, THEME.text.muted),
    ])
}

fn truncate_text(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        return text.to_string();
    }
    if max_len <= 1 {
        return "…".to_string();
    }

    let mut truncated = text.chars().take(max_len - 1).collect::<String>();
    truncated.push('…');
    truncated
}

fn select_next(state: &mut ListState, count: usize) {
    if count == 0 {
        state.select(None);
        return;
    }

    let next = match state.selected() {
        Some(index) if index + 1 < count => index + 1,
        _ => 0,
    };

    state.select(Some(next));
}

fn select_previous(state: &mut ListState, count: usize) {
    if count == 0 {
        state.select(None);
        return;
    }

    let previous = match state.selected() {
        Some(0) | None => count - 1,
        Some(index) => index - 1,
    };

    state.select(Some(previous))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stack::{
        CheckSummary, LayerCommit, LayerDetail, PullRequestDetail, PullRequestRef, ReviewerState,
        SubmitOptions,
    };
    use crate::test_fixtures::stack_summary;
    use crate::tui::layer_resource::LayerResourceCache;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn app_state(stacks: Vec<StackSummary>, screen: Screen) -> AppState {
        AppState {
            stacks,
            screen,
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
            should_quit: false,
        }
    }

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn ctrl_key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::CONTROL)
    }

    fn sample_diff_with_paths() -> String {
        [
            "diff --git a/src/a.rs b/src/a.rs",
            "index 123..456 100644",
            "--- a/src/a.rs",
            "+++ b/src/a.rs",
            "@@ -1 +1 @@",
            "-old",
            "+new",
            "diff --git a/src/nested/b.rs b/src/nested/b.rs",
            "index 789..abc 100644",
            "--- a/src/nested/b.rs",
            "+++ b/src/nested/b.rs",
            "@@ -3 +3 @@",
            "+more",
        ]
        .join("\n")
    }

    #[test]
    fn show_layers_preserves_selection_for_same_stack() {
        let mut component = StackLayers::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::ShowLayers(0), &mut state);

        assert_eq!(component.list_state.selected(), Some(1));
    }

    #[test]
    fn show_layers_resets_selection_when_switching_stacks() {
        let mut component = StackLayers::new();
        let mut state = app_state(
            vec![stack_summary("a", 3), stack_summary("b", 3)],
            Screen::Layers(0),
        );

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        component.update(&Action::ShowLayers(1), &mut state);

        assert_eq!(component.stack_list_state.selected(), Some(1));
        assert_eq!(component.list_state.selected(), Some(0));
    }

    #[test]
    fn stacks_loaded_preserves_selection_for_same_stack_label() {
        let mut component = StackLayers::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        state.stacks = vec![stack_summary("x", 1), stack_summary("a", 3)];
        state.screen = Screen::Layers(1);
        component.update(&Action::StacksLoaded(Some(1)), &mut state);

        assert_eq!(component.list_state.selected(), Some(1));
    }

    #[test]
    fn stacks_loaded_resets_selection_for_different_stack_label() {
        let mut component = StackLayers::new();
        let mut state = app_state(vec![stack_summary("a", 3)], Screen::Layers(0));

        component.update(&Action::ShowLayers(0), &mut state);
        component.update(&Action::SelectNext, &mut state);
        state.stacks = vec![stack_summary("b", 3)];
        state.screen = Screen::Layers(0);
        component.update(&Action::StacksLoaded(Some(0)), &mut state);

        assert_eq!(component.list_state.selected(), Some(0));
    }

    #[test]
    fn handle_key_maps_navigation_actions() {
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);
        component.handle_key(key(KeyCode::Char('a')), &state);

        // Global shortcut letters are captured as text, not intents, while
        // the prompt is open.
        component.handle_key(key(KeyCode::Char('c')), &state);
        component.handle_key(key(KeyCode::Char('q')), &state);
        assert_eq!(component.add_layer_prompt.as_ref().unwrap().branch, "cq");

        // Enter with an empty branch is a no-op.
        let mut empty_component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
        let mut component = StackLayers::new();
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
    fn parse_diff_files_finds_each_file_range() {
        let diff = [
            "diff --git a/src/a.rs b/src/a.rs",
            "index 123..456 100644",
            "--- a/src/a.rs",
            "+++ b/src/a.rs",
            "@@ -1 +1 @@",
            "-old",
            "+new",
            "diff --git a/src/b.rs b/src/b.rs",
            "@@ -3 +3 @@",
            "+more",
        ]
        .join("\n");

        let files = parse_diff_files(&diff);

        assert_eq!(
            files,
            vec![
                DiffFile {
                    path: "src/a.rs".to_string(),
                    status: 'M',
                    additions: 1,
                    deletions: 1,
                    start: 0,
                    end: 7,
                },
                DiffFile {
                    path: "src/b.rs".to_string(),
                    status: 'M',
                    additions: 1,
                    deletions: 0,
                    start: 7,
                    end: 10,
                },
            ]
        );
    }

    #[test]
    fn parse_diff_files_detects_added_file() {
        let diff = [
            "diff --git a/src/new.rs b/src/new.rs",
            "new file mode 100644",
            "--- /dev/null",
            "+++ b/src/new.rs",
            "+new",
        ]
        .join("\n");

        let files = parse_diff_files(&diff);
        assert_eq!(files[0].status, 'A');
        assert_eq!(files[0].additions, 1);
    }

    #[test]
    fn detail_lines_include_cached_layer_detail() {
        let mut layer = crate::test_fixtures::layer("feature/layer-1");
        layer.pull_request = Some(PullRequestRef {
            number: 42,
            url: "https://example.test/pull/42".to_string(),
            state: "OPEN".to_string(),
            title: None,
            is_draft: None,
            checks_status: None,
            review_decision: None,
        });
        let detail = LayerDetail {
            commits: vec![LayerCommit {
                oid: "abcdef123456".to_string(),
                subject: "feat: render details".to_string(),
                author: Some("john-doe".to_string()),
                authored_at: "2026-09-18T10:00:00Z".to_string(),
            }],
            pull_request: PullRequestDetail {
                title: "Layer detail pane".to_string(),
                description_snippet: Some("Shows the selected layer.".to_string()),
                reviewers: vec![ReviewerState {
                    login: "octocat".to_string(),
                    state: "APPROVED".to_string(),
                }],
                checks: CheckSummary {
                    total: 2,
                    passing: 1,
                    failing: 0,
                    pending: 1,
                },
                labels: vec!["tui".to_string()],
            },
        };

        let text = detail_lines(&layer, Span::raw("Up to date"), Some(&detail))
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("Layer detail pane"));
        assert!(text.contains("Shows the selected layer."));
        assert!(text.contains("Shows the selected layer."));
        assert!(text.contains("PR #42"));
        assert!(text.contains("john-doe"));
        assert!(text.contains("1 passing, 1 pending of 2"));
    }

    #[test]
    fn footer_shows_refresh_indicator_when_loading() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.refresh_in_flight = true;
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("Refreshing stacks"));
    }

    #[test]
    fn footer_shows_dismissible_error() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.error = Some("auth required".to_string());
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("error:"));
        assert!(text.contains("dismiss"));
    }

    #[test]
    fn footer_shows_submit_toggle_state() {
        let mut state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));
        state.submit_options.auto = true;
        let text = footer_line(&state)
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>();
        assert!(text.contains("submit"));
        assert!(text.contains("auto:on"));
        assert!(text.contains("open:off"));
    }

    #[test]
    fn handle_key_dispatches_submit_for_selected_stack() {
        let mut component = StackLayers::new();
        let mut state = app_state(vec![stack_summary("a", 2)], Screen::Layers(0));
        component.update(&Action::ShowLayers(0), &mut state);

        let actions = component.handle_key(key(KeyCode::Char('s')), &state);
        assert!(matches!(
            actions.as_slice(),
            [Action::SubmitStack { stack_index: 0 }]
        ));
    }

    #[test]
    fn handle_key_toggles_submit_auto_and_open() {
        let mut component = StackLayers::new();
        let state = app_state(vec![stack_summary("a", 1)], Screen::Layers(0));

        let auto = component.handle_key(key(KeyCode::Char('t')), &state);
        assert!(matches!(auto.as_slice(), [Action::ToggleSubmitAuto]));

        let open = component.handle_key(key(KeyCode::Char('p')), &state);
        assert!(matches!(open.as_slice(), [Action::ToggleSubmitOpen]));
    }

    #[test]
    fn handle_key_while_submitting_only_allows_dismissal() {
        let mut component = StackLayers::new();
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

    #[test]
    fn submit_progress_reports_partial_failure_per_layer_in_rendered_lines() {
        let mut progress =
            SubmitProgress::new(0, vec!["a-layer-0".to_string(), "a-layer-1".to_string()]);
        progress.set_status(0, LayerSubmitStatus::PullRequestCreated { number: 10 });
        progress.set_status(1, LayerSubmitStatus::Failed("push rejected".to_string()));
        progress.finished = true;

        let lines = progress
            .layers
            .iter()
            .map(submit_layer_line)
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>();

        assert!(lines[0].contains("PR #10 created"));
        assert!(lines[1].contains("failed: push rejected"));
    }
}
