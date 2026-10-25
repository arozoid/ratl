# ratl

`ratl` is a small declarative terminal-dialog tool in the spirit of `zenity`,
`yad`, and `gtkdialog` — except that every dialog is described in JSON and
rendered with [ratatui](https://ratatui.rs/).

It is intended for shell scripts, installer frontends, and quick forms that
need more structure than a plain prompt but should not pull in a graphical
toolkit.

```text
+─ ratl demo ─────────────────────────────────────────────+
│ A JSON-described form rendered entirely in the terminal.│
│ ┌ Profile ─────────────────────────────────────────────┐ │
│ │ Name                                                │ │
│ │ Ada Lovelace▏                                        │ │
│ │ Email                    Handle                      │ │
│ │ ada@example.com          @ada                        │ │
│ └──────────────────────────────────────────────────────┘ │
│ [x] Desktop notifications  [ ] Auto-save changes        │
│ Theme                      Volume                        │
│ (*) Catppuccin             [████████████░░░░] 60        │
│ ( ) Nord                                                 │
│ ( ) Solarized                                            │
│ ┌ Session ─────────────────────────────────────────────┐ │
│ │ Startup mode                                         │ │
│ │ (*) Normal                                           │ │
│ │ ( ) Safe                                             │ │
│ │ ( ) Recovery                                         │ │
│ │ [ ] Remember this choice                             │ │
│ └──────────────────────────────────────────────────────┘ │
│                                      [ Reset ] [ Cancel ] [ Save ] │
+──────────────────────────────────────────────────────────+
```

## Build

```sh
cargo build --release
install -Dm755 target/release/ratl ~/.local/bin/ratl
```

## Run

With no arguments, `ratl` loads `./ratl.json`:

```sh
ratl
```

Or pass a file / inline JSON:

```sh
ratl examples/form.json
ratl --json '{"title":"Hi","children":[{"type":"label","text":"Hello"}]}'
cat dialog.json | ratl
```

The repository's `ratl.json` is a complete demo, and
`examples/form.json` is a smaller ready-to-copy example.

`stdin` must be an interactive terminal because that is where the key events
come from. If `stdout` is redirected or captured, the dialog is drawn on the
controlling terminal (`/dev/tty`) and the JSON result still goes to stdout:

```sh
result=$(ratl examples/form.json)
```

Without a terminal, `ratl` exits `2`; use `--validate` for non-interactive
checks.

Validate a document without opening the TUI:

```sh
ratl --validate ratl.json
# {"buttons":3,"controls":8,"schema":"./schema.json","title":"ratl demo","valid":true}
```

Print the machine-readable schema:

```sh
ratl --schema > schema.json
```

## Keyboard

| Key | Action |
| --- | --- |
| `Tab` / `Shift+Tab` | Move focus |
| `Up` / `Down` | Change radio/select/slider, otherwise move focus |
| `Left` / `Right` | Change choice/slider, or move the text cursor |
| `Space` | Toggle a checkbox, cycle a choice, or press a button |
| `Home` / `End` | Jump to start/end of a field or range |
| `Enter` | Submit the dialog (or activate the focused button) |
| `Esc` / `Ctrl+C` | Cancel |

Long content scrolls automatically to keep the focused control visible.

## JSON output

On submit, the selected values are printed as JSON and the process exits `0`:

```json
{"action":"submit","button":"ok","values":{"name":"Ada Lovelace","email":"ada@example.com","handle":"@ada","notifications":true,"autosave":false,"theme":"Catppuccin","volume":60,"mode":"normal","remember":false}}
```

Cancel exits `1` and still prints the current values with
`"action":"cancel"`. Invalid JSON, duplicate control IDs, or a missing
interactive stdin exits `2` with a diagnostic on stderr.

## Layout model

The document is a tree of elements:

- **Containers**: `column`, `row`, `grid`, and `box` (a bordered container
  with a title, padding, gap, and a `column`/`row` direction).
- **Text**: `label`, `separator`, `spacer`.
- **Images**: `image` (PNG, JPEG, WebP, GIF, BMP, TIFF).
- **Controls**: `checkbox`, `radio`, `text`, `select`, `slider`.
- **Buttons**: a footer array with `submit`, `cancel`, and `reset` actions.

Rows and grids divide their available width evenly. Columns allocate the
natural height of each child. Long forms are vertically scrollable.

### Minimal example

```json
{
  "title": "Hello",
  "children": [
    { "type": "label", "text": "What should I call you?" },
    { "type": "text", "id": "name", "label": "Name", "value": "Ada" },
    { "type": "checkbox", "id": "loud", "label": "Be enthusiastic" }
  ]
}
```

### Images

`image` decodes a local file and paints it with true-colour half blocks, so it
works in any terminal that supports 24-bit colour (including `foot`). Paths are
relative to the directory containing the dialog JSON; `~/` and `$HOME/` are
expanded.

```json
{
  "type": "image",
  "path": "wallpapers/forest.png",
  "height": 12,
  "width": "80%",
  "fit": "contain",
  "align": "center",
  "valign": "center",
  "alt": "Wallpaper preview"
}
```

- `height` is in terminal rows (default `12`).
- `width` is an optional cell count or percentage; without it the image uses
  the available width.
- `fit` is `contain` (default), `cover`, or `stretch`.
- `align` is `left`, `center` (default), or `right`.
- `valign` is `top`, `center` (default), or `bottom`.
- `alt` is shown instead of the image when the file cannot be decoded.

Images are loaded once when the dialog opens, not on every redraw. The preview
is a terminal rendering, not a graphics protocol: it is deliberately portable
and does not need sixel or Kitty support.

### Options and controls

```json
{
  "type": "radio",
  "id": "mode",
  "label": "Startup mode",
  "options": [
    { "label": "Normal", "value": "normal" },
    { "label": "Safe", "value": "safe" }
  ],
  "selected": 0
}
```

`select` uses the same option format. It shows a scrolling window of up to
eight options, marks the current choice, and adds a `[current/total]` count to
the label when the list is longer. `Up`/`Down` changes the choice. `text`
supports `placeholder`, `password`, and `max_length`. `slider` supports `min`,
`max`, `step`, and `value`. Every interactive control needs a unique `id`; that
id becomes the key in the result object.

### Theming

Colors can be names (`"cyan"`, `"dark_gray"`, `"reset"`), hex strings
(`"#89b4fa"`), ANSI indexes (`12`), or RGB arrays (`[137, 180, 250]`). The
built-in default is a forest-green palette; a document `theme` replaces it.

```json
{
  "theme": {
    "foreground": "#cdd6f4",
    "background": "#1e1e2e",
    "label": "#cdd6f4",
    "muted": "#6c7086",
    "border": "#89b4fa",
    "title": "#89b4fa",
    "accent": "#a6e3a1",
    "focus_foreground": "#1e1e2e",
    "focus_background": "#89b4fa",
    "input_foreground": "#cdd6f4",
    "input_background": "#313244",
    "danger": "#f38ba8",
    "error": "#f38ba8"
  }
}
```

Individual elements also accept a `style` object with `fg`, `bg`, `bold`,
`italic`, `underlined`, and `dim`. `schema.json` is the authoritative list of
fields and defaults.
