use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Margin, Position, Rect},
    prelude::Stylize,
    style::{Color, Modifier, Style},
    text::{Line, Span, Text},
    widgets::{
        Block, Borders, Clear, List, ListItem, ListState, Paragraph, Scrollbar,
        ScrollbarOrientation, ScrollbarState, Wrap,
    },
};

use crate::{
    app::{App, InputMode},
    tree::{NodeId, NodeKind},
};
use serde_json::Value;

const ACCENT: Color = Color::Cyan;
const MUTED: Color = Color::DarkGray;
const BREADCRUMB_SEPARATOR: &str = " › ";
const HIDDEN_BREADCRUMBS: &str = "… › ";
const FILTER_SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = page_areas(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_body(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    if app.input_mode == InputMode::Filter {
        draw_filter_overlay(frame, app, chunks[1]);
    }

    if app.show_help {
        draw_help(frame);
    }
}

fn page_areas(area: Rect) -> std::rc::Rc<[Rect]> {
    Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(area)
}

pub(crate) fn body_area(area: Rect) -> Rect {
    page_areas(area)[1]
}

fn draw_header(frame: &mut Frame, app: &App, area: Rect) {
    let node = app.tree.node(app.selected);
    let path = app.tree.path(app.selected);
    let mut spans = vec![
        Span::styled(" jex ", Style::default().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(Color::White).bold()),
        Span::styled(
            format!("  ·  {}  ·  depth {}", node.kind.name(), node.depth),
            Style::default().fg(MUTED),
        ),
    ];
    if let Some(expression) = &app.active_filter {
        let count = app.filter_output_count.unwrap_or(0);
        spans.push(Span::styled("  ·  jq ", Style::default().fg(MUTED)));
        spans.push(Span::styled(
            expression.clone(),
            Style::default().fg(Color::Magenta),
        ));
        spans.push(Span::styled(
            format!(
                "  ·  {count} {}",
                if count == 1 { "output" } else { "outputs" }
            ),
            Style::default().fg(MUTED),
        ));
    }
    let header =
        Paragraph::new(Line::from(spans)).block(Block::default().borders(Borders::BOTTOM).title(
            Span::styled(format!(" {} ", app.source), Style::default().fg(MUTED)),
        ));
    frame.render_widget(header, area);
}

fn draw_body(frame: &mut Frame, app: &App, area: Rect) {
    let columns = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Length(app.tree_pane_width(area.width)),
            Constraint::Min(0),
        ])
        .split(area);
    draw_tree(frame, app, columns[0]);
    draw_preview(frame, app, columns[1]);
}

fn draw_tree(frame: &mut Frame, app: &App, area: Rect) {
    let selected = app.selected_visible_index();
    let viewport_height = usize::from(tree_items_area(area).height);
    let offset = selected.saturating_sub(viewport_height.saturating_sub(1));
    let end = offset
        .saturating_add(viewport_height)
        .min(app.visible.len());
    let items = app.visible[offset..end]
        .iter()
        .map(|&id| tree_item(app, id))
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(selected.saturating_sub(offset)));
    let list = List::new(items)
        .block(tree_block(app))
        .highlight_style(Style::default().bg(Color::Rgb(34, 50, 60)).fg(Color::White))
        .highlight_symbol("▌");
    frame.render_stateful_widget(list, area, &mut state);

    let mut scrollbar_state = ScrollbarState::new(app.visible.len()).position(selected);
    let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None)
        .track_symbol(None);
    frame.render_stateful_widget(
        scrollbar,
        area.inner(Margin {
            vertical: 1,
            horizontal: 0,
        }),
        &mut scrollbar_state,
    );
}

fn tree_block(app: &App) -> Block<'static> {
    Block::default()
        .borders(Borders::RIGHT)
        .border_style(if app.is_dragging_divider() {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(MUTED)
        })
        .title(format!(" Tree  {}/{} ", app.visible.len(), app.tree.len()))
}

pub(crate) fn tree_items_area(area: Rect) -> Rect {
    // Keep mouse hit-testing aligned with the exact inset Ratatui applies for the
    // tree block's top title and right border.
    Block::default()
        .borders(Borders::RIGHT)
        .title(" ")
        .inner(area)
}

