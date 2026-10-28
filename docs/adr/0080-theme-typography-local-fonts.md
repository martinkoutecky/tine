# 0080. Theme API 0.3 lets a package name local fonts and a line height, nothing else about type

- **Status:** Accepted
- **Date:** 2026-10-28

## Context

ADR 0059 kept fonts out of theme packages: arbitrary CSS, fonts and markup would tie
an inert package to Tine's private DOM. The cost showed up in practice (GH #760):
typography-led ports such as Paper (IBM Plex) and iA Writer Duo install their colors
and lose the look that makes them recognizable, so authors tell users to paste a
companion block into `logseq/custom.css`, per graph, invisible in Settings and
unvalidated. The existing `editorial-serif` preset covers one serif and a 30px line.
The alternatives on the table were: bundled font files and `@font-face` (binary
assets, licensing, a download surface), a free `font-family` string (selector and
`url()` injection), or a bounded vocabulary of local font names.

## Decision

Martin, 2026-10-28 (D-20261028-S9B): "let theme packages declare fonts and line
height (local font names only, no CSS or downloads; a separate 'Use typography'
button)." We will:

- Add theme API 0.3 with one optional `typography` object: `bodyFont`, `monoFont`,
  `titleFont` (font stacks) and `blockLineHeight` (integer px, 20 to 48). API 0.1 and
  0.2 packages stay valid and cannot declare it.
- Define a font stack by a closed grammar (`src/themes/fontStack.mjs`): at most 8
  families, 240 characters; each a local name of letters, digits, `. _ -` and single
  spaces, or a generic/system keyword. The app parser and `tine-theme.mjs check`
  import that one module. Tine rebuilds the CSS from the parsed families (names are
  re-quoted), never from the manifest string.
- Map the fields onto existing public tokens (`--tine-content-font`,
  `--tine-editable-font`, `--tine-mono-font`) and one internal token for the line
  height, in the `#tine-theme` element that precedes `custom.css`. Tine keeps owning
  every selector, measurement and the caret/bullet line box (GH #459).
- Make typography a third, independent composition role beside style and colors,
  chosen by a separate **Use typography** action, so the choice is visible in
  Settings and reversible.
- Report a missing first family with a non-blocking hint on the installed row, found
  by a memoized canvas width probe (no font enumeration API exists); an environment
  that cannot measure shows nothing.

## Consequences

Authors can ship a typography-led theme as one package; no binary asset, licensing
or network surface is added. The grammar is public vocabulary: widening it
(font-size, weights, letter-spacing, bundled fonts) is a new decision with its own
line-box analysis. A user whose device lacks the font sees the fallback stack plus the
hint, which is honest but cannot tell a near-match from the real face. The persisted
composition record gains a `typography` key only when one is chosen, so a build that
predates it ignores the key.
