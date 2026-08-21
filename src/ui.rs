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

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = page_areas(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_body(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

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
    let header = Paragraph::new(Line::from(vec![
        Span::styled(" jex ", Style::default().fg(Color::Black).bg(ACCENT).bold()),
        Span::raw("  "),
        Span::styled(path, Style::default().fg(Color::White).bold()),
        Span::styled(
            format!("  ·  {}  ·  depth {}", node.kind.name(), node.depth),
            Style::default().fg(MUTED),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::BOTTOM)
            .title(Span::styled(
                format!(" {} ", app.source),
                Style::default().fg(MUTED),
            )),
    );
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
    let items = app
        .visible
        .iter()
        .map(|&id| tree_item(app, id))
        .collect::<Vec<_>>();
    let mut state = ListState::default().with_selected(Some(app.selected_visible_index()));
    let list = List::new(items)
        .block(tree_block(app))
        .highlight_style(Style::default().bg(Color::Rgb(34, 50, 60)).fg(Color::White))
        .highlight_symbol("▌");
    frame.render_stateful_widget(list, area, &mut state);

    let mut scrollbar_state =
        ScrollbarState::new(app.visible.len()).position(app.selected_visible_index());
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
    let is_match = app.matches.contains(&id);
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

    let preview = preview_widget(app);
    let content_height = preview.line_count(preview_area.width);
    let maximum_scroll = content_height.saturating_sub(usize::from(preview_area.height));
    let scroll = usize::from(app.preview_scroll).min(maximum_scroll) as u16;
    frame.render_widget(preview.scroll((scroll, 0)), preview_area);

    if maximum_scroll > 0 {
        let mut scrollbar_state = ScrollbarState::new(content_height)
            .position(usize::from(scroll))
            .viewport_content_length(usize::from(preview_area.height));
        let scrollbar = Scrollbar::new(ScrollbarOrientation::VerticalRight)
            .begin_symbol(None)
            .end_symbol(None)
            .track_symbol(None);
        frame.render_stateful_widget(scrollbar, area, &mut scrollbar_state);
    }
}

fn preview_widget(app: &App) -> Paragraph<'static> {
    let value = app.tree.value_at(app.selected);
    Paragraph::new(highlight_json(value)).wrap(Wrap { trim: false })
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

pub(crate) fn preview_max_scroll(app: &App, area: Rect) -> u16 {
    let (_, area) = preview_areas(area);
    let content_height = preview_widget(app).line_count(area.width);
    content_height
        .saturating_sub(usize::from(area.height))
        .min(usize::from(u16::MAX)) as u16
}

fn highlight_json(value: &Value) -> Text<'static> {
    let mut lines = vec![Vec::new()];
    push_json(value, 0, &mut lines);
    Text::from(lines.into_iter().map(Line::from).collect::<Vec<_>>())
}

fn push_json(value: &Value, depth: usize, lines: &mut Vec<Vec<Span<'static>>>) {
    match value {
        Value::Object(map) if map.is_empty() => push_span(lines, "{}", punctuation_style()),
        Value::Object(map) => {
            push_span(lines, "{", punctuation_style());
            let last = map.len() - 1;
            for (index, (key, value)) in map.iter().enumerate() {
                lines.push(vec![Span::raw("  ".repeat(depth + 1))]);
                push_span(
                    lines,
                    serde_json::to_string(key).expect("JSON object keys are serializable"),
                    Style::default().fg(ACCENT),
                );
                push_span(lines, ": ", punctuation_style());
                push_json(value, depth + 1, lines);
                if index != last {
                    push_span(lines, ",", punctuation_style());
                }
            }
            lines.push(vec![Span::raw("  ".repeat(depth))]);
            push_span(lines, "}", punctuation_style());
        }
        Value::Array(items) if items.is_empty() => push_span(lines, "[]", punctuation_style()),
        Value::Array(items) => {
            push_span(lines, "[", punctuation_style());
            let last = items.len() - 1;
            for (index, value) in items.iter().enumerate() {
                lines.push(vec![Span::raw("  ".repeat(depth + 1))]);
                push_json(value, depth + 1, lines);
                if index != last {
                    push_span(lines, ",", punctuation_style());
                }
            }
            lines.push(vec![Span::raw("  ".repeat(depth))]);
            push_span(lines, "]", punctuation_style());
        }
        Value::String(value) => push_span(
            lines,
            serde_json::to_string(value).expect("JSON strings are serializable"),
            kind_style(NodeKind::String),
        ),
        Value::Number(value) => push_span(lines, value.to_string(), kind_style(NodeKind::Number)),
        Value::Bool(value) => push_span(lines, value.to_string(), kind_style(NodeKind::Bool)),
        Value::Null => push_span(lines, "null", kind_style(NodeKind::Null)),
    }
}

fn push_span(lines: &mut [Vec<Span<'static>>], content: impl Into<String>, style: Style) {
    lines
        .last_mut()
        .expect("JSON output always has a current line")
        .push(Span::styled(content.into(), style));
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
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled(prefix, style.add_modifier(Modifier::BOLD)),
            Span::styled(text, style),
        ])),
        lines[0],
    );
    frame.render_widget(
        Paragraph::new(
            "↑↓/jk move   ←→/hl structure   -/+ resize   / search   : path   p print   ? help",
        )
        .style(Style::default().fg(MUTED)),
        lines[1],
    );

    if app.input_mode != InputMode::Normal {
        let cursor_x = lines[0].x + 1 + app.input.chars().count() as u16;
        frame.set_cursor_position(Position::new(
            cursor_x.min(lines[0].right() - 1),
            lines[0].y,
        ));
    }
}

fn draw_help(frame: &mut Frame) {
    let area = centered_rect(68, 29, frame.area());
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
        let text = highlight_json(&value);
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
        let text =
            highlight_json(&json!({"text": "hello", "number": 7, "bool": false, "nil": null}));
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
}
