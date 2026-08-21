use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend, layout::Rect};
use serde_json::Value;

use crate::{
    tree::{JsonTree, NodeId},
    ui,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    Jump,
}

const DEFAULT_TREE_PANE_PERCENT: u16 = 58;
const MIN_PANE_WIDTH: u16 = 20;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);

pub struct App {
    pub tree: JsonTree,
    pub source: String,
    pub selected: NodeId,
    pub visible: Vec<NodeId>,
    pub input_mode: InputMode,
    pub input: String,
    pub search_query: Option<String>,
    pub matches: Vec<NodeId>,
    pub match_index: Option<usize>,
    pub bookmark: Option<NodeId>,
    pub message: Option<String>,
    pub show_help: bool,
    pub output: Option<String>,
    pub preview_scroll: u16,
    pane_split_percent: u16,
    dragging_divider: bool,
    last_tree_click: Option<(NodeId, Instant)>,
    should_quit: bool,
    history: Vec<NodeId>,
    history_index: usize,
}

impl App {
    pub fn new(value: Value, source: String, expand_depth: usize) -> Self {
        let tree = JsonTree::new(value, expand_depth);
        let visible = tree.visible();
        Self {
            tree,
            source,
            selected: 0,
            visible,
            input_mode: InputMode::Normal,
            input: String::new(),
            search_query: None,
            matches: Vec::new(),
            match_index: None,
            bookmark: None,
            message: None,
            show_help: false,
            output: None,
            preview_scroll: 0,
            pane_split_percent: DEFAULT_TREE_PANE_PERCENT,
            dragging_divider: false,
            last_tree_click: None,
            should_quit: false,
            history: vec![0],
            history_index: 0,
        }
    }

    pub fn selected_visible_index(&self) -> usize {
        self.visible
            .iter()
            .position(|&id| id == self.selected)
            .unwrap_or(0)
    }

    fn refresh_visible(&mut self) {
        self.visible = self.tree.visible();
        if !self.visible.contains(&self.selected) {
            self.selected = self.visible.first().copied().unwrap_or(0);
        }
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        if key.kind != KeyEventKind::Press {
            return;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return;
        }

        let selected_before = self.selected;
        match self.input_mode {
            InputMode::Normal => self.handle_normal_key(key),
            InputMode::Search | InputMode::Jump => self.handle_input_key(key),
        }
        if self.selected != selected_before {
            self.preview_scroll = 0;
        }
    }

