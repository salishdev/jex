use std::{
    collections::{HashMap, HashSet},
    io::{self, Write},
    sync::{
        Arc,
        mpsc::{self, Receiver, Sender},
    },
    thread,
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
    filter,
    tree::{JsonTree, NodeId},
    ui,
};

#[cfg(test)]
use crate::ui_state::DEFAULT_TREE_PANE_PERCENT;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InputMode {
    Normal,
    Search,
    Jump,
    Filter,
}

const MIN_PANE_WIDTH: u16 = 20;
const DOUBLE_CLICK_INTERVAL: Duration = Duration::from_millis(500);
const FILTER_PREVIEW_DEBOUNCE: Duration = Duration::from_millis(75);
const FILTER_WORKER_POLL_INTERVAL: Duration = Duration::from_millis(16);
const FILTER_SPINNER_INTERVAL: Duration = Duration::from_millis(80);
const FILTER_SPINNER_FRAMES: usize = 10;

struct FilterRequest {
    generation: u64,
    expression: String,
}

struct FilterResponse {
    generation: u64,
    expression: String,
    result: Result<filter::FilterOutput, filter::FilterError>,
}

struct FilterWorker {
    requests: Sender<FilterRequest>,
    responses: Receiver<FilterResponse>,
}

impl FilterWorker {
    fn new(source: Arc<Value>) -> Self {
        let (request_sender, request_receiver) = mpsc::channel::<FilterRequest>();
        let (response_sender, response_receiver) = mpsc::channel();
        thread::spawn(move || {
            let input = filter::prepare(&source);
            while let Ok(mut request) = request_receiver.recv() {
                // If typing outran evaluation, skip directly to the newest expression.
                while let Ok(newer) = request_receiver.try_recv() {
                    request = newer;
                }
                let response = FilterResponse {
                    generation: request.generation,
                    expression: request.expression.clone(),
                    result: input
                        .as_ref()
                        .map_err(Clone::clone)
                        .and_then(|input| filter::evaluate_prepared(input, &request.expression)),
                };
                if response_sender.send(response).is_err() {
                    break;
                }
            }
        });
        Self {
            requests: request_sender,
            responses: response_receiver,
        }
    }
}

struct FilterPreview {
    expression: String,
    output: filter::FilterOutput,
    lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ScrollbarDrag {
    Tree { grab_offset: u16 },
    Preview { grab_offset: u16 },
    FilterPreview { grab_offset: u16 },
}

impl FilterPreview {
    fn new(expression: String, output: filter::FilterOutput) -> Self {
        let formatted = serde_json::to_string_pretty(&output.value)
            .unwrap_or_else(|_| output.value.to_string());
        let lines = formatted.lines().map(str::to_owned).collect();
        Self {
            expression,
            output,
            lines,
        }
    }
}

pub struct App {
    pub tree: JsonTree,
    pub source: String,
    source_value: Arc<Value>,
    pub selected: NodeId,
    pub visible: Vec<NodeId>,
    pub input_mode: InputMode,
    pub input: String,
    pub input_cursor: usize,
    pub active_filter: Option<String>,
    pub filter_output_count: Option<usize>,
    pub search_query: Option<String>,
    pub matches: Vec<NodeId>,
    match_set: HashSet<NodeId>,
    pub match_index: Option<usize>,
    pub bookmark: Option<NodeId>,
    pub message: Option<String>,
    pub show_help: bool,
    pub output: Option<String>,
    pub preview_scroll: usize,
    tree_scroll: usize,
    tree_scroll_locked: bool,
    filter_worker: FilterWorker,
    filter_generation: u64,
    filter_preview_deadline: Option<Instant>,
    latest_dispatched_filter: Option<u64>,
    filter_preview: Option<FilterPreview>,
    filter_preview_scroll: usize,
    filter_spinner_frame: usize,
    filter_spinner_deadline: Option<Instant>,
    pane_split_percent: u16,
    pane_split_changed: bool,
    dragging_divider: bool,
    scrollbar_drag: Option<ScrollbarDrag>,
    last_tree_click: Option<(NodeId, Instant)>,
    should_quit: bool,
    history: Vec<NodeId>,
    history_index: usize,
    visible_positions: HashMap<NodeId, usize>,
    expand_depth: usize,
}

impl App {
    #[cfg(test)]
    pub fn new(value: Value, source: String, expand_depth: usize) -> Self {
        Self::with_pane_split_percent(value, source, expand_depth, DEFAULT_TREE_PANE_PERCENT)
    }

    pub fn with_pane_split_percent(
        value: Value,
        source: String,
        expand_depth: usize,
        pane_split_percent: u16,
    ) -> Self {
        let source_value = Arc::new(value);
        let tree = JsonTree::from_shared(Arc::clone(&source_value), expand_depth);
        let filter_worker = FilterWorker::new(Arc::clone(&source_value));
        let visible = tree.visible();
        let visible_positions = Self::index_visible_nodes(&visible);
        Self {
            tree,
            source,
            source_value,
            selected: 0,
            visible,
            input_mode: InputMode::Normal,
            input: String::new(),
            input_cursor: 0,
            active_filter: None,
            filter_output_count: None,
            search_query: None,
            matches: Vec::new(),
            match_set: HashSet::new(),
            match_index: None,
            bookmark: None,
            message: None,
            show_help: false,
            output: None,
            preview_scroll: 0,
            tree_scroll: 0,
            tree_scroll_locked: false,
            filter_worker,
            filter_generation: 0,
            filter_preview_deadline: None,
            latest_dispatched_filter: None,
            filter_preview: None,
            filter_preview_scroll: 0,
            filter_spinner_frame: 0,
            filter_spinner_deadline: None,
            pane_split_percent: pane_split_percent.clamp(1, 99),
            pane_split_changed: false,
            dragging_divider: false,
            scrollbar_drag: None,
            last_tree_click: None,
            should_quit: false,
            history: vec![0],
            history_index: 0,
            visible_positions,
            expand_depth,
        }
    }