fn tree_item(app: &App, id: NodeId) -> ListItem<'static> {
    let node = app.tree.node(id);
    let indent = "  ".repeat(node.depth);
    let disclosure = if node.children.is_empty() {
        "•"
    } else if node.expanded {
        "▾"
    } else {
        "▸"
    };
    let is_match = app.is_match(id);
    let label_style = kind_style(node.kind).add_modifier(if is_match {
        Modifier::UNDERLINED
    } else {
        Modifier::empty()
    });
    ListItem::new(Line::from(vec![
        Span::raw(indent),
        Span::styled(format!("{disclosure} "), Style::default().fg(MUTED)),
        Span::styled(app.tree.label(id), label_style),
        Span::styled(format!("  {}", node.summary), Style::default().fg(MUTED)),
    ]))
}

fn draw_preview(frame: &mut Frame, app: &App, area: Rect) {
    let (header_area, preview_area) = preview_areas(area);
    frame.render_widget(preview_header(app, header_area), header_area);

    let content_height = app.tree.pretty_line_count(app.selected);
    let maximum_scroll = content_height.saturating_sub(usize::from(preview_area.height));
    let scroll = app.preview_scroll.min(maximum_scroll);
    let preview = preview_text(app, scroll, usize::from(preview_area.height));
    frame.render_widget(
        Paragraph::new(preview).wrap(Wrap { trim: false }),
        preview_area,
    );

    if maximum_scroll > 0 {
        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(scroll)
            .viewport_content_length(usize::from(preview_area.height));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn preview_areas(area: Rect) -> (Rect, Rect) {
    let details = area.inner(Margin {
        vertical: 0,
        horizontal: 1,
    });
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(details);
    (rows[0], rows[1])
}

fn preview_header(app: &App, area: Rect) -> Paragraph<'static> {
    Paragraph::new(breadcrumb_line(app, area.width).0)
}

struct BreadcrumbTarget {
    id: NodeId,
    start: u16,
    end: u16,
}

fn breadcrumb_line(app: &App, width: u16) -> (Line<'static>, Vec<BreadcrumbTarget>) {
    let lineage = app.tree.lineage(app.selected);
    if lineage.is_empty() || width == 0 {
        return (Line::default(), Vec::new());
    }

    let labels = lineage
        .iter()
        .map(|&id| {
            let label = app.tree.label(id);
            if label.is_empty() {
                "\"\"".into()
            } else {
                label
            }
        })
        .collect::<Vec<_>>();
    let available = usize::from(width);
    let full_width = labels.iter().map(|label| text_width(label)).sum::<usize>()
        + text_width(BREADCRUMB_SEPARATOR) * labels.len().saturating_sub(1);

    let (start, hidden_prefix) = if full_width <= available {
        (0, false)
    } else if available <= text_width(HIDDEN_BREADCRUMBS) {
        (labels.len() - 1, false)
    } else {
        let suffix_width = available - text_width(HIDDEN_BREADCRUMBS);
        let mut start = labels.len() - 1;
        while start > 0 {
            let candidate_width = labels[start - 1..]
                .iter()
                .map(|label| text_width(label))
                .sum::<usize>()
                + text_width(BREADCRUMB_SEPARATOR) * (labels.len() - start);
            if candidate_width > suffix_width {
                break;
            }
            start -= 1;
        }
        (start, true)
    };

    let mut spans = Vec::new();
    let mut targets = Vec::new();
    let mut column = 0usize;
    if hidden_prefix {
        spans.push(Span::styled(HIDDEN_BREADCRUMBS, Style::default().fg(MUTED)));
        column += text_width(HIDDEN_BREADCRUMBS);
    }

    for (visible_index, index) in (start..labels.len()).enumerate() {
        if visible_index > 0 {
            spans.push(Span::styled(
                BREADCRUMB_SEPARATOR,
                Style::default().fg(MUTED),
            ));
            column += text_width(BREADCRUMB_SEPARATOR);
        }

        let remaining = available.saturating_sub(column);
        let label = truncate_to_width(&labels[index], remaining);
        let label_width = text_width(&label);
        let is_current = lineage[index] == app.selected;
        let style = if is_current {
            Style::default().fg(Color::White).bold()
        } else {
            Style::default().fg(ACCENT).underlined()
        };
        spans.push(Span::styled(label, style));
        targets.push(BreadcrumbTarget {
            id: lineage[index],
            start: column.min(usize::from(u16::MAX)) as u16,
            end: column
                .saturating_add(label_width)
                .min(usize::from(u16::MAX)) as u16,
        });
        column += label_width;
    }

    (Line::from(spans), targets)
}

fn text_width(text: &str) -> usize {
    Span::raw(text.to_owned()).width()
}

fn truncate_to_width(text: &str, maximum: usize) -> String {
    if text_width(text) <= maximum {
        return text.to_owned();
    }
    if maximum == 0 {
        return String::new();
    }
    if maximum == 1 {
        return "…".into();
    }

    let mut shortened = String::new();
    let content_width = maximum - 1;
    let mut used = 0;
    for ch in text.chars() {
        let char_width = text_width(&ch.to_string());
        if used + char_width > content_width {
            break;
        }
        shortened.push(ch);
        used += char_width;
    }
    shortened.push('…');
    shortened
}

pub(crate) fn breadcrumb_target_at(app: &App, area: Rect, position: Position) -> Option<NodeId> {
    let (header, _) = preview_areas(area);
    if header.height == 0
        || position.y != header.y
        || position.x < header.x
        || position.x >= header.right()
    {
        return None;
    }

    let column = position.x.saturating_sub(header.x);
    breadcrumb_line(app, header.width)
        .1
        .into_iter()
        .find(|target| column >= target.start && column < target.end)
        .map(|target| target.id)
}

pub(crate) fn preview_max_scroll(app: &App, area: Rect) -> usize {
    let (_, area) = preview_areas(area);
    app.tree
        .pretty_line_count(app.selected)
        .saturating_sub(usize::from(area.height))
}

fn preview_text(app: &App, start: usize, height: usize) -> Text<'static> {
    let mut collector = PreviewCollector {
        skip: start,
        limit: height,
        lines: Vec::with_capacity(height),
    };
    render_json_node(app, app.selected, 0, Vec::new(), false, &mut collector);
    Text::from(
        collector
            .lines
            .into_iter()
            .map(Line::from)
            .collect::<Vec<_>>(),
    )
}

