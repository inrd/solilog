# Themes

Solilog ships with built-in themes (`dark`, `light`, `8086`, `mmx`, `sse`, `sse2`,
and `sse2-dark`) and loads any custom JSON themes you place in your themes folder.

## Switching themes

- Open the command palette (`⌘K` on macOS, `Ctrl+K` elsewhere) and run
  **Workspace: Set Theme**.
- Or bind a specific theme in your `keymap.json`:

```json
[
  {
    "bindings": {
      "g t": ["workspace::SetTheme", { "name": "mmx" }]
    }
  }
]
```

Your chosen theme is saved in the database and restored on startup.

## Creating a custom theme

Drop a `.json` file into your user themes folder:

- `$SOLILOG_CONFIG_DIR/themes/<id>.json`, else
- `$XDG_CONFIG_HOME/solilog/themes/<id>.json`, else
- `~/.config/solilog/themes/<id>.json`

You can open (and create) this folder from the command palette with
**Workspace: Open Themes Directory**.

Edits to files in this folder hot-reload automatically while Solilog is running.
The file stem becomes the theme's `id` (e.g. `solarized.json` → `"solarized"`).
A user file with the same id as a built-in theme (`dark.json`, `mmx.json`)
overrides it.

Comments (`//`, `/* ... */`) and trailing commas are allowed.

### Minimal example

Every theme can inherit from a built-in theme using `"extends"` (`"dark"` by
default, or `"mmx"`) and override only the fields it wants to change:

```json
// ~/.config/solilog/themes/hotdog.json
{
  "name": "Hot Dog Stand",
  "extends": "mmx",
  "colors": {
    "surface": "#ff0000",
    "sidebar": "#ff0000",
    "background": "#ffff00",
    "input": "#ffffff",
    "highlight": "#000000",
    "highlight_text": "#ffffff",
    "text": "#000000",
    "accent": "#000000",
  }
}
```

## Schema reference

### Top-level fields

| Field | Values | Description |
|---|---|---|
| `name` | string | Display name shown in the theme picker (defaults to the file id). |
| `extends` | `"dark"` \| `"light"` \| `"8086"` \| `"mmx"` \| `"sse"` \| `"sse2"` \| `"sse2-dark"` | Built-in base theme to inherit omitted fields from (default: `"dark"`). |
| `appearance` | `"dark"` \| `"light"` | Tells the OS (e.g. macOS window chrome & scrollbars) whether the theme is dark or light. |
| `chrome` | `"flat"` \| `"bevel"` \| `"glass"` \| `"crt"` | `"flat"` uses hairline borders; `"bevel"` draws 3D raised/sunken classic desktop borders, sunken pane wells, pressed button states, and dotted focus outlines; `"glass"` draws glossy two-tone specular highlights, glowing selection rows, and gel progress bars/orbs; `"crt"` adds subtle horizontal CRT scanlines and crisp terminal chrome. |
| `titlebar` | `"native"` \| `"classic"` \| `"glass"` \| `"crt"` \| `"luna"` | `"native"` uses the system titlebar; `"classic"` draws a retro gradient titlebar with raised `[_][□][×]` buttons and a raised outer window frame; `"glass"` draws a glossy gradient titlebar with capsule window buttons; `"crt"` draws a reverse-video `[ SOLILOG ]` terminal bar with square bordered controls; `"luna"` draws a bold early-2000s titlebar (bright top rim fading into the body color) with rounded white-rimmed buttons and a red close button, and frames the window in the titlebar color. |
| `icons` | `"default"` \| `"pixel"` | `"default"` uses smooth vector icons; `"pixel"` switches UI and status icons to chunky 16×16 pixel-art variants. |
| `radius` | number (`>= 0`) | Base corner radius in pixels (`0` for sharp corners, `4` for modern rounded corners). |
| `shadow` | `"soft"` \| `"hard"` \| `"none"` | Popup and tooltip shadow style (`"hard"` draws a crisp unblurred offset drop shadow). |
| `toolbars` | `"plain"` \| `"surface"` | What the bars above the main pane (list filters, issue, project and label headers, archive title) are drawn on: `"plain"` uses `background`, `"surface"` uses `surface`. Bevel themes always use `surface`. |
| `font_family` | string or `null` | Optional font family name applied across the workspace (`null` uses the system UI font; `"W95FA"` is bundled and `"monospace"` resolves to the platform monospace font). |
| `colors` | object | Color tokens (see below). |