    pub fn selected_visible_index(&self) -> usize {
        self.visible_positions
            .get(&self.selected)
            .copied()
            .unwrap_or(0)
    }

    pub fn is_match(&self, id: NodeId) -> bool {
        self.match_set.contains(&id)
    }

    fn index_visible_nodes(visible: &[NodeId]) -> HashMap<NodeId, usize> {
        let mut positions = HashMap::with_capacity(visible.len());
        for (index, &id) in visible.iter().enumerate() {
            positions.insert(id, index);
        }
        positions
    }

    fn refresh_visible(&mut self) {
        self.visible = self.tree.visible();
        self.visible_positions = Self::index_visible_nodes(&self.visible);
        if !self.visible_positions.contains_key(&self.selected) {
            self.selected = self.visible.first().copied().unwrap_or(0);
            self.tree_scroll_locked = false;
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
            InputMode::Search | InputMode::Jump | InputMode::Filter => self.handle_input_key(key),
        }
        if self.selected != selected_before {
            self.preview_scroll = 0;
            self.tree_scroll_locked = false;
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
            (KeyCode::Char('|'), _) => self.begin_input(InputMode::Filter),
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
                if self.search_query.is_some() {
                    self.clear_search();
                } else if self.active_filter.is_some() {
                    self.clear_filter();
                }
            }
            _ => {}
        }
    }

