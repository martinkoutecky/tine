# Tine themes (theme API 0.3)

Tine community themes are inert JSON packages. They do not contain WebAssembly,
JavaScript, CSS selectors, imports, font files, images, or network resources. A
package sets a bounded allowlist of Logseq-compatible semantic color variables for
light, dark, or both modes. API 0.2 may also select a few host-owned presentation
presets, and API 0.3 may name local fonts and a block line height. Tine generates the selector-bounded CSS and inserts it before the graph
owner's `logseq/custom.css`, which remains the final override.

## Package shape

Create a directory with `theme.json`:

```json
{
  "schemaVersion": 1,
  "id": "dev.example.my-theme",
  "name": "My theme",
  "version": "1.0.0",
  "apiVersion": "0.3",
  "description": "A calm light and dark palette.",
  "author": "Your name",
  "license": "MIT",
  "source": "https://github.com/you/my-tine-theme",
  "modes": {
    "light": {
      "--ls-primary-background-color": "#ffffff",
      "--ls-primary-text-color": "#303030",
      "--ls-active-primary-color": "#315efb"
    },
    "dark": {
      "--ls-primary-background-color": "#17181c",
      "--ls-primary-text-color": "#d9dbe1",
      "--ls-active-primary-color": "#86a5ff"
    }
  },
  "presentation": {
    "contentTypography": "editorial-serif",
    "journalHeader": "editorial",
    "todayTaskSummary": "compact"
  },
  "typography": {
    "bodyFont": "\"IBM Plex Sans\", -apple-system, sans-serif",
    "monoFont": "\"IBM Plex Mono\", Menlo, monospace",
    "titleFont": "\"IBM Plex Sans\", -apple-system, sans-serif",
    "blockLineHeight": 28
  },
  "screenshots": ["https://github.com/you/my-tine-theme/raw/main/screenshot.png"],
  "aiDevelopment": "primary"
}
```

Run `npm run theme:check -- /path/to/theme --json`, then install `theme.json`
from Settings → Appearance → Theme packages. The checker rejects unknown tokens,
CSS indirection, `url()`, `var()`, selector escapes, non-HTTPS metadata, and missing
registry licenses.

`presentation` is optional. The accepted API 0.2 values are:

| Field | Values | Host behavior |
| --- | --- | --- |
| `contentTypography` | `default`, `editorial-serif` | Reading/editor typography from a bundled system-font stack; the block line box is unchanged |
| `journalHeader` | `default`, `editorial` | Larger journal dates; Today is centered and omits the calendar glyph |
| `todayTaskSummary` | `hidden`, `compact` | A host-rendered count below Today, computed from that loaded journal page's canonical task facets |

The compact summary counts open tasks on the Today page. “In progress” is its
`DOING`, `NOW`, `STARTED`, and `IN-PROGRESS` subset. It does not run a theme-owned
query or scan the graph. Tine may later offer a separate indexed graph-wide count.

## Typography (API 0.3)

`typography` is optional and needs `"apiVersion": "0.3"`. It names fonts that
are already installed on the reader's device; a package cannot ship, link or
download a font.

| Field | Value | Where Tine applies it |
| --- | --- | --- |
| `bodyFont` | a font stack | Block text and the interface, and the block editor while you type |
| `monoFont` | a font stack | Code blocks, inline code, query and diagnostic text, the code editor |
| `titleFont` | a font stack | Page titles, heading blocks (also while editing) and the editorial journal title |
| `blockLineHeight` | an even integer, 20 to 48 (pixels) | The height of a block's first line; bullets, the editor and caret hit-testing follow it. Even only, so the bullet centre stays on a whole pixel |

A font stack is 1 to 8 comma-separated families, at most 240 characters; the only
whitespace is the plain space (a no-break space, U+2028 and the like are refused,
not trimmed). A family
is one of:

- a name made of letters, digits, `.`, `_`, `-` and single spaces (at most 64
  characters), bare or in one pair of matching quotes: `"IBM Plex Sans"`,
  `Helvetica Neue`, `'Segoe UI'`, `M PLUS 1p`;
- a generic or system keyword: `serif`, `sans-serif`, `monospace`, `cursive`,
  `fantasy`, `system-ui`, `ui-serif`, `ui-sans-serif`, `ui-monospace`,
  `ui-rounded`, `math`, `emoji`, `fangsong`, `-apple-system`, `BlinkMacSystemFont`.

Anything else is rejected, including `url()`, `@import`, `@font-face`, `var()`,
selectors, braces, semicolons, colons, comments, backslash escapes, quotes inside a
name, CSS-wide keywords (`inherit`, `initial`, ...), empty families, fractional,
odd or out-of-range line heights, and unknown keys. End each stack with a generic family
so a device without your font falls back sensibly. `tine-theme.mjs check` and the
app use the same grammar (`src/themes/fontStack.mjs`); the JSON schema
(`plugin-sdk/schema/theme.schema.json`) expresses as much of it as JSON Schema can,
and the checker stays the authority.

An older Tine (before theme API 0.3) keeps a 0.3 package, with a load warning, and
does not apply its typography.

Users choose typography apart from colors and style: **Use typography** on the
installed row (and the Typography selector in Settings → Appearance → Themes). If
the first family of a stack is not installed, the row says so and the rest of the
stack is used; installing never fails for it. Choosing the default, uninstalling or
revoking the package returns to Tine's own fonts. The user's own `custom.css`
still wins over a package.

Installed versions are addressed immutably by `id@version`. A version revoked by the
signed community registry cannot be installed or selected; if it was active, Tine
immediately clears the style and/or colors selected from that package while preserving
an independent safe selection. Uninstall remains
available so revocation never traps a package on the device.

## Ports

Behavioral or source-derived ports add `portedFrom` with the original ecosystem,
name, public source URL, immutable revision, license, authors, and relationship.
All seven are required; `tine-theme.mjs check` holds the package to the same
vocabulary Tine installs against, so a package it passes is one the app accepts.
A behavioral port preserves the visual design through Tine's semantic tokens; it
does not claim that Logseq or Obsidian selectors run unchanged.

Theme API 0.1 packages remain supported and cover colors only. API 0.2 adds the
presentation presets above, and API 0.3 adds `typography`. Packaged assets and advanced CSS are not silently
accepted; propose the smallest reusable host-owned extension instead.
