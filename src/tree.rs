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
}

#[derive(Debug)]
pub struct JsonTree {
    value: Value,
    nodes: Vec<Node>,
}

impl JsonTree {
    pub fn new(value: Value, expand_depth: usize) -> Self {
        let mut tree = Self {
            value,
            nodes: Vec::new(),
        };
        tree.build(None, None, 0, expand_depth);
        tree
    }

    fn build(
        &mut self,
        parent: Option<NodeId>,
        segment: Option<Segment>,
        depth: usize,
        expand_depth: usize,
    ) -> NodeId {
        let value = match (&parent, &segment) {
            (None, None) => &self.value,
            (Some(parent), Some(segment)) => {
                let parent_value = self.value_at(*parent);
                match segment {
                    Segment::Key(key) => &parent_value[key],
                    Segment::Index(index) => &parent_value[*index],
                }
            }
            _ => unreachable!("only the root has no parent and segment"),
        };
        let kind = NodeKind::of(value);
        let summary = summary(value);
        let child_segments = match value {
            Value::Object(map) => map.keys().cloned().map(Segment::Key).collect(),
            Value::Array(items) => (0..items.len()).map(Segment::Index).collect(),
            _ => Vec::new(),
        };

        let id = self.nodes.len();
        self.nodes.push(Node {
            parent,
            children: Vec::new(),
            segment,
            depth,
            kind,
            summary,
            expanded: depth < expand_depth,
        });

        for child_segment in child_segments {
            let child = self.build(Some(id), Some(child_segment), depth + 1, expand_depth);
            self.nodes[id].children.push(child);
        }
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

        let mut value = &self.value;
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
        self.collect_visible(0, &mut visible);
        visible
    }

    fn collect_visible(&self, id: NodeId, visible: &mut Vec<NodeId>) {
        visible.push(id);
        let node = &self.nodes[id];
        if node.expanded {
            for &child in &node.children {
                self.collect_visible(child, visible);
            }
        }
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

    pub fn collapse_descendants(&mut self, id: NodeId) {
        let children = self.nodes[id].children.clone();
        for child in children {
            self.nodes[child].expanded = false;
            self.collapse_descendants(child);
        }
    }

    pub fn expand_descendants(&mut self, id: NodeId) {
        self.nodes[id].expanded = true;
        let children = self.nodes[id].children.clone();
        for child in children {
            self.expand_descendants(child);
        }
    }

    pub fn searchable_text(&self, id: NodeId) -> String {
        format!(
            "{} {} {}",
            self.path(id),
            self.label(id),
            self.nodes[id].summary
        )
        .to_lowercase()
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
}
