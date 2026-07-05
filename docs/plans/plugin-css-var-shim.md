# Plan 3 — OG `--ls-*` CSS-variable alias shim (theme compat)

**Status:** grounded, ready to execute · **Est:** ~1–2 days · **Backlog:** P2

## Goal

Make the **Awesome-Styler theme family** (and common file-version Logseq themes)
"mostly work" when a user drops one into `logseq/custom.css`, by defining the OG
`--ls-*` CSS variables those themes recolor and aliasing them onto Tine's own
semantic tokens. This is a **theme-compat slice, NOT plugin support** (no
`@logseq/libs`, no JS runtime — that stays WONTFIX). Success = the popular
Awesome-Styler variants change Tine's background/text/link/accent/border/code
colors as intended, without the user editing anything.

## Current state (grounded)

- **Tine's tokens** live in one file: `src/styles/theme.css`. Structural tokens in
  `:root`; color tokens duplicated under `html[data-theme="light"]` and
  `html[data-theme="dark"]`. Theme switch = the `data-theme` attribute on `<html>`
  (no `.dark` class, no media query).
- Tine already uses some OG-style names verbatim in `:root`: `--ls-font-family`,
  `--ls-font-mono`, `--ls-main-content-max-width`, `--ls-left-sidebar-width`,
  `--ls-border-radius-low`, `--ls-page-title-size`, etc. So the shim only needs the
  **color** aliases plus a few structural ones OG themes touch.
- Tine's color tokens (both themes): `--bg-primary/secondary/tertiary/quaternary`,
  `--text-primary/secondary/title/muted`, `--link-color`, `--link-hover`,
  `--tag-color`, `--border-color`, `--guide-color`/`--guide-hover`, `--bullet-color`,
  `--bullet-ring`, `--selection-bg`, `--block-select-bg`, `--block-highlight`,
  `--code-bg`, `--mark-bg`, `--mark-text`, `--marker-color`, `--done-color`.
- **No `--accent` token exists.** OG's de-facto accent (`--ls-active-primary-color`
  / block-ref color) has no single Tine equivalent; `--link-color` is the closest.
  → **Sub-task:** introduce a real `--accent` token in `theme.css` (both themes),
  point existing link/tag/bullet-active uses at it where sensible, and map the OG
  accent vars onto it. This also clears the "referenced-but-never-set `--accent`"
  note in the backlog theming row.
