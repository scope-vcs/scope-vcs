use super::{
    dependencies::DependencySummary,
    state::{
        ChangeListKind, DependencyPathSide, ReviewInput, ReviewMode, ReviewRow, ReviewState,
        ReviewStateAction,
    },
};
use crate::local_dependency_analysis::AnalysisJob;
use anyhow::Context;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{prelude::*, widgets::Paragraph};
use scope_domain::repo_config::RepoConfig;
use std::{io, time::Duration};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TuiOutcome {
    Exit,
    Cancel,
    ContinuePush,
}

pub fn run_review_tui(
    mut state: ReviewState,
    mut analysis_job: Option<AnalysisJob>,
    mut save_config: impl FnMut(&RepoConfig) -> anyhow::Result<()>,
) -> anyhow::Result<TuiOutcome> {
    enable_raw_mode().context("enable terminal raw mode")?;
    let _guard = TerminalRestoreGuard;
    execute!(io::stdout(), EnterAlternateScreen).context("enter alternate terminal screen")?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend).context("open terminal UI")?;

    loop {
        if let Some(result) = analysis_job.as_mut().and_then(AnalysisJob::try_result) {
            state.complete_dependency_analysis(result);
        }
        terminal
            .draw(|frame| render(frame, &mut state))
            .context("draw Scope review UI")?;

        if !event::poll(Duration::from_millis(100)).context("poll terminal input")? {
            continue;
        }
        let Event::Key(key) = event::read().context("read terminal input")? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }

        let Some(input) = key_to_input(&state, key) else {
            continue;
        };
        match state.handle_input(input) {
            ReviewStateAction::None => {}
            ReviewStateAction::Save => {
                if state.is_dirty() {
                    save_config(state.config())?;
                    state.mark_saved();
                }
            }
            ReviewStateAction::ContinuePush => {
                if state.is_dirty() {
                    save_config(state.config())?;
                }
                return Ok(TuiOutcome::ContinuePush);
            }
            ReviewStateAction::Exit => return Ok(TuiOutcome::Exit),
            ReviewStateAction::Cancel => return Ok(TuiOutcome::Cancel),
        }
    }
}

fn render(frame: &mut Frame<'_>, state: &mut ReviewState) {
    let area = frame.area();
    let width = area.width as usize;
    let footer_hints = footer_hints(state.mode(), width);
    let footer_height = footer_hints.len().saturating_add(1) as u16;
    let chunks = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(1),
        Constraint::Length(footer_height),
    ])
    .split(area);

    let mode = match state.mode() {
        ReviewMode::Standalone => "scope visibility edit",
        ReviewMode::Push => "scope push review",
    };
    let dirty = if state.is_dirty() {
        "modified"
    } else {
        "clean"
    };
    let filter = if state.filter().is_empty() {
        "filter: none".to_string()
    } else {
        format!("filter: {}", tui_escaped(state.filter()))
    };
    let filter_mode = if state.editing_filter() {
        " editing"
    } else {
        ""
    };
    let rewrite_note = if state.history_rewrite_count() == 0 {
        String::new()
    } else {
        format!(
            "  history rewrites: {} read-only",
            state.history_rewrite_count()
        )
    };
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(fit_cell(
                &format!("{mode}  Scope repo config  {dirty}{rewrite_note}"),
                width,
            )),
            review_header_line(&format!("{filter}{filter_mode}"), width),
            Line::from("─".repeat(width)),
        ]),
        chunks[0],
    );

    let body_height = chunks[1].height as usize;
    let read_only_lines = state
        .history_rewrite_summaries()
        .into_iter()
        .map(|line| Line::from(tui_escaped(&line)))
        .collect::<Vec<_>>();
    let (read_only_height, row_height) = review_body_heights(
        body_height,
        read_only_lines.len(),
        state.visible_row_count(),
    );
    state.adjust_scroll(row_height);
    let rows = state.visible_rows(state.scroll(), row_height);
    let mut lines = read_only_lines
        .into_iter()
        .take(read_only_height)
        .collect::<Vec<_>>();
    lines.extend(
        rows.iter()
            .enumerate()
            .map(|(index, row)| row_line(row, index + state.scroll() == state.cursor(), width)),
    );
    frame.render_widget(Paragraph::new(lines), chunks[1]);

    let mut footer_lines = footer_hints
        .into_iter()
        .map(|hint| Line::from(fit_cell(&hint, width)))
        .collect::<Vec<_>>();
    footer_lines.push(Line::from(fit_cell(&tui_escaped(state.message()), width)));
    frame.render_widget(Paragraph::new(footer_lines), chunks[2]);
}

