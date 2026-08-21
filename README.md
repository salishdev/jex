# jex

`jex` is a navigation-first terminal JSON explorer. It treats a document as a tree you can move through structurally, so finding your way around a large file does not depend on scrolling through a wall of formatted text.

It also embeds a jq-compatible filter engine, so you can narrow or transform a document without leaving the explorer.

## Run it

```sh
cargo run -- data.json
```

Piped input works on macOS and Linux while keyboard input remains attached to the terminal:

```sh
curl -s https://example.com/data.json | cargo run
```

To install the development build on your path:

```sh
cargo install --path .
jex data.json
```

Use `--expand-depth N` (or `-d N`) to control how much of the tree is initially open.

## Navigation

The left pane is the document tree. The right pane shows the complete JSON value at the selected node, with a breadcrumb trail above it. Click any ancestor in the trail to navigate there; breadcrumb jumps are included in back/forward history. The header always shows the selected node's JSON Pointer.

| Key | Action |
| --- | --- |
| `j` / `k`, `↓` / `↑` | Next / previous visible node |
| `l` / `→` | Expand a container, then move to its first child |
| `h` / `←` | Collapse a container, then move to its parent |
| `[` / `]` | Previous / next sibling |
| `g` / `G` | First / last visible node |
| `Ctrl-u` / `Ctrl-d` | Move up / down by a page |
| `Space` / `Enter` | Toggle the selected container |
| `e` / `c` | Expand / collapse the whole selected branch |
| `-` / `+` | Shrink / grow the tree pane |

You can click a row in the tree to select it, or click its disclosure marker to expand or collapse it. The mouse wheel follows the pointer: over the tree it changes the selected row, and over the value pane it scrolls the formatted JSON. You can also drag the divider between the panes; both retain a minimum usable width.

## Finding and returning

- `/` searches keys, JSON Pointer paths, and scalar values across the entire document. A match is revealed even when its ancestors were collapsed. Use `n` and `N` to cycle through results.
- `:` jumps directly to a JSON Pointer such as `/users/0/profile/name`. `/` and `$` both refer to the root.
- `b` and `f` move backward and forward through search and path jumps.
- `m` marks the current node; `'` returns to it.
- `Esc` clears the current search.

JSON Pointer escaping follows RFC 6901: `/` inside an object key becomes `~1`, and `~` becomes `~0`.

## Filtering with jq

Press `|` to edit a jq-compatible expression. The result tree updates as you type; press `Enter` to keep the current result or `Esc` to cancel the edit and restore the previous tree. Filters always run against the original input document, so editing an applied filter does not create an implicit chain of transformations:

```jq
.users[] | select(.active) | {name, email}
```

The filtered value becomes a normal browsable tree. A filter that emits several values displays them as a result array; a filter that emits no values displays an empty array. The header shows the active expression and its output count.

While editing, use the arrow keys, `Home`, `End`, `Delete`, `Ctrl-u`, and `Ctrl-w` for line editing. Live evaluation is briefly debounced and runs on a background worker, so expensive expressions do not block typing. Syntax and runtime errors leave the last valid preview in place while you correct the expression. In normal mode, `Esc` clears an active search first, then clears an applied filter and restores the original document.

The embedded engine is [jaq](https://github.com/01mf02/jaq), which supports a large jq-compatible language without requiring a separate `jq` installation. Filter output is limited to 10,000 values to protect the interactive UI from unbounded result streams.

## Extracting a value

Press `p` to close the UI and print the selected JSON value, or `P` to print its JSON Pointer. The terminal UI uses the controlling terminal separately from stdout on macOS and Linux, so extraction can be redirected cleanly:

```sh
jex data.json > selected.json
```

Press `?` in the app for the complete shortcut reference, or `q` to quit without output.

## Development

```sh
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```