### Color format

Colors are hex strings: `#rgb`, `#rrggbb`, or `#rrggbbaa` (with alpha).

### `colors`

| Token | Used for |
|---|---|
| `background` | Main pane background (issue list, open issue, archive). |
| `sidebar` | Sidebar and footer bar background. |
| `surface` | Elevated surfaces: popups, pickers, banners, review panels, buttons, and toolbar chrome in bevel themes. |
| `input` | Editable text wells and checkboxes (e.g. `#ffffff` in MMX). |
| `tooltip` | Tooltip popup background (e.g. `#ffffe1` in MMX). |
| `border` | Control and popup borders in flat themes. |
| `divider` | Hairline separators between panes and sections. |
| `hover` | Subtle hover highlight and unfocused cursor row. |
| `highlight` | Focused cursor row (issue list, sidebar, picker, archive). |
| `highlight_text` | Text color on a `highlight` row. |
| `selection` | Multi-selected issue rows. |
| `selection_highlight` | The focused cursor row when it is also multi-selected. |
| `selection_text` | Text color on `selection` / `selection_highlight` rows. |
| `text` | Primary text. |
| `text_muted` | Secondary text, counts, issue ids, placeholders, muted icons. |
| `accent` | Primary accent: links, cursor, progress bars, project chips, unreviewed indicator. |
| `error` | Error messages, failed checks, blocked status. |
| `review` | Review panel accent and `in_review` status. |
| `passed` | Passed checks in review receipts. |
| `text_selection` | Text selection highlight inside text inputs (typically translucent, e.g. `#7c8cff40`). |
| `overlay` | Backdrop behind the full-window image attachment preview. |
| `bevel_light` | Outer top/left edge of raised bevels (and outer bottom/right of sunken bevels). |
| `bevel_highlight` | Inner top/left edge of raised bevels. |
| `bevel_shadow` | Inner bottom/right edge of raised bevels (and outer top/left of sunken bevels). |
| `bevel_dark` | Outer bottom/right edge of raised bevels (and inner top/left of sunken bevels). |
| `titlebar_active_start` | Left gradient color of the classic titlebar when the window is focused (top rim in `"luna"`). |
| `titlebar_active_end` | Right gradient color of the classic titlebar when the window is focused (body and window frame in `"luna"`). |
| `titlebar_inactive_start` | Left gradient color of the classic titlebar when the window is unfocused. |
| `titlebar_inactive_end` | Right gradient color of the classic titlebar when the window is unfocused. |
| `titlebar_text` | Title text color on the focused classic titlebar. |
| `titlebar_inactive_text` | Title text color on the unfocused classic/glass titlebar. |
| `glass_highlight` | Translucent top-half specular sheen color used by `"chrome": "glass"` and `"titlebar": "glass"`. |
| `glass_glow` | Luminous border highlight color drawn around active/selected rows in `"chrome": "glass"`. |

### `colors.status`

Status icon colors: `backlog`, `todo`, `in_progress`, `in_review`, `blocked`,
`done`, `cancelled`, plus `glyph` (the checkmark/cross color drawn inside filled
status icons and project chips).

### `colors.labels`

The 9 label colors: `red`, `orange`, `yellow`, `green`, `teal`, `blue`,
`purple`, `pink`, `gray`.

## Built-in presets

For complete examples, see the built-in presets in the repository:

- `crates/solilog/assets/themes/dark.json`
- `crates/solilog/assets/themes/light.json`
- `crates/solilog/assets/themes/8086.json`
- `crates/solilog/assets/themes/mmx.json`
- `crates/solilog/assets/themes/sse.json`
- `crates/solilog/assets/themes/sse2.json`
- `crates/solilog/assets/themes/sse2-dark.json`