    fn handle_input_key(&mut self, key: KeyEvent) {
        let input_before = self.input.clone();
        match key.code {
            KeyCode::Esc => {
                if self.input_mode == InputMode::Filter {
                    self.cancel_filter_edit();
                }
                self.input_mode = InputMode::Normal;
                self.input.clear();
                self.input_cursor = 0;
                self.message = None;
            }
            KeyCode::Enter => {
                let mode = self.input_mode;
                let accepted = match mode {
                    InputMode::Search => {
                        self.submit_search();
                        true
                    }
                    InputMode::Jump => {
                        self.submit_jump();
                        true
                    }
                    InputMode::Filter => self.submit_filter(),
                    InputMode::Normal => true,
                };
                if accepted {
                    self.input_mode = InputMode::Normal;
                    self.input.clear();
                    self.input_cursor = 0;
                }
            }
            KeyCode::Backspace => {
                self.message = None;
                self.remove_before_cursor();
            }
            KeyCode::Delete => {
                self.message = None;
                self.remove_at_cursor();
            }
            KeyCode::Left => self.input_cursor = self.input_cursor.saturating_sub(1),
            KeyCode::Right => {
                self.input_cursor = (self.input_cursor + 1).min(self.input.chars().count())
            }
            KeyCode::Up if self.input_mode == InputMode::Filter => self.scroll_filter_preview(-1),
            KeyCode::Down if self.input_mode == InputMode::Filter => self.scroll_filter_preview(1),
            KeyCode::PageUp if self.input_mode == InputMode::Filter => {
                self.scroll_filter_preview(-5)
            }
            KeyCode::PageDown if self.input_mode == InputMode::Filter => {
                self.scroll_filter_preview(5)
            }
            KeyCode::Home => self.input_cursor = 0,
            KeyCode::End => self.input_cursor = self.input.chars().count(),
            KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input_cursor = 0
            }
            KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.input_cursor = self.input.chars().count()
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.message = None;
                self.input.clear();
                self.input_cursor = 0;
            }
            KeyCode::Char('w') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.message = None;
                self.remove_previous_word()
            }
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.message = None;
                let byte = char_to_byte(&self.input, self.input_cursor);
                self.input.insert(byte, ch);
                self.input_cursor += 1;
            }
            _ => {}
        }
        if self.input_mode == InputMode::Filter && self.input != input_before {
            self.schedule_filter_preview();
        }
    }

    fn begin_input(&mut self, mode: InputMode) {
        self.filter_preview = None;
        self.filter_preview_scroll = 0;
        self.input_mode = mode;
        self.input = match mode {
            InputMode::Search => self.search_query.clone().unwrap_or_default(),
            InputMode::Filter => self.active_filter.clone().unwrap_or_default(),
            InputMode::Jump | InputMode::Normal => String::new(),
        };
        self.input_cursor = self.input.chars().count();
        self.message = None;
        if mode == InputMode::Filter && !self.input.trim().is_empty() {
            self.schedule_filter_preview();
        }
    }

    fn remove_before_cursor(&mut self) {
        if self.input_cursor == 0 {
            return;
        }
        let start = char_to_byte(&self.input, self.input_cursor - 1);
        let end = char_to_byte(&self.input, self.input_cursor);
        self.input.replace_range(start..end, "");
        self.input_cursor -= 1;
    }

    fn remove_at_cursor(&mut self) {
        if self.input_cursor == self.input.chars().count() {
            return;
        }
        let start = char_to_byte(&self.input, self.input_cursor);
        let end = char_to_byte(&self.input, self.input_cursor + 1);
        self.input.replace_range(start..end, "");
    }

    fn remove_previous_word(&mut self) {
        while self.input_cursor > 0 {
            let previous = self.input.chars().nth(self.input_cursor - 1);
            if previous.is_some_and(|ch| !ch.is_whitespace()) {
                break;
            }
            self.remove_before_cursor();
        }
        while self.input_cursor > 0 {
            let previous = self.input.chars().nth(self.input_cursor - 1);
            if previous.is_some_and(char::is_whitespace) {
                break;
            }
            self.remove_before_cursor();
        }
    }

    fn move_by(&mut self, amount: isize) {
        self.tree_scroll_locked = false;
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
        self.tree_scroll_locked = false;
        if let Some(&id) = if end {
            self.visible.last()
        } else {
            self.visible.first()
        } {
            self.selected = id;
        }
    }

    fn move_left(&mut self) {
        self.tree_scroll_locked = false;
        let node = self.tree.node(self.selected);
        if node.expanded && !node.children.is_empty() {
            self.tree.node_mut(self.selected).expanded = false;
            self.refresh_visible();
        } else if let Some(parent) = node.parent {
            self.selected = parent;
        }
    }

    fn move_right(&mut self) {
        self.tree_scroll_locked = false;
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
        self.tree_scroll_locked = false;
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
            self.match_set.clear();
            self.match_index = None;
            return;
        }
        self.search_query = Some(query.clone());
        self.matches = self.tree.search(&query);
        self.match_set = self.matches.iter().copied().collect();
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

    fn clear_search(&mut self) {
        self.search_query = None;
        self.matches.clear();
        self.match_set.clear();
        self.match_index = None;
    }

    fn submit_filter(&mut self) -> bool {
        self.cancel_pending_filter_preview();
        let expression = self.input.trim().to_owned();
        if expression.is_empty() {
            self.clear_filter();
            self.filter_preview = None;
            self.filter_preview_scroll = 0;
            return true;
        }

        if self.active_filter.as_deref() == Some(expression.as_str()) {
            let count = self.filter_output_count.unwrap_or(0);
            self.message = Some(filter_status("Filter applied", count));
            self.filter_preview = None;
            self.filter_preview_scroll = 0;
            return true;
        }

        let preview_matches = self
            .filter_preview
            .as_ref()
            .is_some_and(|preview| preview.expression == expression);
        let result = if preview_matches {
            Ok(self
                .filter_preview
                .take()
                .expect("matching filter preview is present")
                .output)
        } else {
            filter::evaluate(&self.source_value, &expression)
        };
        match result {
            Ok(output) => {
                let count = output.count;
                self.apply_filter_output(expression, output);
                self.message = Some(filter_status("Filter applied", count));
                self.filter_preview_scroll = 0;
                true
            }
            Err(error) => {
                self.message = Some(error.to_string());
                false
            }
        }
    }

    fn clear_filter(&mut self) {
        self.replace_tree(Arc::clone(&self.source_value));
        self.active_filter = None;
        self.filter_output_count = None;
        self.message = Some("Filter cleared".into());
    }

    fn apply_filter_output(&mut self, expression: String, output: filter::FilterOutput) {
        let count = output.count;
        self.replace_tree(Arc::new(output.value));
        self.active_filter = Some(expression);
        self.filter_output_count = Some(count);
    }

    fn schedule_filter_preview(&mut self) {
        self.filter_generation = self.filter_generation.wrapping_add(1);
        let now = Instant::now();
        self.filter_preview_deadline = Some(now + FILTER_PREVIEW_DEBOUNCE);
        self.filter_preview_scroll = 0;
        self.filter_spinner_frame = 0;
        self.filter_spinner_deadline = Some(now + FILTER_SPINNER_INTERVAL);
    }

    fn cancel_pending_filter_preview(&mut self) {
        self.filter_generation = self.filter_generation.wrapping_add(1);
        self.filter_preview_deadline = None;
        self.filter_spinner_deadline = None;
    }

    fn cancel_filter_edit(&mut self) {
        self.cancel_pending_filter_preview();
        self.filter_preview = None;
        self.filter_preview_scroll = 0;
    }

    fn process_filter_preview(&mut self, now: Instant) -> bool {
        let mut changed = false;
        if self.is_filter_preview_pending()
            && self
                .filter_spinner_deadline
                .is_some_and(|deadline| deadline <= now)
        {
            self.filter_spinner_frame = (self.filter_spinner_frame + 1) % FILTER_SPINNER_FRAMES;
            self.filter_spinner_deadline = Some(now + FILTER_SPINNER_INTERVAL);
            changed = true;
        }
        if self
            .filter_preview_deadline
            .is_some_and(|deadline| deadline <= now)
        {
            self.filter_preview_deadline = None;
            let expression = self.input.trim().to_owned();
            if expression.is_empty() {
                self.filter_preview = None;
                self.message = None;
                changed = true;
            } else {
                let request = FilterRequest {
                    generation: self.filter_generation,
                    expression,
                };
                if self.filter_worker.requests.send(request).is_ok() {
                    self.latest_dispatched_filter = Some(self.filter_generation);
                }
            }
        }

        while let Ok(response) = self.filter_worker.responses.try_recv() {
            if self.latest_dispatched_filter == Some(response.generation) {
                self.latest_dispatched_filter = None;
            }
            if self.input_mode != InputMode::Filter
                || response.generation != self.filter_generation
                || response.expression != self.input.trim()
            {
                continue;
            }
            match response.result {
                Ok(output) => {
                    self.filter_preview = Some(FilterPreview::new(response.expression, output));
                    self.message = None;
                }
                Err(error) => self.message = Some(error.to_string()),
            }
            changed = true;
        }
        if !self.is_filter_preview_pending() {
            self.filter_spinner_deadline = None;
        }
        changed
    }

    fn next_filter_update_in(&self, now: Instant) -> Option<Duration> {
        let mut wait = self
            .filter_preview_deadline
            .map(|deadline| deadline.saturating_duration_since(now));
        if self.latest_dispatched_filter.is_some() {
            wait = Some(
                wait.map(|duration| duration.min(FILTER_WORKER_POLL_INTERVAL))
                    .unwrap_or(FILTER_WORKER_POLL_INTERVAL),
            );
        }
        if let Some(deadline) = self.filter_spinner_deadline {
            let spinner_wait = deadline.saturating_duration_since(now);
            wait = Some(
                wait.map(|duration| duration.min(spinner_wait))
                    .unwrap_or(spinner_wait),
            );
        }
        wait
    }

    pub fn is_filter_preview_pending(&self) -> bool {
        self.filter_preview_deadline.is_some()
            || self.latest_dispatched_filter == Some(self.filter_generation)
    }

    pub fn filter_preview(&self) -> Option<(&Value, usize)> {
        self.filter_preview
            .as_ref()
            .map(|preview| (&preview.output.value, preview.output.count))
    }

    pub fn filter_preview_lines(&self) -> Option<&[String]> {
        self.filter_preview
            .as_ref()
            .map(|preview| preview.lines.as_slice())
    }

    pub fn filter_preview_scroll(&self) -> usize {
        self.filter_preview_scroll
    }

    pub fn filter_spinner_frame(&self) -> usize {
        self.filter_spinner_frame
    }

    fn scroll_filter_preview(&mut self, amount: isize) {
        if self.filter_preview.is_none() {
            return;
        }
        self.filter_preview_scroll = if amount < 0 {
            self.filter_preview_scroll
                .saturating_sub(amount.unsigned_abs())
        } else {
            self.filter_preview_scroll.saturating_add(amount as usize)
        };
    }

    fn replace_tree(&mut self, value: Arc<Value>) {
        self.tree = JsonTree::from_shared(value, self.expand_depth);
        self.selected = 0;
        self.visible = self.tree.visible();
        self.visible_positions = Self::index_visible_nodes(&self.visible);
        self.clear_search();
        self.bookmark = None;
        self.preview_scroll = 0;
        self.tree_scroll = 0;
        self.tree_scroll_locked = false;
        self.history = vec![0];
        self.history_index = 0;
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
        self.tree_scroll_locked = false;
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
        self.tree_scroll_locked = false;
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

    pub fn pane_split_percent(&self) -> u16 {
        self.pane_split_percent
    }

    pub fn pane_split_changed(&self) -> bool {
        self.pane_split_changed
    }

    fn set_pane_split_percent(&mut self, percent: u16) {
        let percent = percent.clamp(1, 99);
        if percent != self.pane_split_percent {
            self.pane_split_percent = percent;
            self.pane_split_changed = true;
        }
    }

    fn resize_panes(&mut self, percentage_points: i16) {
        let percent = (self.pane_split_percent as i16 + percentage_points).clamp(1, 99) as u16;
        self.set_pane_split_percent(percent);
    }

    fn resize_to_column(&mut self, column: u16, body: Rect) {
        if body.width <= 1 {
            return;
        }

        let desired_width = column.saturating_sub(body.x).saturating_add(1);
        let maximum = body.width.saturating_sub(MIN_PANE_WIDTH).max(1);
        let minimum = MIN_PANE_WIDTH.min(maximum);
        let width = desired_width.clamp(minimum, maximum);
        let percent = ((u32::from(width) * 100 + u32::from(body.width) / 2) / u32::from(body.width))
            .clamp(1, 99) as u16;
        self.set_pane_split_percent(percent);
    }

    pub(crate) fn tree_max_scroll(&self, viewport_height: u16) -> usize {
        self.visible
            .len()
            .saturating_sub(usize::from(viewport_height))
    }

    pub(crate) fn tree_view_offset(&self, viewport_height: u16) -> usize {
        let maximum = self.tree_max_scroll(viewport_height);
        if self.tree_scroll_locked {
            self.tree_scroll.min(maximum)
        } else {
            self.selected_visible_index()
                .saturating_sub(usize::from(viewport_height).saturating_sub(1))
                .min(maximum)
        }
    }

    fn select_tree_row(&mut self, mouse: MouseEvent, tree_area: Rect) {
        let items_area = ui::tree_items_area(tree_area);
        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if !items_area.contains(position) {
            self.last_tree_click = None;
            return;
        }

        let row = usize::from(mouse.row.saturating_sub(items_area.y));
        let offset = self.tree_view_offset(items_area.height);
        let index = offset.saturating_add(row);
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
        self.tree_scroll = offset;
        self.tree_scroll_locked = true;
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
            current.saturating_sub(direction.unsigned_abs())
        } else {
            current.saturating_add(direction as usize).min(maximum)
        };
    }

    fn scroll_tree(&mut self, direction: isize, area: Rect) {
        let viewport_height = ui::tree_items_area(area).height;
        let maximum = self.tree_max_scroll(viewport_height);
        let current = self.tree_view_offset(viewport_height);
        self.tree_scroll = if direction < 0 {
            current.saturating_sub(direction.unsigned_abs())
        } else {
            current.saturating_add(direction as usize).min(maximum)
        };
        self.tree_scroll_locked = true;
    }

    fn drag_scrollbar(&mut self, drag: ScrollbarDrag, mouse: MouseEvent, body: Rect) {
        let tree_width = self.tree_pane_width(body.width);
        let tree_area = Rect::new(body.x, body.y, tree_width, body.height);
        let right_x = body.x.saturating_add(tree_width);
        let right_area = Rect::new(
            right_x,
            body.y,
            body.right().saturating_sub(right_x),
            body.height,
        );

        match drag {
            ScrollbarDrag::Tree { grab_offset } => {
                let maximum = self.tree_max_scroll(ui::tree_items_area(tree_area).height);
                let Some(scrollbar) = ui::tree_scrollbar(self, tree_area) else {
                    return;
                };
                self.tree_scroll = scrollbar.position_for_drag(mouse.row, grab_offset, maximum);
                self.tree_scroll_locked = true;
            }
            ScrollbarDrag::Preview { grab_offset } => {
                let maximum = ui::preview_max_scroll(self, right_area);
                let Some(scrollbar) = ui::preview_scrollbar(self, right_area) else {
                    return;
                };
                self.preview_scroll = scrollbar.position_for_drag(mouse.row, grab_offset, maximum);
            }
            ScrollbarDrag::FilterPreview { grab_offset } => {
                let maximum = ui::filter_preview_max_scroll(self, body);
                let Some(scrollbar) = ui::filter_preview_scrollbar(self, body) else {
                    return;
                };
                self.filter_preview_scroll =
                    scrollbar.position_for_drag(mouse.row, grab_offset, maximum);
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, body: Rect) {
        if self.show_help {
            return;
        }

        if let Some(drag) = self.scrollbar_drag
            && matches!(
                mouse.kind,
                MouseEventKind::Drag(MouseButton::Left) | MouseEventKind::Up(MouseButton::Left)
            )
        {
            self.drag_scrollbar(drag, mouse, body);
            if matches!(mouse.kind, MouseEventKind::Up(MouseButton::Left)) {
                self.scrollbar_drag = None;
            }
            return;
        }

        let position = ratatui::layout::Position::new(mouse.column, mouse.row);
        if self.input_mode == InputMode::Filter
            && !self.dragging_divider
            && ui::filter_overlay_area(body).contains(position)
        {
            self.last_tree_click = None;
            match mouse.kind {
                MouseEventKind::ScrollUp => self.scroll_filter_preview(-3),
                MouseEventKind::ScrollDown => self.scroll_filter_preview(3),
                MouseEventKind::Down(MouseButton::Left) => {
                    if let Some(grab_offset) = ui::filter_preview_scrollbar(self, body)
                        .and_then(|scrollbar| scrollbar.grab_offset(position))
                    {
                        self.scrollbar_drag = Some(ScrollbarDrag::FilterPreview { grab_offset });
                    }
                }
                _ => {}
            }
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
            MouseEventKind::ScrollUp if over_body => self.scroll_tree(-3, tree_area),
            MouseEventKind::ScrollDown if over_body => self.scroll_tree(3, tree_area),
            MouseEventKind::Down(MouseButton::Left) if over_body => {
                let divider = body.x.saturating_add(tree_width).saturating_sub(1);
                if let Some(grab_offset) = ui::tree_scrollbar(self, tree_area)
                    .and_then(|scrollbar| scrollbar.grab_offset(position))
                {
                    self.last_tree_click = None;
                    self.scrollbar_drag = Some(ScrollbarDrag::Tree { grab_offset });
                } else if let Some(grab_offset) = ui::preview_scrollbar(self, right_area)
                    .and_then(|scrollbar| scrollbar.grab_offset(position))
                {
                    self.last_tree_click = None;
                    self.scrollbar_drag = Some(ScrollbarDrag::Preview { grab_offset });
                } else if mouse.column.abs_diff(divider) <= 1 {
                    self.last_tree_click = None;
                    self.dragging_divider = true;
                    self.resize_to_column(mouse.column, body);
                } else if mouse.column < divider {
                    self.select_tree_row(mouse, tree_area);
                } else {
                    self.last_tree_click = None;
                    if let Some(target) = ui::breadcrumb_target_at(self, right_area, position) {
                        self.jump_to(target);
                    }
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

fn char_to_byte(text: &str, character: usize) -> usize {
    text.char_indices()
        .nth(character)
        .map(|(byte, _)| byte)
        .unwrap_or(text.len())
}

fn filter_status(prefix: &str, count: usize) -> String {
    format!(
        "{prefix} · {count} {}",
        if count == 1 { "output" } else { "outputs" }
    )
}

pub fn run(app: &mut App) -> Result<()> {
    enable_raw_mode()?;
    let guard = TerminalGuard;
    let mut output = terminal_output()?;
    execute!(output, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(output);
    let mut terminal = Terminal::new(backend)?;

    terminal.draw(|frame| ui::draw(frame, app))?;
    while !app.should_quit {
        let now = Instant::now();
        if app.process_filter_preview(now) {
            terminal.draw(|frame| ui::draw(frame, app))?;
        }
        let wait = app
            .next_filter_update_in(now)
            .unwrap_or(Duration::from_secs(60));
        if !event::poll(wait)? {
            continue;
        }
        let should_redraw = match event::read()? {
            Event::Key(key) => {
                app.handle_key(key);
                true
            }
            Event::Mouse(mouse) => {
                let size = terminal.size()?;
                let area = Rect::new(0, 0, size.width, size.height);
                app.handle_mouse(mouse, ui::body_area(area));
                true
            }
            Event::Resize(_, _) => true,
            Event::FocusGained | Event::FocusLost | Event::Paste(_) => false,
        };
        if should_redraw && !app.should_quit {
            terminal.draw(|frame| ui::draw(frame, app))?;
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

    fn set_input(app: &mut App, input: &str, mode: InputMode) {
        app.input = input.into();
        app.input_cursor = input.chars().count();
        app.input_mode = mode;
    }

    fn type_text(app: &mut App, text: &str) {
        for ch in text.chars() {
            app.handle_key(key(KeyCode::Char(ch)));
        }
    }

    fn finish_filter_preview(app: &mut App) {
        app.process_filter_preview(Instant::now() + FILTER_PREVIEW_DEBOUNCE);
        let timeout = Instant::now() + Duration::from_secs(1);
        while app.is_filter_preview_pending() && Instant::now() < timeout {
            thread::yield_now();
            app.process_filter_preview(Instant::now());
        }
        assert!(!app.is_filter_preview_pending(), "filter preview timed out");
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
        set_input(&mut app, "needle", InputMode::Search);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.path(app.selected), "/a/needle");
        assert!(app.visible.contains(&app.selected));
        assert!(app.matches.iter().all(|&id| app.is_match(id)));
        app.handle_key(key(KeyCode::Char('n')));
        assert_eq!(app.tree.path(app.selected), "/needle2");
    }

    #[test]
    fn pointer_jump_and_history_restore_locations() {
        let mut app = App::new(json!({"a": 1, "b": 2}), "test".into(), 1);
        app.selected = app.tree.find_pointer("/a").unwrap();
        set_input(&mut app, "/b", InputMode::Jump);
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
    fn pane_width_can_be_restored_and_tracks_later_changes() {
        let mut app = App::with_pane_split_percent(json!({}), "test".into(), 1, 73);

        assert_eq!(app.tree_pane_width(100), 73);
        assert_eq!(app.pane_split_percent(), 73);
        assert!(!app.pane_split_changed());

        app.handle_key(key(KeyCode::Char('+')));
        assert_eq!(app.pane_split_percent(), 78);
        assert!(app.pane_split_changed());
    }

    #[test]
    fn restored_pane_width_is_defensively_clamped() {
        let app = App::with_pane_split_percent(json!({}), "test".into(), 1, u16::MAX);

        assert_eq!(app.pane_split_percent(), 99);
    }

    #[test]
    fn divider_drag_resizes_and_clamps_the_panes() {
        let mut app = App::new(json!({}), "test".into(), 1);
        let body = Rect::new(0, 3, 100, 20);

        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 57, 10), body);
        assert!(app.is_dragging_divider());

        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 74, 10), body);
        assert_eq!(app.tree_pane_width(body.width), 75);
        assert!(app.pane_split_changed());

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
    fn mouse_wheel_scrolls_the_hovered_pane_without_changing_selection() {
        let value = Value::Array((0..20).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 5);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 70, 4), body);
        assert_eq!(app.selected, 0);
        assert_eq!(app.preview_scroll, 3);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 10, 4), body);
        assert_eq!(app.selected, 0);
        assert_eq!(app.tree_view_offset(body.height), 3);
        assert_eq!(app.preview_scroll, 3);
    }

    #[test]
    fn keyboard_navigation_brings_the_selection_back_into_view() {
        let value = Value::Array((0..20).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 5);

        app.handle_mouse(mouse(MouseEventKind::ScrollDown, 10, 4), body);
        assert_eq!(app.tree_view_offset(body.height), 3);

        app.handle_key(key(KeyCode::Down));

        assert_eq!(app.selected_visible_index(), 1);
        assert_eq!(app.tree_view_offset(body.height), 0);
    }

    #[test]
    fn dragging_the_tree_scrollbar_handle_moves_only_the_viewport() {
        let value = Value::Array((0..100).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 10);

        // The tree thumb starts at the top of the divider column.
        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 57, body.y + 1),
            body,
        );
        assert!(matches!(
            app.scrollbar_drag,
            Some(ScrollbarDrag::Tree { .. })
        ));

        app.handle_mouse(
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                57,
                body.bottom() - 2,
            ),
            body,
        );
        assert_eq!(app.selected, 0);
        assert_eq!(
            app.tree_view_offset(body.height),
            app.tree_max_scroll(body.height)
        );

        app.handle_mouse(
            mouse(MouseEventKind::Up(MouseButton::Left), 57, body.bottom()),
            body,
        );
        assert_eq!(app.scrollbar_drag, None);
    }

    #[test]
    fn dragging_the_value_scrollbar_handle_scrolls_the_preview() {
        let value = Value::Array((0..100).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 10);
        let right_x = app.tree_pane_width(body.width);
        let right_area = Rect::new(right_x, body.y, body.width - right_x, body.height);

        app.handle_mouse(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                body.right() - 1,
                body.y,
            ),
            body,
        );
        assert!(matches!(
            app.scrollbar_drag,
            Some(ScrollbarDrag::Preview { .. })
        ));

        app.handle_mouse(
            mouse(
                MouseEventKind::Drag(MouseButton::Left),
                body.right() - 1,
                body.bottom() - 2,
            ),
            body,
        );
        assert_eq!(app.preview_scroll, ui::preview_max_scroll(&app, right_area));
    }

    #[test]
    fn clicking_a_breadcrumb_navigates_and_records_history() {
        let mut app = App::new(json!({"a": {"b": {"c": 1}}}), "test".into(), 0);
        let body = Rect::new(0, 3, 100, 10);
        app.selected = app.tree.find_pointer("/a/b/c").unwrap();

        // The right pane starts at x=58, has a one-column inset, and renders
        // "$ › a" on the breadcrumb row. Click the "a" segment.
        app.handle_mouse(
            mouse(MouseEventKind::Down(MouseButton::Left), 63, body.y),
            body,
        );

        assert_eq!(app.tree.path(app.selected), "/a");
        app.handle_key(key(KeyCode::Char('b')));
        assert_eq!(app.tree.path(app.selected), "/a/b/c");
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

    #[test]
    fn filter_overlay_absorbs_mouse_events_over_the_document() {
        let mut app = App::new(
            Value::Array((0..30).map(Value::from).collect()),
            "test".into(),
            1,
        );
        let body = Rect::new(0, 3, 100, 20);
        let overlay = ui::filter_overlay_area(body);
        app.handle_key(key(KeyCode::Char('|')));

        app.handle_mouse(
            mouse(
                MouseEventKind::Down(MouseButton::Left),
                overlay.x.saturating_add(1),
                overlay.y,
            ),
            body,
        );

        assert_eq!(app.selected, 0);
    }

    #[test]
    fn jq_filter_replaces_the_tree_with_navigable_stream_results() {
        let mut app = App::new(
            json!({"users": [{"name": "Ada", "active": true}, {"name": "Lin", "active": false}]}),
            "test".into(),
            1,
        );
        set_input(
            &mut app,
            ".users[] | select(.active) | .name",
            InputMode::Filter,
        );

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(
            app.active_filter.as_deref(),
            Some(".users[] | select(.active) | .name")
        );
        assert_eq!(app.filter_output_count, Some(1));
        assert_eq!(app.tree.value_at(0), &json!("Ada"));
        assert_eq!(app.selected, 0);
    }

    #[test]
    fn jq_filter_previews_live_without_replacing_the_document() {
        let original = json!({"users": [{"name": "Ada"}, {"name": "Lin"}]});
        let mut app = App::new(original.clone(), "test".into(), 1);

        app.handle_key(key(KeyCode::Char('|')));
        type_text(&mut app, ".users[].name");

        assert_eq!(app.tree.value_at(0), &original);
        assert!(app.is_filter_preview_pending());
        let spinner_before = app.filter_spinner_frame();
        assert!(app.process_filter_preview(Instant::now() + FILTER_SPINNER_INTERVAL));
        assert_ne!(app.filter_spinner_frame(), spinner_before);

        finish_filter_preview(&mut app);

        assert_eq!(app.input_mode, InputMode::Filter);
        assert_eq!(app.tree.value_at(0), &original);
        assert_eq!(app.active_filter, None);
        assert_eq!(app.filter_output_count, None);
        assert_eq!(app.filter_preview(), Some((&json!(["Ada", "Lin"]), 2)));
        let cached_lines = app
            .filter_preview_lines()
            .expect("valid filter has cached preview lines");
        let cached_lines_address = cached_lines.as_ptr();
        assert_eq!(cached_lines, ["[", "  \"Ada\",", "  \"Lin\"", "]"]);

        app.handle_key(key(KeyCode::Down));
        app.handle_key(key(KeyCode::PageDown));
        assert_eq!(app.filter_preview_scroll(), 6);
        assert_eq!(
            app.filter_preview_lines().unwrap().as_ptr(),
            cached_lines_address
        );
        app.handle_key(key(KeyCode::Up));
        assert_eq!(app.filter_preview_scroll(), 5);

        let body = Rect::new(0, 3, 100, 20);
        let overlay = ui::filter_overlay_area(body);
        app.handle_mouse(
            mouse(
                MouseEventKind::ScrollDown,
                overlay.x.saturating_add(1),
                overlay.y.saturating_add(1),
            ),
            body,
        );
        assert_eq!(app.filter_preview_scroll(), 8);
    }

    #[test]
    fn dragging_the_filter_scrollbar_handle_scrolls_the_live_preview() {
        let value = Value::Array((0..100).map(Value::from).collect());
        let mut app = App::new(value, "test".into(), 1);
        let body = Rect::new(0, 3, 100, 20);
        app.handle_key(key(KeyCode::Char('|')));
        type_text(&mut app, ".");
        finish_filter_preview(&mut app);

        // For this body, the live-preview scrollbar spans y=15..22 at x=87.
        app.handle_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 87, 15), body);
        assert!(matches!(
            app.scrollbar_drag,
            Some(ScrollbarDrag::FilterPreview { .. })
        ));

        app.handle_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 87, 21), body);
        assert_eq!(
            app.filter_preview_scroll(),
            ui::filter_preview_max_scroll(&app, body)
        );
    }

    #[test]
    fn jq_live_errors_keep_the_last_valid_preview() {
        let mut app = App::new(json!({"left": [1, 2], "right": [3, 4]}), "test".into(), 1);
        app.handle_key(key(KeyCode::Char('|')));
        type_text(&mut app, ".left");
        finish_filter_preview(&mut app);
        assert_eq!(
            app.tree.value_at(0),
            &json!({"left": [1, 2], "right": [3, 4]})
        );
        assert_eq!(app.filter_preview(), Some((&json!([1, 2]), 1)));

        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_text(&mut app, ".[\"");
        finish_filter_preview(&mut app);

        assert_eq!(
            app.tree.value_at(0),
            &json!({"left": [1, 2], "right": [3, 4]})
        );
        assert_eq!(app.filter_preview(), Some((&json!([1, 2]), 1)));
        assert_eq!(app.active_filter, None);
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("syntax error"))
        );
    }

    #[test]
    fn enter_keeps_and_escape_cancels_live_filter_results() {
        let original = json!({"left": [1, 2], "right": [3, 4]});
        let mut app = App::new(original.clone(), "test".into(), 1);
        set_input(&mut app, ".left", InputMode::Filter);
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(key(KeyCode::Char('|')));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_text(&mut app, ".right");
        finish_filter_preview(&mut app);
        assert_eq!(app.tree.value_at(0), &json!([1, 2]));
        assert_eq!(app.filter_preview(), Some((&json!([3, 4]), 1)));

        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.input_mode, InputMode::Normal);
        assert_eq!(app.tree.value_at(0), &json!([1, 2]));
        assert_eq!(app.active_filter.as_deref(), Some(".left"));

        app.handle_key(key(KeyCode::Char('|')));
        app.handle_key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
        type_text(&mut app, ".right");
        finish_filter_preview(&mut app);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.value_at(0), &json!([3, 4]));
        assert_eq!(app.active_filter.as_deref(), Some(".right"));

        app.handle_key(key(KeyCode::Esc));
        assert_eq!(app.tree.value_at(0), &original);
        assert!(app.active_filter.is_none());
    }

    #[test]
    fn every_jq_filter_runs_against_the_original_document() {
        let mut app = App::new(json!({"left": [1, 2], "right": [3, 4]}), "test".into(), 1);
        set_input(&mut app, ".left[]", InputMode::Filter);
        app.handle_key(key(KeyCode::Enter));
        assert_eq!(app.tree.value_at(0), &json!([1, 2]));

        set_input(&mut app, ".right[]", InputMode::Filter);
        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.tree.value_at(0), &json!([3, 4]));
        assert_eq!(app.filter_output_count, Some(2));
    }

    #[test]
    fn jq_errors_keep_the_editor_and_previous_tree_open() {
        let mut app = App::new(json!({"name": "Ada"}), "test".into(), 1);
        set_input(&mut app, ".[", InputMode::Filter);

        app.handle_key(key(KeyCode::Enter));

        assert_eq!(app.input_mode, InputMode::Filter);
        assert_eq!(app.input, ".[");
        assert!(
            app.message
                .as_deref()
                .is_some_and(|message| message.contains("syntax error"))
        );
        assert_eq!(app.tree.value_at(0), &json!({"name": "Ada"}));
        assert_eq!(app.active_filter, None);
    }

    #[test]
    fn escape_clears_search_before_restoring_the_unfiltered_document() {
        let original = json!({"left": [1, 2], "right": 3});
        let mut app = App::new(original.clone(), "test".into(), 1);
        set_input(&mut app, ".left", InputMode::Filter);
        app.handle_key(key(KeyCode::Enter));
        set_input(&mut app, "2", InputMode::Search);
        app.handle_key(key(KeyCode::Enter));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.search_query.is_none());
        assert!(app.active_filter.is_some());
        assert_eq!(app.tree.value_at(0), &json!([1, 2]));

        app.handle_key(key(KeyCode::Esc));
        assert!(app.active_filter.is_none());
        assert_eq!(app.tree.value_at(0), &original);
    }

    #[test]
    fn prompt_editing_inserts_and_deletes_at_a_unicode_cursor() {
        let mut app = App::new(json!(null), "test".into(), 1);
        set_input(&mut app, "aé", InputMode::Filter);
        app.handle_key(key(KeyCode::Left));
        app.handle_key(key(KeyCode::Char('!')));
        assert_eq!(app.input, "a!é");
        assert_eq!(app.input_cursor, 2);

        app.handle_key(key(KeyCode::Delete));
        assert_eq!(app.input, "a!");
        app.handle_key(key(KeyCode::Backspace));
        assert_eq!(app.input, "a");
        assert_eq!(app.input_cursor, 1);
    }
}