fn review_body_heights(
    body_height: usize,
    read_only_line_count: usize,
    row_count: usize,
) -> (usize, usize) {
    let reserved_row_height = usize::from(body_height > 0 && row_count > 0);
    let read_only_height =
        read_only_line_count.min(body_height.saturating_sub(reserved_row_height));
    (
        read_only_height,
        body_height.saturating_sub(read_only_height),
    )
}

fn row_line(row: &ReviewRow, selected: bool, width: usize) -> Line<'static> {
    let (line, select_entire_row) = match row {
        ReviewRow::DependencySummary(summary) => (dependency_summary_line(summary, width), true),
        ReviewRow::DependencyFinding {
            source_path,
            target_path,
            selected_side,
        } => (
            dependency_finding_line(source_path, target_path, *selected_side, selected, width),
            false,
        ),
        ReviewRow::DependencyGap(gap) => (
            Line::from(Span::styled(
                fit_cell(
                    &format!(
                        "    gap  {}  {}",
                        tui_escaped(&gap.path),
                        tui_escaped(&gap.reason)
                    ),
                    width,
                ),
                Style::new().fg(Color::Yellow),
            )),
            true,
        ),
        ReviewRow::DependencyCoverage {
            analyzed_file_count,
            unsupported_file_count,
        } => (
            Line::from(Span::styled(
                fit_cell(
                    &format!(
                        "    Checked {analyzed_file_count} JS/TS file{}; {unsupported_file_count} other source file{} not checked",
                        if *analyzed_file_count == 1 { "" } else { "s" },
                        if *unsupported_file_count == 1 {
                            ""
                        } else {
                            "s"
                        },
                    ),
                    width,
                ),
                Style::new().fg(Color::DarkGray),
            )),
            true,
        ),
        ReviewRow::ChangeSection {
            kind,
            count,
            expanded,
        } => (change_section_line(*kind, *count, *expanded, width), true),
        ReviewRow::ChangePath { kind, path } => (
            Line::from(Span::styled(
                fit_cell(&format!("    {}", tui_escaped(path)), width),
                change_list_style(*kind),
            )),
            true,
        ),
        ReviewRow::TreeNode {
            depth,
            name,
            kind,
            expanded,
            visibility,
            rule,
            reserved,
            change_status,
            ..
        } => {
            let symbol = match kind {
                super::tree::ReviewNodeKind::Root => " . ",
                super::tree::ReviewNodeKind::File => "   ",
                super::tree::ReviewNodeKind::Directory if *expanded => "[v]",
                super::tree::ReviewNodeKind::Directory => "[>]",
            };
            let path = format!("{}{symbol} {}", "  ".repeat(*depth), tui_escaped(name));
            let change = change_status
                .as_deref()
                .filter(|status| !status.starts_with(['A', 'D']))
                .map(|status| format!("  {}", tui_escaped(status)))
                .unwrap_or_default();
            let reserved = if *reserved { " reserved" } else { "" };
            let detail = format!("{}{}{}", tui_escaped(rule), reserved, change);
            (tree_line(&path, *visibility, &detail, width), true)
        }
    };
    if selected && select_entire_row {
        line.style(Style::new().add_modifier(Modifier::REVERSED))
    } else {
        line
    }
}