    fn handle_normal_key(&mut self, key: KeyEvent) {
        if self.show_help {
            self.show_help = false;
            return;
        }

        self.message = None;
        match (key.code, key.modifiers) {
            (KeyCode::Char('q'), _) => self.should_quit = true,
            (KeyCode::Char('p'), _) => self.print_value_and_quit(),
            (KeyCode::Char('P'), _) => {
                self.output = Some(self.tree.path(self.selected));
                self.should_quit = true;
            }
            (KeyCode::Char('?'), _) => self.show_help = true,
            (KeyCode::Up | KeyCode::Char('k'), _) => self.move_by(-1),
            (KeyCode::Down | KeyCode::Char('j'), _) => self.move_by(1),
            (KeyCode::PageUp, _) | (KeyCode::Char('u'), KeyModifiers::CONTROL) => {
                self.move_page(-1)
            }
            (KeyCode::PageDown, _) | (KeyCode::Char('d'), KeyModifiers::CONTROL) => {
                self.move_page(1)
            }
            (KeyCode::Home | KeyCode::Char('g'), _) => self.move_to_visible_edge(false),
            (KeyCode::End | KeyCode::Char('G'), _) => self.move_to_visible_edge(true),
            (KeyCode::Left | KeyCode::Char('h'), _) => self.move_left(),
            (KeyCode::Right | KeyCode::Char('l'), _) => self.move_right(),
            (KeyCode::Enter | KeyCode::Char(' '), _) => self.toggle_current(),
            (KeyCode::Char('['), _) => self.move_sibling(-1),
            (KeyCode::Char(']'), _) => self.move_sibling(1),
            (KeyCode::Char('e'), _) => {
                self.tree.expand_descendants(self.selected);
                self.refresh_visible();
            }
            (KeyCode::Char('c'), _) => {
                self.tree.node_mut(self.selected).expanded = false;
                self.tree.collapse_descendants(self.selected);
                self.refresh_visible();
            }
            (KeyCode::Char('-'), _) => self.resize_panes(-5),
            (KeyCode::Char('+') | KeyCode::Char('='), _) => self.resize_panes(5),
            (KeyCode::Char('/'), _) => self.begin_input(InputMode::Search),
            (KeyCode::Char(':'), _) => self.begin_input(InputMode::Jump),
            (KeyCode::Char('n'), _) => self.next_match(1),
            (KeyCode::Char('N'), _) => self.next_match(-1),
            (KeyCode::Char('b'), _) => self.history_back(),
            (KeyCode::Char('f'), _) => self.history_forward(),
            (KeyCode::Char('m'), _) => {
                self.bookmark = Some(self.selected);
                self.message = Some(format!("Marked {}", self.tree.path(self.selected)));
            }
            (KeyCode::Char('\''), _) => self.return_to_bookmark(),
            (KeyCode::Esc, _) => {
                self.search_query = None;
                self.matches.clear();
                self.match_index = None;
            }
            _ => {}
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.input_mode = InputMode::Normal;
                self.input.clear();
            }
            KeyCode::Enter => {
                let mode = self.input_mode;
                self.input_mode = InputMode::Normal;
                match mode {
                    InputMode::Search => self.submit_search(),
                    InputMode::Jump => self.submit_jump(),
                    InputMode::Normal => {}
                }
                self.input.clear();
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input.push(ch);
            }
            _ => {}
        }
    }

    fn begin_input(&mut self, mode: InputMode) {
        self.input_mode = mode;
        self.input = if mode == InputMode::Search {
            self.search_query.clone().unwrap_or_default()
        } else {
            String::new()
        };
    }

    fn move_by(&mut self, amount: isize) {
        let current = self.selected_visible_index() as isize;
        let last = self.visible.len().saturating_sub(1) as isize;
        let next = (current + amount).clamp(0, last) as usize;
        self.selected = self.visible[next];
    }

    fn move_page(&mut self, direction: isize) {
        // A stable step works predictably across small and large terminals; the list renderer
        // keeps the new selection on screen.
        self.move_by(direction * 10);
    }

    fn move_to_visible_edge(&mut self, end: bool) {
        if let Some(&id) = if end {
            self.visible.last()
        } else {
            self.visible.first()
        } {
            self.selected = id;
        }
    }

    fn move_left(&mut self) {
        let node = self.tree.node(self.selected);
        if node.expanded && !node.children.is_empty() {
            self.tree.node_mut(self.selected).expanded = false;
            self.refresh_visible();
        } else if let Some(parent) = node.parent {
            self.selected = parent;
        }
    }

    fn move_right(&mut self) {
        let node = self.tree.node(self.selected);
        if node.children.is_empty() {
            return;
        }
        if !node.expanded {
            self.tree.node_mut(self.selected).expanded = true;
            self.refresh_visible();
        } else {
            self.selected = node.children[0];
        }
    }

    fn toggle_current(&mut self) {
        if self.tree.node(self.selected).children.is_empty() {
            return;
        }
        let expanded = self.tree.node(self.selected).expanded;
        self.tree.node_mut(self.selected).expanded = !expanded;
        self.refresh_visible();
    }

    fn move_sibling(&mut self, direction: isize) {
        let Some(parent) = self.tree.node(self.selected).parent else {
            return;
        };
        let siblings = &self.tree.node(parent).children;
        let Some(index) = siblings.iter().position(|&id| id == self.selected) else {
            return;
        };
        let last = siblings.len().saturating_sub(1) as isize;
        let next = (index as isize + direction).clamp(0, last) as usize;
        self.selected = siblings[next];
    }

    fn submit_search(&mut self) {
        let query = self.input.trim().to_lowercase();
        if query.is_empty() {
            self.search_query = None;
            self.matches.clear();
            self.match_index = None;
            return;
        }
        self.search_query = Some(query.clone());
        self.matches = (0..self.tree.len())
            .filter(|&id| self.tree.searchable_text(id).contains(&query))
            .collect();
        if self.matches.is_empty() {
            self.match_index = None;
            self.message = Some(format!("No matches for {query:?}"));
            return;
        }
        let index = self
            .matches
            .iter()
            .position(|&id| id > self.selected)
            .unwrap_or(0);
        self.visit_match(index);
    }

    fn next_match(&mut self, direction: isize) {
        if self.matches.is_empty() {
            self.message = Some("Start a search with /".into());
            return;
        }
        let current = self.match_index.unwrap_or(0) as isize;
        let len = self.matches.len() as isize;
        let next = (current + direction).rem_euclid(len) as usize;
        self.visit_match(next);
    }

    fn visit_match(&mut self, index: usize) {
        let target = self.matches[index];
        self.match_index = Some(index);
        self.jump_to(target);
    }

    fn submit_jump(&mut self) {
        let pointer = self.input.trim();
        match self.tree.find_pointer(pointer) {
            Some(id) => self.jump_to(id),
            None => self.message = Some(format!("Path not found: {pointer}")),
        }
    }

    fn jump_to(&mut self, target: NodeId) {
        if target == self.selected {
            self.tree.reveal(target);
            self.refresh_visible();
            return;
        }
        self.remember_current_location();
        self.history.truncate(self.history_index + 1);
        self.history.push(target);
        self.history_index = self.history.len() - 1;
        self.tree.reveal(target);
        self.refresh_visible();
        self.selected = target;
    }

    fn remember_current_location(&mut self) {
        if self.history.get(self.history_index).copied() != Some(self.selected) {
            self.history.truncate(self.history_index + 1);
            self.history.push(self.selected);
            self.history_index = self.history.len() - 1;
        }
    }

    fn history_back(&mut self) {
        if self.history_index == 0 {
            self.message = Some("No earlier location".into());
            return;
        }
        self.history_index -= 1;
        self.restore_history_location();
    }

    fn history_forward(&mut self) {
        if self.history_index + 1 >= self.history.len() {
            self.message = Some("No later location".into());
            return;
        }
        self.history_index += 1;
        self.restore_history_location();
    }

    fn restore_history_location(&mut self) {
        let target = self.history[self.history_index];
        self.tree.reveal(target);
        self.refresh_visible();
        self.selected = target;
    }

    fn return_to_bookmark(&mut self) {
        match self.bookmark {
            Some(id) => self.jump_to(id),
            None => self.message = Some("No mark set; press m to set one".into()),
        }
    }

    fn print_value_and_quit(&mut self) {
        let value = self.tree.value_at(self.selected);
        self.output =
            Some(serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string()));
        self.should_quit = true;
    }

    pub fn tree_pane_width(&self, total_width: u16) -> u16 {
        if total_width <= 1 {
            return total_width;
        }

        let maximum = total_width.saturating_sub(MIN_PANE_WIDTH).max(1);
        let minimum = MIN_PANE_WIDTH.min(maximum);
        let preferred = (u32::from(total_width) * u32::from(self.pane_split_percent) / 100) as u16;
        preferred.clamp(minimum, maximum)
    }

    pub fn is_dragging_divider(&self) -> bool {
        self.dragging_divider
    }

    fn resize_panes(&mut self, percentage_points: i16) {
        self.pane_split_percent =
            (self.pane_split_percent as i16 + percentage_points).clamp(1, 99) as u16;
    }

    fn resize_to_column(&mut self, column: u16, body: Rect) {
        if body.width <= 1 {
            return;
        }

        let desired_width = column.saturating_sub(body.x).saturating_add(1);
        let maximum = body.width.saturating_sub(MIN_PANE_WIDTH).max(1);
        let minimum = MIN_PANE_WIDTH.min(maximum);
        let width = desired_width.clamp(minimum, maximum);
        self.pane_split_percent = ((u32::from(width) * 100 + u32::from(body.width) / 2)
            / u32::from(body.width))
        .clamp(1, 99) as u16;
    }

    fn tree_view_offset(&self, viewport_height: u16) -> usize {
        self.selected_visible_index()
            .saturating_sub(usize::from(viewport_height).saturating_sub(1))
    }

    fn select_tree_row(&mut self, mouse: MouseEvent, tree_area: Rect) {
        let items_area = ui::tree_items_area(tree_area);
        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if !items_area.contains(position) {
            self.last_tree_click = None;
            return;
        }

        let row = usize::from(mouse.row.saturating_sub(items_area.y));
        let index = self.tree_view_offset(items_area.height).saturating_add(row);
        let Some(&id) = self.visible.get(index) else {
            self.last_tree_click = None;
            return;
        };

        let indent_width = self.tree.node(id).depth.min(usize::from(u16::MAX)) as u16;
        let disclosure_column = items_area
            .x
            .saturating_add(1) // the list's selection marker
            .saturating_add(indent_width.saturating_mul(2));
        let clicked_disclosure = mouse.column == disclosure_column;
        let now = Instant::now();
        let double_clicked_row = !clicked_disclosure
            && self
                .last_tree_click
                .is_some_and(|(previous_id, previous_time)| {
                    previous_id == id
                        && now.saturating_duration_since(previous_time) <= DOUBLE_CLICK_INTERVAL
                });
        self.last_tree_click = if clicked_disclosure || double_clicked_row {
            None
        } else {
            Some((id, now))
        };
        let selected_before = self.selected;
        self.selected = id;
        if self.selected != selected_before {
            self.preview_scroll = 0;
        }
        if clicked_disclosure || double_clicked_row {
            self.toggle_current();
        }
    }

    fn scroll_preview(&mut self, direction: isize, area: Rect) {
        let maximum = ui::preview_max_scroll(self, area);
        let current = self.preview_scroll.min(maximum);
        self.preview_scroll = if direction < 0 {
            current.saturating_sub(direction.unsigned_abs() as u16)
        } else {
            current.saturating_add(direction as u16).min(maximum)
        };
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, body: Rect) {
        if self.show_help {
            return;
        }

        let over_body = mouse.row >= body.y
            && mouse.row < body.bottom()
            && mouse.column >= body.x
            && mouse.column < body.right();
        let tree_width = self.tree_pane_width(body.width);
        let tree_area = Rect::new(body.x, body.y, tree_width, body.height);
        let right_x = body.x.saturating_add(tree_width);
        let right_area = Rect::new(
            right_x,
            body.y,
            body.right().saturating_sub(right_x),
            body.height,
        );
        let selected_before = self.selected;
        match mouse.kind {
            MouseEventKind::ScrollUp if over_body && mouse.column >= right_x => {
                self.scroll_preview(-3, right_area)
            }
            MouseEventKind::ScrollDown if over_body && mouse.column >= right_x => {
                self.scroll_preview(3, right_area)
            }
            MouseEventKind::ScrollUp if over_body => self.move_by(-3),
            MouseEventKind::ScrollDown if over_body => self.move_by(3),
            MouseEventKind::Down(MouseButton::Left) if over_body => {
                let divider = body.x.saturating_add(tree_width).saturating_sub(1);
                if mouse.column.abs_diff(divider) <= 1 {
                    self.last_tree_click = None;
                    self.dragging_divider = true;
                    self.resize_to_column(mouse.column, body);
                } else if mouse.column < divider {
                    self.select_tree_row(mouse, tree_area);
                } else {
                    self.last_tree_click = None;
                }
            }
            MouseEventKind::Drag(MouseButton::Left) if self.dragging_divider => {
                self.resize_to_column(mouse.column, body);
            }
            MouseEventKind::Up(MouseButton::Left) if self.dragging_divider => {
                self.resize_to_column(mouse.column, body);
                self.dragging_divider = false;
            }
            MouseEventKind::Down(MouseButton::Left) => self.last_tree_click = None,
            _ => {}
        }
        if self.selected != selected_before {
            self.preview_scroll = 0;
        }
    }
}

