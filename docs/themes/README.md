# Tine themes (theme API 0.2)

Tine community themes are inert JSON packages. They do not contain WebAssembly,
JavaScript, CSS selectors, imports, font files, images, or network resources. A
package sets a bounded allowlist of Logseq-compatible semantic color variables for
light, dark, or both modes. API 0.2 may also select a few host-owned presentation
presets. Tine generates the selector-bounded CSS and inserts it before the graph
owner's `logseq/custom.css`, which remains the final override.

## Package shape

Create a directory with `theme.json`:

```json
{
  "schemaVersion": 1,
  "id": "dev.example.my-theme",
  "name": "My theme",
  "version": "1.0.0",
  "apiVersion": "0.2",
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

Theme API 0.1 packages remain supported and cover colors only. API 0.2 adds only
the presentation presets above. Packaged assets and advanced CSS are not silently
accepted; propose the smallest reusable host-owned extension instead.
