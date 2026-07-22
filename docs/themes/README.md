# Tine token themes (theme API 0.1)

Tine community themes are inert JSON packages. They do not contain WebAssembly,
JavaScript, CSS selectors, imports, fonts, images, or network resources. A package
sets a bounded allowlist of Logseq-compatible semantic color variables for light,
dark, or both modes. Tine generates the selector-bounded CSS and inserts it before
the graph owner's `logseq/custom.css`, which remains the final override.

## Package shape

Create a directory with `theme.json`:

```json
{
  "schemaVersion": 1,
  "id": "dev.example.my-theme",
  "name": "My theme",
  "version": "1.0.0",
  "apiVersion": "0.1",
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
  "screenshots": ["https://github.com/you/my-tine-theme/raw/main/screenshot.png"],
  "aiDevelopment": "primary"
}
```

Run `npm run theme:check -- /path/to/theme --json`, then install `theme.json`
from Settings → Appearance → Theme packages. The checker rejects unknown tokens,
CSS indirection, `url()`, `var()`, selector escapes, non-HTTPS metadata, and missing
registry licenses.

Installed versions are addressed immutably by `id@version`. A version revoked by the
signed community registry cannot be installed or selected; if it was active, Tine
immediately clears its generated style and returns to Default. Uninstall remains
available so revocation never traps a package on the device.

## Ports

Behavioral or source-derived ports add `portedFrom` with the original ecosystem,
name, public source URL, immutable revision, license, authors, and relationship.
A behavioral port preserves the visual design through Tine's semantic tokens; it
does not claim that Logseq or Obsidian selectors run unchanged.

Theme API 0.1 intentionally covers colors only. Typography, spacing, packaged
assets, and advanced CSS are not silently accepted; propose the smallest reusable
token extension instead.