struct PreviewCollector {
    skip: usize,
    limit: usize,
    lines: Vec<Vec<Span<'static>>>,
}

impl PreviewCollector {
    fn is_full(&self) -> bool {
        self.lines.len() >= self.limit
    }

    fn emit(&mut self, line: Vec<Span<'static>>) {
        if self.skip > 0 {
            self.skip -= 1;
        } else if !self.is_full() {
            self.lines.push(line);
        }
    }
}

fn render_json_node(
    app: &App,
    id: NodeId,
    depth: usize,
    mut prefix: Vec<Span<'static>>,
    trailing_comma: bool,
    collector: &mut PreviewCollector,
) {
    if collector.is_full() {
        return;
    }
    let line_count = app.tree.pretty_line_count(id);
    if collector.skip >= line_count {
        collector.skip -= line_count;
        return;
    }

    let node = app.tree.node(id);
    if node.children.is_empty() {
        let value = app.tree.value_at(id);
        let (content, style) = match value {
            Value::Object(_) => ("{}".into(), punctuation_style()),
            Value::Array(_) => ("[]".into(), punctuation_style()),
            Value::String(value) => (
                serde_json::to_string(value).expect("JSON strings are serializable"),
                kind_style(NodeKind::String),
            ),
            Value::Number(value) => (value.to_string(), kind_style(NodeKind::Number)),
            Value::Bool(value) => (value.to_string(), kind_style(NodeKind::Bool)),
            Value::Null => ("null".into(), kind_style(NodeKind::Null)),
        };
        prefix.push(Span::styled(content, style));
        if trailing_comma {
            prefix.push(Span::styled(",", punctuation_style()));
        }
        collector.emit(prefix);
        return;
    }

    let (opening, closing) = match node.kind {
        NodeKind::Object => ("{", "}"),
        NodeKind::Array => ("[", "]"),
        _ => unreachable!("only containers have child nodes"),
    };
    prefix.push(Span::styled(opening, punctuation_style()));
    collector.emit(prefix);

    let last = node.children.len() - 1;
    let first_child = if collector.skip == 0 {
        0
    } else {
        let children_start = app.tree.pretty_line_start(id) + 1;
        let target = children_start + collector.skip;
        let index = node.children.partition_point(|&child| {
            app.tree.pretty_line_start(child) + app.tree.pretty_line_count(child) <= target
        });
        let skipped_lines = if index < node.children.len() {
            app.tree.pretty_line_start(node.children[index]) - children_start
        } else {
            line_count - 2
        };
        collector.skip -= skipped_lines;
        index
    };
    for (index, &child) in node.children.iter().enumerate().skip(first_child) {
        let mut child_prefix = vec![Span::raw("  ".repeat(depth + 1))];
        if node.kind == NodeKind::Object {
            let crate::tree::Segment::Key(key) = app
                .tree
                .node(child)
                .segment
                .as_ref()
                .expect("object children have key segments")
            else {
                unreachable!("object children have key segments");
            };
            child_prefix.push(Span::styled(
                serde_json::to_string(key).expect("JSON object keys are serializable"),
                Style::default().fg(ACCENT),
            ));
            child_prefix.push(Span::styled(": ", punctuation_style()));
        }
        render_json_node(
            app,
            child,
            depth + 1,
            child_prefix,
            index != last,
            collector,
        );
        if collector.is_full() {
            return;
        }
    }

    let mut closing_line = vec![
        Span::raw("  ".repeat(depth)),
        Span::styled(closing, punctuation_style()),
    ];
    if trailing_comma {
        closing_line.push(Span::styled(",", punctuation_style()));
    }
    collector.emit(closing_line);
}

