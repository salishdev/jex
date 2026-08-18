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

pub fn draw(frame: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(8),
            Constraint::Length(2),
        ])
        .split(frame.area());

    draw_header(frame, app, chunks[0]);
    draw_body(frame, app, chunks[1]);
    draw_footer(frame, app, chunks[2]);

    if app.show_help {
        draw_help(frame);
    }
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
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
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
        .block(Block::default().borders(Borders::RIGHT).title(format!(
            " Tree  {}/{} ",
            app.visible.len(),
            app.tree.len()
        )))
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
    let value = app.tree.value_at(app.selected);
    let preview = Paragraph::new(highlight_json(value))
        .block(
            Block::default()
                .title(format!(
                    " Value · {} ",
                    app.tree.node(app.selected).kind.name()
                ))
                .borders(Borders::NONE),
        )
        .wrap(Wrap { trim: false });
    frame.render_widget(
        preview,
        area.inner(Margin {
            vertical: 0,
            horizontal: 1,
        }),
    );
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
        Paragraph::new("↑↓/jk move   ←→/hl structure   / search   : path   p print   ? help")
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
    let area = centered_rect(68, 24, frame.area());
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
}