fn dependency_summary_line(summary: &DependencySummary, width: usize) -> Line<'static> {
    let symbol = if !summary.expandable {
        " • "
    } else if summary.expanded {
        "[v]"
    } else {
        "[>]"
    };
    let meta = summary
        .meta
        .as_deref()
        .map(|meta| format!("  {meta}"))
        .unwrap_or_default();
    let style = if summary.warning {
        Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD)
    } else {
        Style::new().fg(Color::DarkGray)
    };
    Line::from(Span::styled(
        fit_cell(&format!("{symbol} {}{meta}", summary.label), width),
        style,
    ))
}

fn dependency_finding_line(
    source_path: &str,
    target_path: &str,
    selected_side: DependencyPathSide,
    selected: bool,
    width: usize,
) -> Line<'static> {
    let arrow_width = 3.min(width);
    let path_width = width.saturating_sub(arrow_width) / 2;
    let target_width = width.saturating_sub(arrow_width + path_width);
    let public_style = Style::new().fg(Color::Green);
    let private_style = Style::new().fg(Color::Red);
    let selected_modifiers = Modifier::BOLD | Modifier::UNDERLINED;
    let source_style = if selected && selected_side == DependencyPathSide::PublicSource {
        public_style.add_modifier(selected_modifiers)
    } else {
        public_style
    };
    let target_style = if selected && selected_side == DependencyPathSide::PrivateTarget {
        private_style.add_modifier(selected_modifiers)
    } else {
        private_style
    };
    Line::from(vec![
        Span::styled(
            fit_cell(&format!("public {}", tui_escaped(source_path)), path_width),
            source_style,
        ),
        Span::raw(fit_cell(" → ", arrow_width)),
        Span::styled(
            fit_cell(
                &format!("private {}", tui_escaped(target_path)),
                target_width,
            ),
            target_style,
        ),
    ])
}

fn review_header_line(filter: &str, width: usize) -> Line<'static> {
    let (path_width, visibility_width, detail_width) = row_column_widths(width);
    Line::from(vec![
        Span::raw(fit_cell("Path", path_width)),
        Span::raw(fit_cell("Visibility", visibility_width)),
        Span::raw(fit_cell(&format!("Rule  {filter}"), detail_width)),
    ])
}

fn tree_line(
    path: &str,
    visibility: scope_domain::repo_visibility::ReviewVisibility,
    detail: &str,
    width: usize,
) -> Line<'static> {
    let (path_width, visibility_width, detail_width) = row_column_widths(width);
    let (visibility_text, visibility_style) = visibility_cell(visibility);
    Line::from(vec![
        Span::raw(fit_cell(path, path_width)),
        Span::styled(
            fit_cell(visibility_text, visibility_width),
            visibility_style,
        ),
        Span::raw(fit_cell(detail, detail_width)),
    ])
}

fn change_section_line(
    kind: ChangeListKind,
    count: usize,
    expanded: bool,
    width: usize,
) -> Line<'static> {
    let symbol = if expanded { "[v]" } else { "[>]" };
    Line::from(Span::styled(
        fit_cell(
            &format!("{symbol} {} files ({count})", change_list_label(kind)),
            width,
        ),
        change_list_style(kind).add_modifier(Modifier::BOLD),
    ))
}

fn change_list_label(kind: ChangeListKind) -> &'static str {
    match kind {
        ChangeListKind::Added => "Added",
        ChangeListKind::Deleted => "Deleted",
    }
}

fn change_list_style(kind: ChangeListKind) -> Style {
    match kind {
        ChangeListKind::Added => Style::new().fg(Color::Green),
        ChangeListKind::Deleted => Style::new().fg(Color::Red),
    }
}

fn visibility_cell(
    visibility: scope_domain::repo_visibility::ReviewVisibility,
) -> (&'static str, Style) {
    use scope_domain::repo_visibility::ReviewVisibility;

    match visibility {
        ReviewVisibility::Public => ("🌐 public", Style::new().fg(Color::Green)),
        ReviewVisibility::Private => ("🔒 private", Style::new().fg(Color::Red)),
        ReviewVisibility::Mixed => ("− mixed", Style::new().fg(Color::Yellow)),
    }
}