fn punctuation_style() -> Style {
    Style::default().fg(Color::Gray)
}

fn draw_footer(frame: &mut Frame, app: &App, area: Rect) {
    let lines = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    let (prefix, text, style) = match app.input_mode {
        InputMode::Search => ("/", app.input.clone(), Style::default().fg(Color::Yellow)),
        InputMode::Jump => (":", app.input.clone(), Style::default().fg(ACCENT)),
        InputMode::Filter => ("", String::new(), Style::default().fg(Color::Magenta)),
        InputMode::Normal => {
            let status = app
                .message
                .as_deref()
                .map(str::to_owned)
                .or_else(|| {
                    app.search_query.as_ref().map(|query| {
                        let current = app.match_index.map(|index| index + 1).unwrap_or(0);
                        format!("/{query}  ·  {current}/{} matches", app.matches.len())
                    })
                })
                .unwrap_or_default();
            ("", status, Style::default().fg(Color::Yellow))
        }
    };
    let (text, cursor_column) = if app.input_mode == InputMode::Normal {
        (text, 0)
    } else {
        prompt_view(
            &text,
            app.input_cursor,
            usize::from(lines[0].width.saturating_sub(1)),
        )
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
            Span::styled(text, style),
        ])),
        lines[0],
    );
    let hint = if app.input_mode == InputMode::Filter {
        "↑↓/PgUp/PgDn scroll preview   Enter apply   Esc cancel"
    } else {
        "↑↓/jk move   ←→/hl structure   / search   : path   | jq   p print   ? help"
    };
    let hint_style = Style::default().fg(MUTED);
    frame.render_widget(Paragraph::new(hint).style(hint_style), lines[1]);

    if matches!(app.input_mode, InputMode::Search | InputMode::Jump) {
        let cursor_x = lines[0].x + 1 + cursor_column;
        frame.set_cursor_position(Position::new(
            cursor_x.min(lines[0].right() - 1),
            lines[0].y,
        ));
    }
}

