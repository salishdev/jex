# jex

`jex` is a navigation-first terminal JSON explorer. It treats a document as a tree you can move through structurally, so finding your way around a large file does not depend on scrolling through a wall of formatted text.

This initial version deliberately focuses on browsing and extraction. It does not evaluate arbitrary `jq` expressions yet.

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

The left pane is the document tree. The right pane shows the complete JSON value at the selected node, and the header always shows its JSON Pointer.

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

You can also drag the divider between the tree and value panes with the mouse. Both panes retain a minimum usable width.

## Finding and returning

- `/` searches keys, JSON Pointer paths, and scalar values across the entire document. A match is revealed even when its ancestors were collapsed. Use `n` and `N` to cycle through results.
- `:` jumps directly to a JSON Pointer such as `/users/0/profile/name`. `/` and `$` both refer to the root.
- `b` and `f` move backward and forward through search and path jumps.
- `m` marks the current node; `'` returns to it.
- `Esc` clears the current search.

JSON Pointer escaping follows RFC 6901: `/` inside an object key becomes `~1`, and `~` becomes `~0`.

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
