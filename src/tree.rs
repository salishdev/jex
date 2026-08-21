use std::sync::Arc;

use serde_json::Value;

pub type NodeId = usize;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Segment {
    Key(String),
    Index(usize),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NodeKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub children: Vec<NodeId>,
    pub segment: Option<Segment>,
    pub depth: usize,
    pub kind: NodeKind,
    pub summary: String,
    pub expanded: bool,
    pretty_start: usize,
    pretty_lines: usize,
    subtree_end: NodeId,
}

#[derive(Debug)]
pub struct JsonTree {
    value: Arc<Value>,
    nodes: Vec<Node>,
}

impl JsonTree {
    #[cfg(test)]
    pub fn new(value: Value, expand_depth: usize) -> Self {
        Self::from_shared(Arc::new(value), expand_depth)
    }

    pub fn from_shared(value: Arc<Value>, expand_depth: usize) -> Self {
        let mut nodes = Vec::new();
        Self::build_value(&value, &mut nodes, None, None, 0, expand_depth, 0);
        Self { value, nodes }
    }

    fn build_value(
        value: &Value,
        nodes: &mut Vec<Node>,
        parent: Option<NodeId>,
        segment: Option<Segment>,
        depth: usize,
        expand_depth: usize,
        pretty_start: usize,
    ) -> NodeId {
        let kind = NodeKind::of(value);
        let summary = summary(value);

        let id = nodes.len();
        nodes.push(Node {
            parent,
            children: Vec::new(),
            segment,
            depth,
            kind,
            summary,
            expanded: depth < expand_depth,
            pretty_start,
            pretty_lines: 1,
            subtree_end: id + 1,
        });

        match value {
            Value::Object(map) => {
                let mut child_start = pretty_start + 1;
                for (key, child_value) in map {
                    let child = Self::build_value(
                        child_value,
                        nodes,
                        Some(id),
                        Some(Segment::Key(key.clone())),
                        depth + 1,
                        expand_depth,
                        child_start,
                    );
                    child_start += nodes[child].pretty_lines;
                    nodes[id].children.push(child);
                }
            }
            Value::Array(items) => {
                let mut child_start = pretty_start + 1;
                for (index, child_value) in items.iter().enumerate() {
                    let child = Self::build_value(
                        child_value,
                        nodes,
                        Some(id),
                        Some(Segment::Index(index)),
                        depth + 1,
                        expand_depth,
                        child_start,
                    );
                    child_start += nodes[child].pretty_lines;
                    nodes[id].children.push(child);
                }
            }
            _ => {}
        }

        if !nodes[id].children.is_empty() {
            nodes[id].pretty_lines = 2 + nodes[id]
                .children
                .iter()
                .map(|&child| nodes[child].pretty_lines)
                .sum::<usize>();
        }
        nodes[id].subtree_end = nodes.len();
        id
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn node_mut(&mut self, id: NodeId) -> &mut Node {
        &mut self.nodes[id]
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn pretty_line_count(&self, id: NodeId) -> usize {
        self.nodes[id].pretty_lines
    }

    pub fn pretty_line_start(&self, id: NodeId) -> usize {
        self.nodes[id].pretty_start
    }

    pub fn value_at(&self, id: NodeId) -> &Value {
        let mut segments = Vec::new();
        let mut cursor = Some(id);
        while let Some(node_id) = cursor {
            let node = &self.nodes[node_id];
            if let Some(segment) = &node.segment {
                segments.push(segment);
            }
            cursor = node.parent;
        }

        let mut value = self.value.as_ref();
        for segment in segments.into_iter().rev() {
            value = match segment {
                Segment::Key(key) => &value[key],
                Segment::Index(index) => &value[*index],
            };
        }
        value
    }

    pub fn visible(&self) -> Vec<NodeId> {
        let mut visible = Vec::new();
        let mut pending = vec![0];
        while let Some(id) = pending.pop() {
            visible.push(id);
            let node = &self.nodes[id];
            if node.expanded {
                pending.extend(node.children.iter().rev().copied());
            }
        }
        visible
    }

    pub fn path(&self, id: NodeId) -> String {
        if id == 0 {
            return "/".into();
        }
        let mut segments = Vec::new();
        let mut cursor = Some(id);
        while let Some(node_id) = cursor {
            let node = &self.nodes[node_id];
            if let Some(segment) = &node.segment {
                segments.push(match segment {
                    Segment::Key(key) => escape_pointer(key),
                    Segment::Index(index) => index.to_string(),
                });
            }
            cursor = node.parent;
        }
        format!(
            "/{}",
            segments.into_iter().rev().collect::<Vec<_>>().join("/")
        )
    }

    pub fn label(&self, id: NodeId) -> String {
        match &self.nodes[id].segment {
            None => "$".into(),
            Some(Segment::Key(key)) => key.clone(),
            Some(Segment::Index(index)) => format!("[{index}]"),
        }
    }

    pub fn lineage(&self, id: NodeId) -> Vec<NodeId> {
        let mut lineage = Vec::with_capacity(self.nodes[id].depth + 1);
        let mut cursor = Some(id);
        while let Some(node_id) = cursor {
            lineage.push(node_id);
            cursor = self.nodes[node_id].parent;
        }
        lineage.reverse();
        lineage
    }

    pub fn find_pointer(&self, pointer: &str) -> Option<NodeId> {
        if pointer == "/" || pointer.is_empty() || pointer == "$" {
            return Some(0);
        }
        let pointer = pointer.strip_prefix('$').unwrap_or(pointer);
        if !pointer.starts_with('/') {
            return None;
        }
        let mut current = 0;
        for raw in pointer[1..].split('/') {
            let part = unescape_pointer(raw)?;
            current = self.nodes[current]
                .children
                .iter()
                .copied()
                .find(|&child| match &self.nodes[child].segment {
                    Some(Segment::Key(key)) => key == &part,
                    Some(Segment::Index(index)) => part.parse::<usize>() == Ok(*index),
                    None => false,
                })?;
        }
        Some(current)
    }

    pub fn reveal(&mut self, id: NodeId) {
        let mut cursor = self.nodes[id].parent;
        while let Some(parent) = cursor {
            self.nodes[parent].expanded = true;
            cursor = self.nodes[parent].parent;
        }
    }

    pub fn search(&self, query: &str) -> Vec<NodeId> {
        let mut matches = Vec::new();
        let mut path = String::new();
        let mut path_lengths = Vec::new();

        for (id, node) in self.nodes.iter().enumerate() {
            let label_matches;
            if node.depth == 0 {
                path.clear();
                label_matches = "$".contains(query);
            } else {
                path.truncate(path_lengths[node.depth - 1]);
                path.push('/');
                match node.segment.as_ref().expect("non-root nodes have segments") {
                    Segment::Key(key) => {
                        let key = key.to_lowercase();
                        label_matches = key.contains(query);
                        path.push_str(&escape_pointer(&key));
                    }
                    Segment::Index(index) => {
                        label_matches = (query.contains('[') || query.contains(']'))
                            && format!("[{index}]").contains(query);
                        path.push_str(&index.to_string());
                    }
                }
            }
            if path_lengths.len() <= node.depth {
                path_lengths.push(path.len());
            } else {
                path_lengths[node.depth] = path.len();
            }

            let path_matches = if id == 0 {
                "/".contains(query)
            } else {
                path.contains(query)
            };
            if path_matches || label_matches || node.summary.to_lowercase().contains(query) {
                matches.push(id);
            }
        }

        matches
    }

    pub fn collapse_descendants(&mut self, id: NodeId) {
        let subtree_end = self.nodes[id].subtree_end;
        for node in &mut self.nodes[id + 1..subtree_end] {
            node.expanded = false;
        }
    }

    pub fn expand_descendants(&mut self, id: NodeId) {
        let subtree_end = self.nodes[id].subtree_end;
        for node in &mut self.nodes[id..subtree_end] {
            node.expanded = true;
        }
    }
}

impl NodeKind {
    fn of(value: &Value) -> Self {
        match value {
            Value::Object(_) => Self::Object,
            Value::Array(_) => Self::Array,
            Value::String(_) => Self::String,
            Value::Number(_) => Self::Number,
            Value::Bool(_) => Self::Bool,
            Value::Null => Self::Null,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Self::Object => "object",
            Self::Array => "array",
            Self::String => "string",
            Self::Number => "number",
            Self::Bool => "boolean",
            Self::Null => "null",
        }
    }
}

fn summary(value: &Value) -> String {
    match value {
        Value::Object(map) => format!("{{{}}}", count_label(map.len(), "key", "keys")),
        Value::Array(items) => format!("[{}]", count_label(items.len(), "item", "items")),
        Value::String(text) => {
            let shortened: String = text.chars().take(80).collect();
            if shortened.chars().count() < text.chars().count() {
                format!("\"{shortened}…\"")
            } else {
                format!("\"{shortened}\"")
            }
        }
        other => other.to_string(),
    }
}

fn count_label(count: usize, singular: &str, plural: &str) -> String {
    format!("{count} {}", if count == 1 { singular } else { plural })
}

fn escape_pointer(value: &str) -> String {
    value.replace('~', "~0").replace('/', "~1")
}

fn unescape_pointer(value: &str) -> Option<String> {
    let mut result = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '~' {
            match chars.next() {
                Some('0') => result.push('~'),
                Some('1') => result.push('/'),
                _ => return None,
            }
        } else {
            result.push(ch);
        }
    }
    Some(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn builds_paths_and_resolves_pointers() {
        let tree = JsonTree::new(json!({"a/b": [{"~name": true}]}), 1);
        let id = tree.find_pointer("/a~1b/0/~0name").unwrap();
        assert_eq!(tree.path(id), "/a~1b/0/~0name");
        assert_eq!(tree.value_at(id), &json!(true));
    }

    #[test]
    fn visibility_tracks_expansion() {
        let mut tree = JsonTree::new(json!({"a": {"b": 1}, "c": 2}), 1);
        assert_eq!(tree.visible().len(), 3);
        let a = tree.find_pointer("/a").unwrap();
        tree.node_mut(a).expanded = true;
        assert_eq!(tree.visible().len(), 4);
    }

    #[test]
    fn reveal_opens_every_ancestor() {
        let mut tree = JsonTree::new(json!({"a": {"b": {"c": 1}}}), 0);
        let c = tree.find_pointer("/a/b/c").unwrap();
        tree.reveal(c);
        assert!(tree.visible().contains(&c));
    }

    #[test]
    fn lineage_runs_from_root_to_selected_node() {
        let tree = JsonTree::new(json!({"a": [{"b": 1}]}), 0);
        let b = tree.find_pointer("/a/0/b").unwrap();

        let labels = tree
            .lineage(b)
            .into_iter()
            .map(|id| tree.label(id))
            .collect::<Vec<_>>();

        assert_eq!(labels, vec!["$", "a", "[0]", "b"]);
    }

    #[test]
    fn search_matches_raw_keys_escaped_paths_and_values() {
        let tree = JsonTree::new(json!({"A/B": [{"name": "Needle"}]}), 0);
        let keyed = tree.find_pointer("/A~1B").unwrap();
        let value = tree.find_pointer("/A~1B/0/name").unwrap();

        assert!(tree.search("a/b").contains(&keyed));
        assert!(tree.search("a~1b/0").contains(&value));
        assert!(tree.search("needle").contains(&value));
        assert!(
            tree.search("[0]")
                .contains(&tree.find_pointer("/A~1B/0").unwrap())
        );
    }
}