fn draw_filter_overlay(frame: &mut Frame, app: &App, body: Rect) {
    let area = filter_overlay_area(body);
    frame.render_widget(Clear, area);

    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta))
        .title_top(Line::from(Span::styled(
            " jq filter ",
            Style::default().fg(Color::Magenta).bold(),
        )));
    if app.is_filter_preview_pending() {
        let spinner = FILTER_SPINNER[app.filter_spinner_frame() % FILTER_SPINNER.len()];
        block = block.title_top(
            Line::from(Span::styled(
                format!(" {spinner} "),
                Style::default().fg(Color::Yellow),
            ))
            .right_aligned(),
        );
    } else if let Some((_, count)) = app.filter_preview() {
        block = block.title_top(
            Line::from(Span::styled(
                format!(
                    " {count} {} ",
                    if count == 1 { "output" } else { "outputs" }
                ),
                Style::default().fg(MUTED),
            ))
            .right_aligned(),
        );
    }
    if let Some(message) = app.message.as_deref() {
        block = block.title_bottom(Line::from(Span::styled(
            format!(" {message} "),
            Style::default().fg(Color::Red),
        )));
    }
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(inner);
    let available = usize::from(rows[0].width.saturating_sub(2));
    let (input, cursor_column) = prompt_view(&app.input, app.input_cursor, available);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("| ", Style::default().fg(Color::Magenta).bold()),
            Span::styled(input, Style::default().fg(Color::White)),
        ])),
        rows[0],
    );

    if app.filter_preview().is_some() {
        let maximum_lines = usize::from(rows[1].height);
        let all_lines = app.filter_preview_lines().unwrap_or_default();
        let total_lines = all_lines.len();
        let maximum_scroll = total_lines.saturating_sub(maximum_lines);
        let scroll = app.filter_preview_scroll().min(maximum_scroll);
        let lines = all_lines
            .iter()
            .skip(scroll)
            .take(maximum_lines)
            .map(|line| Line::from(Span::styled(line.clone(), Style::default().fg(Color::Gray))))
            .collect::<Vec<_>>();
        frame.render_widget(Paragraph::new(lines), rows[1]);
        if maximum_scroll > 0 {
            let mut scrollbar_state = ScrollbarState::new(total_lines)
                .position(scroll)
                .viewport_content_length(maximum_lines);
            let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
                .begin_symbol(None)
                .end_symbol(None)
                .track_symbol(None);
            frame.render_stateful_widget(scrollbar, rows[1], &mut scrollbar_state);
        }
    }

    let cursor_x = rows[0]
        .x
        .saturating_add(2)
        .saturating_add(cursor_column)
        .min(rows[0].right().saturating_sub(1));
    frame.set_cursor_position(Position::new(cursor_x, rows[0].y));
}

pub(crate) fn filter_overlay_area(body: Rect) -> Rect {
    let available_width = body.width.saturating_sub(2);
    let preferred_width = (u32::from(body.width) * 78 / 100) as u16;
    let width = preferred_width
        .max(40.min(available_width))
        .min(available_width);
    let height = (body.height / 2).clamp(7.min(body.height), 12.min(body.height));
    Rect::new(
        body.x.saturating_add(body.width.saturating_sub(width) / 2),
        body.bottom().saturating_sub(height),
        width,
        height,
    )
}