fn row_column_widths(width: usize) -> (usize, usize, usize) {
    let path_width = (width / 2).clamp(12, 52).min(width);
    let visibility_width = 14.min(width.saturating_sub(path_width));
    let detail_width = width.saturating_sub(path_width + visibility_width);
    (path_width, visibility_width, detail_width)
}

fn footer_hints(mode: ReviewMode, width: usize) -> Vec<String> {
    let full = match mode {
        ReviewMode::Push => {
            "Arrows navigate  Space toggle/open  D dependencies  S save  P push  Q cancel  / filter  ? help"
        }
        ReviewMode::Standalone => "Arrows navigate  Space toggle  S save  Q quit  / filter  ? help",
    };
    if UnicodeWidthStr::width(full) <= width {
        return vec![full.to_string()];
    }

    let controls: &[&str] = match mode {
        ReviewMode::Push => &[
            "↑↓←→ move",
            "Space toggle",
            "D dependencies",
            "S save",
            "P push",
            "Q cancel",
            "/ filter",
            "? help",
        ],
        ReviewMode::Standalone => &[
            "↑↓←→ move",
            "Space toggle",
            "S save",
            "Q quit",
            "/ filter",
            "? help",
        ],
    };
    let mut lines = Vec::new();
    let mut line = String::new();
    for control in controls {
        let candidate = if line.is_empty() {
            (*control).to_string()
        } else {
            format!("{line} {control}")
        };
        if !line.is_empty() && UnicodeWidthStr::width(candidate.as_str()) > width {
            lines.push(line);
            line = (*control).to_string();
        } else {
            line = candidate;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn fit_cell(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let text_width = UnicodeWidthStr::width(text);
    if text_width <= width {
        return format!("{text}{}", " ".repeat(width - text_width));
    }

    let target_width = width.saturating_sub(1);
    let mut clipped = String::new();
    let mut clipped_width = 0;
    for value in text.chars() {
        let value_width = UnicodeWidthChar::width(value).unwrap_or(0);
        if clipped_width + value_width > target_width {
            break;
        }
        clipped.push(value);
        clipped_width += value_width;
    }
    clipped.push('…');
    clipped.push_str(&" ".repeat(width.saturating_sub(clipped_width + 1)));
    clipped
}

fn tui_escaped(text: &str) -> String {
    text.chars().flat_map(char::escape_default).collect()
}

fn key_to_input(state: &ReviewState, key: KeyEvent) -> Option<ReviewInput> {
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        return Some(ReviewInput::Cancel);
    }
    if state.editing_filter() {
        return match key.code {
            KeyCode::Esc | KeyCode::Enter => Some(ReviewInput::Escape),
            KeyCode::Backspace => Some(ReviewInput::Backspace),
            KeyCode::Char(value) => Some(ReviewInput::Char(value)),
            _ => None,
        };
    }

    match key.code {
        KeyCode::Up => Some(ReviewInput::Up),
        KeyCode::Down => Some(ReviewInput::Down),
        KeyCode::Left => Some(ReviewInput::Left),
        KeyCode::Right => Some(ReviewInput::Right),
        KeyCode::Char(' ') => Some(ReviewInput::Toggle),
        KeyCode::Char('s') | KeyCode::Char('S') => Some(ReviewInput::Save),
        KeyCode::Char('p') | KeyCode::Char('P') => Some(ReviewInput::ContinuePush),
        KeyCode::Char('q') | KeyCode::Char('Q') => Some(ReviewInput::Quit),
        KeyCode::Char('/') => Some(ReviewInput::Filter),
        KeyCode::Char('?') => Some(ReviewInput::Help),
        KeyCode::Char('d') | KeyCode::Char('D') => Some(ReviewInput::Dependencies),
        KeyCode::Esc => Some(ReviewInput::Escape),
        _ => None,
    }
}

struct TerminalRestoreGuard;

impl Drop for TerminalRestoreGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