- **custom.css injection:** `src/graph.ts` `injectCustomCss()` (~lines 144–159) reads
  `read_custom_css` (`src-tauri/src/commands.rs:280` → graph's `logseq/custom.css`)
  and sets `.textContent` on a single `<style id="tine-custom-css">` appended to
  `document.head`.
- **OG `--ls-*` names** the themes target (from `/aux/koutecky/logseq/og`
  `resources/css/shui.css`, `src/main/frontend/common.css`): backgrounds
  `--ls-primary/secondary/tertiary/quaternary-background-color`; text
  `--ls-primary/secondary/title-text-color`; links/accent `--ls-link-text-color`,
  `--ls-link-text-hover-color`, `--ls-link-ref-text-color`,
  `--ls-block-ref-link-text-color`, `--ls-active-primary-color`, `--ls-a-chosen-bg`;
  borders `--ls-border-color`, `--ls-secondary-border-color`, `--ls-guideline-color`;
  bullets/highlight `--ls-block-bullet-color`, `--ls-block-bullet-active-color`,
  `--ls-block-highlight-color`; selection `--ls-selection-background-color`,
  `--ls-selection-text-color`; code `--ls-page-inline-code-color`,
  `--ls-page-inline-code-bg-color`; tag/mark `--ls-tag-text-color`,
  `--ls-page-mark-color`, `--ls-page-mark-bg-color`; states
  `--ls-warning/error/success-text-color` (+ `-background-color`).

## Approach

**A shim stylesheet, not a rename.** Add a new stylesheet that *defines* each OG
`--ls-*` color var as `var(--tine-token)`, at `:root`/`html` scope so it resolves
under whichever `data-theme` block is active. This makes two things work:
1. Tine's own components keep reading Tine tokens (unchanged).
2. A theme's `custom.css` that does `--ls-primary-background-color: #123` **overrides
   the alias**, and — critically — Tine's components must then pick that up. They
   won't automatically, because they read `--bg-primary`, not the OG var.

**So the alias must flow the OTHER way for overrides to take effect.** The shim
therefore does the reverse binding: Tine's tokens fall back *through* the OG vars.
i.e. in the shim, redefine Tine's tokens as:
```css
:root {
  --bg-primary: var(--ls-primary-background-color, <tine-default>);
  --link-color: var(--ls-link-text-color, var(--accent, <tine-default>));
  /* … */
}
```
and separately seed the OG vars so unstyled Tine still renders:
```css
:root { --ls-primary-background-color: <tine-default>; /* … */ }
```
This way: no theme → Tine defaults; a theme setting `--ls-*` → Tine components
recolor because their tokens resolve through the OG var. This is the load-bearing
design decision (worth a short note in the theming section, not a full ADR).

**Cascade / injection order.** Inject the shim `<style id="tine-ls-shim">` into
`document.head` **before** `tine-custom-css` (give it its own id; insert it earlier
in head). Order: base `theme.css` → shim → user `custom.css`. User CSS loads last →
wins. Adjust `injectCustomCss()` to ensure the shim exists and precedes the custom
node (create the shim node at app start or lazily before the custom node).

## Steps

1. Add a real `--accent` token to `src/styles/theme.css` (both themes); repoint
   link/active-bullet uses at it where it's genuinely the accent.
2. Create `src/styles/ls-shim.css`: (a) seed OG `--ls-*` color vars with Tine
   defaults; (b) redefine Tine color tokens to resolve *through* the OG vars (fallback
   to the seed). Cover the mapping table below.
3. Import the shim so it loads after `theme.css` but is overridable by custom.css
   (import in the same place theme.css is imported, immediately after it), and make
   `injectCustomCss()` append the custom `<style>` *after* the shim.
4. Structural OG vars: alias the handful themes touch (`--ls-main-content-max-width`,
   `--ls-page-title-size`, `--ls-border-radius-low`) — most already share names.
5. Test with 2–3 real Awesome-Styler variants dropped into a scratch graph's
   `logseq/custom.css`; screenshot before/after in both light & dark.
6. Docs: FEATURES (theming note), a line in the backlog theming row, CHANGELOG.

## Mapping table (OG `--ls-*` → Tine token)

```
--ls-primary-background-color    ↔ --bg-primary
--ls-secondary-background-color  ↔ --bg-secondary
--ls-tertiary-background-color   ↔ --bg-tertiary
--ls-quaternary-background-color ↔ --bg-quaternary
--ls-primary-text-color          ↔ --text-primary
--ls-secondary-text-color        ↔ --text-secondary
--ls-title-text-color            ↔ --text-title
--ls-link-text-color             ↔ --link-color (→ --accent)
--ls-link-text-hover-color       ↔ --link-hover
--ls-link-ref-text-color         ↔ --link-color
--ls-block-ref-link-text-color   ↔ --link-color
--ls-active-primary-color        ↔ --accent            (new token)
--ls-a-chosen-bg                 ↔ --block-select-bg
--ls-border-color                ↔ --border-color
--ls-secondary-border-color      ↔ --border-color
--ls-guideline-color             ↔ --guide-color
--ls-block-highlight-color       ↔ --block-highlight
--ls-block-bullet-color          ↔ --bullet-color
--ls-block-bullet-active-color   ↔ --accent
--ls-selection-background-color  ↔ --selection-bg
--ls-page-inline-code-bg-color   ↔ --code-bg
--ls-page-inline-code-color      ↔ --text-primary
--ls-page-mark-bg-color          ↔ --mark-bg
--ls-page-mark-color             ↔ --mark-text
--ls-tag-text-color              ↔ --tag-color
```

## Risks / decisions

- **The two-way binding is the subtle part.** If we only *define* OG vars as Tine
  tokens (one-way), theme overrides won't reach Tine components. The "Tine tokens
  resolve through OG vars" direction is what makes overrides work — verify with a
  real theme early (step 5) before writing the full table.
- **Radix `--rx-*`/`--lx-*` layer:** modern OG themes increasingly target Radix vars,
  which an `--ls-*` shim does NOT cover. Scope decision: this plan targets the
  classic `--ls-*` set (what Awesome-Styler uses); Radix compat is explicitly out of
  scope (note it). If a target theme is Radix-only, it won't work — accept and document.
- **Lossy accent:** OG's accent maps to one Tine `--accent`; themes that recolor
  several distinct OG accents will collapse. Acceptable for "mostly works".
- Non-color OG vars (spacing/sizes) mostly already share names — low risk.

## Acceptance

- With no custom.css: Tine looks identical to today (shim is transparent).
- Dropping a known Awesome-Styler variant into `logseq/custom.css` recolors bg / text
  / links / borders / code in **both** light and dark, no user edits — verified by
  before/after screenshots (headless harness OK for color; it's not WebKitGTK-specific).
- User `custom.css` still overrides everything (loads after the shim).