fn prompt_view(text: &str, cursor: usize, available: usize) -> (String, u16) {
    if available == 0 {
        return (String::new(), 0);
    }

    let chars = text.chars().collect::<Vec<_>>();
    let cursor = cursor.min(chars.len());
    let mut start = 0;
    while start < cursor
        && text_width(&chars[start..cursor].iter().collect::<String>()) >= available
    {
        start += 1;
    }

    let mut shown = String::new();
    for &ch in &chars[start..] {
        let mut candidate = shown.clone();
        candidate.push(ch);
        if text_width(&candidate) > available {
            break;
        }
        shown.push(ch);
    }
    let before_cursor = chars[start..cursor].iter().collect::<String>();
    (
        shown,
        text_width(&before_cursor).min(usize::from(u16::MAX)) as u16,
    )
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(68, 35, frame.area());
    frame.render_widget(Clear, area);
    let help = vec![
        Line::from(Span::styled("Navigate", Style::default().fg(ACCENT).bold())),
        Line::from("  j/k, ↑/↓       previous / next visible node"),
        Line::from("  h/l, ←/→       collapse or parent / expand or child"),
        Line::from("  [/ ]           previous / next sibling"),
        Line::from("  g/G             first / last visible node"),
        Line::from("  Ctrl-u/Ctrl-d   move by a page"),
        Line::from(""),
        Line::from(Span::styled(
            "Find your place",
            Style::default().fg(ACCENT).bold(),
        )),
        Line::from("  /               search keys, paths, and values"),
        Line::from("  n/N             next / previous search match"),
        Line::from("  :               jump to JSON Pointer (/users/0/name)"),
        Line::from("  b/f             back / forward through jumps"),
        Line::from("  m / '           set / return to a bookmark"),
        Line::from(""),
        Line::from(Span::styled(
            "Filter with jq",
            Style::default().fg(ACCENT).bold(),
        )),
        Line::from("  |               open the jq editor with live preview"),
        Line::from("  ↑↓ / PgUp/PgDn scroll the live result preview"),
        Line::from("  Enter / Esc     apply / dismiss the overlay result"),
        Line::from("  Esc             clear an applied filter (after search)"),
        Line::from(""),
        Line::from(Span::styled(
            "Shape the tree",
            Style::default().fg(ACCENT).bold(),
        )),
        Line::from("  Space/Enter     expand or collapse"),
        Line::from("  e/c             expand / collapse the entire branch"),
        Line::from("  -/+             resize the tree and value panes"),
        Line::from("  Mouse click     select a tree row or navigate a breadcrumb"),
        Line::from("  Double-click    expand or collapse a container row"),
        Line::from("  Mouse wheel     move the tree or scroll the hovered value"),
        Line::from("  Mouse drag      resize using the pane divider"),
        Line::from("  Esc             clear the active search"),
        Line::from("  p / P           print selected value / path and quit"),
        Line::from("  q, Ctrl-c       quit"),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to close",
            Style::default().fg(MUTED),
        )),
    ];
    frame.render_widget(
        Paragraph::new(help).block(Block::default().borders(Borders::ALL).title(" jex help ")),
        area,
    );
}