pub fn run(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let guard = TerminalGuard;
    let mut output = terminal_output()?;
    execute!(output, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;

    while !app.should_quit {
        terminal.draw(|frame| ui::draw(frame, app))?;
        if event::poll(Duration::from_millis(250))? {
            match event::read()? {
                Event::Key(key) => app.handle_key(key),
                Event::Mouse(mouse) => {
                    let size = terminal.size()?;
                    let area = Rect::new(0, 0, size.width, size.height);
                    app.handle_mouse(mouse, ui::body_area(area));
                }
                Event::Resize(_, _) | Event::FocusGained | Event::FocusLost | Event::Paste(_) => {}
            }
        }
    }

    drop(terminal);
    drop(guard);
    Ok(())
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        if let Ok(mut output) = terminal_output() {
            let _ = execute!(output, LeaveAlternateScreen, DisableMouseCapture);
        }
    }
}

#[cfg(unix)]
fn terminal_output() -> io::Result<Box<dyn Write>> {
    use std::fs::OpenOptions;

    let tty = OpenOptions::new().read(true).write(true).open("/dev/tty")?;
    Ok(Box::new(tty))
}

#[cfg(not(unix))]
fn terminal_output() -> io::Result<Box<dyn Write>> {
    Ok(Box::new(io::stdout()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyEvent, MouseEvent};
    use serde_json::json;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn mouse(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column,
            row,
            modifiers: KeyModifiers::NONE,
        }
    }

    #[test]
    fn structural_navigation_works_when_children_are_hidden() {
        let mut app = App::new(json!({"a": {"b": 1}, "c": 2}), "test".into(), 1);
        app.handle_key(key(KeyCode::Down));
        assert_eq!(app.tree.path(app.selected), "/a");
        app.handle_key(key(KeyCode::Right));
        app.handle_key(key(KeyCode::Right));
        assert_eq!(app.tree.path(app.selected), "/a/b");
        app.handle_key(key(KeyCode::Left));
        assert_eq!(app.tree.path(app.selected), "/a");
    }

    #[test]
    fn search_reveals_hidden_match_and_cycles() {
        let mut app = App::new(json!({"a": {"needle": 1}, "needle2": 2}), "test".into(), 0);
        app.input = "needle".into();
        app.input_mode = InputMode::Search;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.path(app.selected), "/a/needle");
        assert!(app.visible.contains(&app.selected));
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.tree.path(app.selected), "/needle2");
    }

    #[test]
    fn pointer_jump_and_history_restore_locations() {
        let mut app = App::new(json!({"a": 1, "b": 2}), "test".into(), 1);
        app.selected = app.tree.find_pointer("/a").unwrap();
        app.input = "/b".into();
        app.input_mode = InputMode::Jump;
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.path(app.selected), "/b");
        app.handle_key(key(KeyCode::Char('b')));
        assert_eq!(app.tree.path(app.selected), "/a");
        app.handle_key(key(KeyCode::Char('f')));
        assert_eq!(app.tree.path(app.selected), "/b");
    }

    #[test]
    fn print_emits_only_the_selected_subtree() {
        let mut app = App::new(json!({"a": {"b": 1}, "c": 2}), "test".into(), 1);
        app.selected = app.tree.find_pointer("/a").unwrap();
        app.handle_key(key(KeyCode::Char('p')));
        assert_eq!(app.output.as_deref(), Some("{\n  \"b\": 1\n}"));
        assert!(app.should_quit);
    }

    #[test]
    fn pane_width_uses_default_split_and_preserves_minimums() {
        let app = App::new(json!({}), "test".into(), 1);

        assert_eq!(app.tree_pane_width(100), 58);
        assert_eq!(app.tree_pane_width(40), 20);
        assert_eq!(app.tree_pane_width(30), 10);
    }

    #[test]
    fn divider_drag_resizes_and_clamps_the_panes() {
        let mut app = App::new(json!({}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 20);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 57, 10), body);
        assert!(app.is_dragging_divider());

        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 74, 10), body);
        assert_eq!(app.tree_pane_width(body.width), 75);

        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 99, 10), body);
        assert!(!app.is_dragging_divider());
        assert_eq!(app.tree_pane_width(body.width), 80);
    }

    #[test]
    fn divider_only_starts_dragging_from_the_body_separator() {
        let mut app = App::new(json!({}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 20);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 20, 10), body);
        assert!(!app.is_dragging_divider());

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 57, 1), body);
        assert!(!app.is_dragging_divider());
    }

    #[test]
    fn clicking_a_tree_row_selects_it() {
        let mut app = App::new(json!({"a": 1, "b": 2}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 10);

        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 10, body.y + 3),
            body,
        );

        assert_eq!(app.tree.path(app.selected), "/b");
    }

    #[test]
    fn clicking_the_tree_title_does_not_select_a_row() {
        let mut app = App::new(json!({"a": 1, "b": 2}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 10);
        app.selected = app.tree.find_pointer("/b").unwrap();

        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 10, body.y),
            body,
        );

        assert_eq!(app.tree.path(app.selected), "/b");
    }

    #[test]
    fn clicking_a_tree_row_accounts_for_the_visible_list_offset() {
        let value = Value::Array((0..20).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 5);
        app.selected = app.visible[15];

        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 10, body.y + 1),
            body,
        );

        assert_eq!(app.selected_visible_index(), 12);
    }

    #[test]
    fn clicking_a_disclosure_marker_toggles_the_container() {
        let mut app = App::new(json!({"a": 1}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 10);

        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 1, body.y + 1),
            body,
        );

        assert!(!app.tree.node(0).expanded);
        assert_eq!(app.visible, vec![0]);
    }

    #[test]
    fn double_clicking_a_container_row_toggles_it() {
        let mut app = App::new(json!({"a": {"b": 1}}), "test".into(), 2);
        let body = Rect::new(0, 3, 100, 10);
        let click = mouse(MouseEventKind::Down(MouseButton::Left), 6, body.y + 2);

        app.handle_mouse(click, body);
        let container = app.tree.find_pointer("/a").unwrap();
        assert_eq!(app.selected, container);
        assert!(app.tree.node(container).expanded);

        app.handle_mouse(click, body);
        assert!(!app.tree.node(container).expanded);
        assert_eq!(app.visible, vec![0, container]);

        app.handle_mouse(click, body);
        app.handle_mouse(click, body);
        assert!(app.tree.node(container).expanded);
        assert_eq!(app.visible.len(), 3);
    }

    #[test]
    fn mouse_wheel_targets_the_pane_under_the_pointer() {
        let value = Value::Array((0..20).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 5);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 70, 4), body);
        assert_eq!(app.selected, 0);
        assert_eq!(app.preview_scroll, 3);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 10, 4), body);
        assert_eq!(app.selected_visible_index(), 3);
        assert_eq!(app.preview_scroll, 0);
    }

    #[test]
    fn divider_drag_ends_even_when_the_pointer_leaves_the_body() {
        let mut app = App::new(json!({}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 20);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 57, 10), body);
        app.handle_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 75, 1), body);

        assert!(!app.is_dragging_divider());
        assert_eq!(app.tree_pane_width(body.width), 76);
    }

    #[test]
    fn keyboard_shortcuts_resize_the_panes() {
        let mut app = App::new(json!({}), "test".into(), 1);

        app.handle_key(key(KeyCode::Char('-')));
        assert_eq!(app.tree_pane_width(100), 53);

        app.handle_key(key(KeyCode::Char('+')));
        assert_eq!(app.tree_pane_width(100), 58);
    }
}