fn centered_rect(percent_x: u16, height: u16, area: Rect) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height.min(area.height)),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn kind_style(kind: NodeKind) -> Style {
    let color = match kind {
        NodeKind::Object | NodeKind::Array => ACCENT,
        NodeKind::String => Color::Green,
        NodeKind::Number => Color::Magenta,
        NodeKind::Bool => Color::Yellow,
        NodeKind::Null => MUTED,
    };
    Style::default().fg(color)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::{Terminal, backend::TestBackend};
    use serde_json::json;

    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect()
    }

    #[test]
    fn highlighted_json_preserves_pretty_printed_content() {
        let value = json!({"name": "Ada\nLovelace", "active": true, "score": 42, "other": null});
        let app = App::new(value.clone(), "test".into(), 0);
        let text = preview_text(&app, 0, app.tree.pretty_line_count(0));
        let rendered = text
            .lines
            .iter()
            .map(|line| {
                line.spans
                    .iter()
                    .map(|span| span.content.as_ref())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(rendered, serde_json::to_string_pretty(&value).unwrap());
    }

    #[test]
    fn highlighted_json_styles_each_token_kind() {
        let app = App::new(
            json!({"text": "hello", "number": 7, "bool": false, "nil": null}),
            "test".into(),
            0,
        );
        let text = preview_text(&app, 0, app.tree.pretty_line_count(0));
        let spans = text.lines.iter().flat_map(|line| &line.spans);
        let style_for = |needle: &str| {
            spans
                .clone()
                .find(|span| span.content == needle)
                .and_then(|span| span.style.fg)
        };

        assert_eq!(style_for("\"text\""), Some(ACCENT));
        assert_eq!(style_for("\"hello\""), Some(Color::Green));
        assert_eq!(style_for("7"), Some(Color::Magenta));
        assert_eq!(style_for("false"), Some(Color::Yellow));
        assert_eq!(style_for("null"), Some(MUTED));
        assert_eq!(style_for("{"), Some(Color::Gray));
    }

    #[test]
    fn preview_generation_returns_only_the_requested_window() {
        let value = json!({
            "alpha": [1, 2, 3],
            "beta": {"nested": true},
            "gamma": null,
        });
        let expected = serde_json::to_string_pretty(&value)
            .unwrap()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<_>>();
        let app = App::new(value, "test".into(), 0);

        let text = preview_text(&app, 3, 4);
        let actual = text.lines.iter().map(line_text).collect::<Vec<_>>();

        assert_eq!(actual, expected[3..7]);
    }

    #[test]
    fn preview_scroll_supports_documents_longer_than_u16() {
        let value = Value::Array((0..70_000).map(Value::from).collect());
        let app = App::new(value, "test".into(), 0);

        let maximum = preview_max_scroll(&app, Rect::new(0, 0, 80, 20));
        let tail_start = app.tree.pretty_line_count(0) - 3;
        let tail = preview_text(&app, tail_start, 3)
            .lines
            .iter()
            .map(line_text)
            .collect::<Vec<_>>();

        assert!(maximum > usize::from(u16::MAX));
        assert_eq!(tail, ["  69998,", "  69999", "]"]);
    }

    #[test]
    fn breadcrumbs_keep_the_selected_node_visible_in_a_narrow_pane() {
        let mut app = App::new(json!({"alpha": {"beta": {"gamma": 1}}}), "test".into(), 0);
        app.selected = app.tree.find_pointer("/alpha/beta/gamma").unwrap();

        let (line, targets) = breadcrumb_line(&app, 10);

        assert_eq!(line_text(&line), "… › gamma");
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, app.selected);
        assert!(line.width() <= 10);
    }

    #[test]
    fn breadcrumb_hit_testing_only_selects_labels() {
        let mut app = App::new(json!({"alpha": {"beta": 1}}), "test".into(), 0);
        app.selected = app.tree.find_pointer("/alpha/beta").unwrap();
        let area = Rect::new(20, 5, 40, 10);

        assert_eq!(
            breadcrumb_target_at(&app, area, Position::new(25, 5)),
            app.tree.find_pointer("/alpha")
        );
        assert_eq!(breadcrumb_target_at(&app, area, Position::new(23, 5)), None);
        assert_eq!(breadcrumb_target_at(&app, area, Position::new(25, 6)), None);
    }

    #[test]
    fn long_filter_prompts_keep_the_cursor_visible() {
        let (shown, cursor) = prompt_view(".users[] | select(.active)", 27, 12);

        assert_eq!(shown, "ct(.active)");
        assert_eq!(usize::from(cursor), text_width(&shown));
    }

    #[test]
    fn filter_editor_is_an_overlay_that_leaves_the_document_visible() {
        let mut app = App::new(
            json!({"source_key": {"nested": true}, "another": 42}),
            "test".into(),
            1,
        );
        app.handle_key(KeyEvent::new(KeyCode::Char('|'), KeyModifiers::NONE));
        for ch in ".source_key".chars() {
            app.handle_key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
        }
        app.message = Some("syntax error: expected a key".into());
        let backend = TestBackend::new(100, 28);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal.draw(|frame| draw(frame, &app)).unwrap();

        let rendered = terminal
            .backend()
            .buffer()
            .content
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("source_key"));
        assert!(rendered.contains("jq filter"));
        assert!(!rendered.contains("live preview"));
        assert!(rendered.contains(" ⠋ "));
        assert!(!rendered.contains("Filtering"));
        assert!(rendered.contains(".source_key"));

        let overlay = filter_overlay_area(body_area(Rect::new(0, 0, 100, 28)));
        let bottom_border = (overlay.x..overlay.right())
            .map(|x| terminal.backend().buffer()[(x, overlay.bottom() - 1)].symbol())
            .collect::<String>();
        assert!(bottom_border.contains("syntax error: expected a key"));
    }
}
