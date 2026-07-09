# Plan 4 — Built-in Theme Gallery (one-click themes on top of the shim)

**Status:** grounded, ready to execute · **Est:** ~1 day · **Backlog:** P2 ·
**Depends on:** Plan 3 (`plugin-css-var-shim.md`) landed.

## Goal

Give Tine a small **built-in theme gallery** in Settings → Appearance so a user
can try a different look in **one click**, live, with no file editing. This is
the user-facing surface for the `--ls-*` shim: the shim makes `--ls-*`-based CSS
recolor Tine; this plan makes a curated handful of such themes **discoverable and
one-click-applyable** from inside the app.

Scope is deliberately small — "a few favorites, nothing crazy". This is the
realistic substitute for "the OG marketplace, filtered to what works": a curated,
hand-vetted set, not a scraper.

## Non-goals (explicit)

- **No `@logseq/libs` / JS plugin runtime.** Stays WONTFIX (see theming backlog row).
- **No editing the user's `logseq/custom.css`.** The gallery applies themes
  *client-side only* (a managed `<style>`), never writes to the graph. The user's
  own `custom.css` keeps loading last and still overrides everything. (This is why
  no `write_custom_css` command is needed — and we avoid a data-safety surface on
  the real graph.)
- **No marketplace scrape / auto-compat-testing.** The "does this theme work"
  signal is human eyeball (per-theme, one-time); the set is a few entries, curated
  by hand. Plan 3(c) auto-scrape is out of scope, revisit only if the curated list
  outgrows manual upkeep.
- **No Radix (`--rx-*`/`--lx-*`) coverage.** Same boundary as the shim — gallery
  themes are classic `--ls-*` recolors only.

## Current state (grounded)

- **`AppearanceTab()`** — `src/components/Settings.tsx:322`. Today it holds only
  the light/dark `theme-switch`. The gallery is a new subsection *in this tab*, so
  3(a) "an Appearance pane" already exists; we extend it. No new tab/route.
- **Theme mode state** — `src/ui.ts`: `theme` signal, applied to
  `<html data-theme>` at startup (`applyTheme()` called once). NOTE it persists via
  `localStorage` (`THEME_KEY`), which the BACKLOG flags as a known bug — **WebKitGTK
  localStorage does not survive a restart** (memory `tine-localstorage-ephemeral`).
  The gallery must NOT repeat this: persist the selection through the backend (see
  Design §1), not localStorage.
- **App-level persistence that survives restart** — the generic
  `get_app_string`/`set_app_string` (+ bool) backend, written to
  `tine-settings.json`. Frontend API: `backend().getAppString(key, fallback)` /
  `setAppString(key, value)` (`backend.ts:248`). Device-local settings
  (`copySettings.ts`, `spellcheckSettings.ts`) use exactly this and read back at
  startup via an `init…()`. This is app config, NOT graph data — it does not touch
  the user's `logseq/` folder, so it's consistent with the data-safety non-goal.
- **Shim + custom.css injection** — `src/lsShim.ts` defines
  `LS_SHIM_STYLE_ID = "tine-ls-shim"` and `CUSTOM_CSS_STYLE_ID = "tine-custom-css"`.
  `graph.ts:injectCustomCss()` (~147) calls `ensureLsShimStyle()` then appends the
  user custom.css `<style>`. **Cascade today:** `theme.css` (bundled) → shim →
  user custom.css.
- **CSS-as-string bundling** — Vite `?inline` already used (`lsShim.ts:1`:
  `import lsShimCss from "./styles/ls-shim.css?inline"`). Bundled theme CSS imports
  the same way — build-time, offline, zero network permission.
- **No CSS write command** exists (`commands.rs` has only `read_custom_css`, :296).
  Good — the gallery needs none.

## Design

### 1. A managed theme layer (the load-bearing piece)

Add a third `<style>` node, **id `tine-theme`**, whose `.textContent` is the
selected built-in theme's CSS (empty string = "Default", i.e. none). Insert it in
the cascade **between the shim and the user custom.css**:

```
theme.css (bundled)  →  #tine-ls-shim  →  #tine-theme  →  #tine-custom-css (user)
     base                  aliases          gallery pick     user's own CSS wins
```

- Because it sits **after** the shim, a gallery theme's `--ls-*: …` overrides reach
  Tine components through the shim's reverse binding (exactly the mechanism the shim
  was built for).
- Because it sits **before** `#tine-custom-css`, a user who *also* hand-wrote a
  `logseq/custom.css` still wins — the gallery never fights the user's own CSS.
- Ordering is by DOM insertion order in `<head>`. `ensureThemeStyle()` must insert
  `#tine-theme` immediately after `#tine-ls-shim` and ensure `injectCustomCss()`
  still appends `#tine-custom-css` last (it already appends last; just guarantee the
  theme node exists before it, mirroring how the shim is ensured).

Put `ensureThemeStyle()` + `applyTheme(id)` + the selection signal + an
`initThemeGallery()` in a new `src/themeGallery.ts` (peer of `lsShim.ts`,
structured like `spellcheckSettings.ts`). **Persist the selected id via
`backend().setAppString("theme.gallery", id)` and read it back at startup with
`initThemeGallery()` → `getAppString("theme.gallery", "")`** (survives restart;
NOT localStorage — see current-state note). `initThemeGallery()` is called at
startup next to where `ensureLsShimStyle()` / mode `applyTheme()` are wired
(`main.tsx` / `App.tsx`); it awaits the stored id then applies it. Until it
resolves, no theme is applied (stock look) — acceptable, it's one IPC round-trip.

**Interaction with light/dark:** a theme may define colors for both `data-theme`
blocks or only one. Each bundled theme's CSS should scope its overrides the same
way `theme.css` does (`html[data-theme="light"]` / `[data-theme="dark"]`) so it
tracks the user's existing light/dark toggle. Themes that only cover one mode:
mark them "light-only"/"dark-only" in the card (see badges) rather than trying to
synthesize the other mode.

### 2. Bundled themes

Ship the favorites as CSS files under `src/styles/themes/<id>.css`, imported
`?inline` into a static registry `src/styles/themes/index.ts`:

```ts
export interface GalleryTheme {
  id: string;            // stable, kebab
  name: string;          // display
  author: string;        // "Tine" for our own, or upstream author + attribution
  compat: "full" | "partial";
  modes: ("light" | "dark")[];
  css: string;           // imported via ?inline
  // thumbnail: generated screenshot path (see harness step), NOT hand-drawn
}
```

**Seed the set with Tine-authored recolor themes** — this is the "nothing crazy,
few favorites" core and it sidesteps license/redistribution entirely (palette
*color values* like Nord/Solarized/Gruvbox are public facts, not copyrighted CSS;
we author the `--ls-*` rules ourselves). Proposed initial set (all guaranteed
`compat: "full"` because they only set `--ls-*` the shim maps):

- `nord` — Nord palette (light + dark)
- `solarized` — Solarized (light + dark)
- `gruvbox` — Gruvbox (light + dark)

Optionally add **1–2 vetted community `--ls-*` themes** as a stretch, but ONLY if
(a) they pass the harness recolor check below, (b) their license permits bundling
(MIT/similar) and we add attribution in `author` + a `THIRD-PARTY-THEMES.md`, and
(c) they degrade sanely (partial is fine, broken is not). Do NOT pad the set with
themes you haven't screenshot-verified — an empty-but-honest gallery beats a
listed-but-broken one. If the shim's Plan 3 step-5 validation already confirmed
specific real themes recolor, prefer reusing exactly those.

### 3. Gallery UI (in `AppearanceTab`)

A `settings-section` "Themes" below the light/dark switch:

- A responsive grid of cards. Each card = thumbnail + name + author + a compat
  badge (`Full` / `Partial` / `Light-only` / `Dark-only`). Plus a "Default" card
  (clears the selection → empty `#tine-theme`).
- **Click applies instantly** (live preview — the whole app recolors), and marks
  the card selected. No "Apply" button, no confirm. This is the "very easy to try"
  bar: click, see it, click Default to revert.
- A one-line hint under the grid: "Themes recolor Tine using Logseq's `--ls-*`
  variables. If you keep your own `logseq/custom.css`, it still takes priority."
  (So the custom.css interaction is stated, not silent — OG-parity working
  agreement point 2.)

Keep it consistent with the existing `Field`/section styling in Settings.tsx; no
new design language.

## Steps

1. `src/themeGallery.ts`: selection signal + `localStorage` key,
   `ensureThemeStyle()` (insert `#tine-theme` after `#tine-ls-shim`), `applyTheme(id)`
   (look up registry, set `.textContent`, persist). Guarantee cascade order vs
   `#tine-custom-css`.
2. Wire `applyTheme(stored)` at startup alongside the existing mode `applyTheme()`
   and `ensureLsShimStyle()`; make sure it runs before/independent of graph open
   (theme is app-level, not graph-level — unlike `injectCustomCss`).
3. `src/styles/themes/{nord,solarized,gruvbox}.css` + `index.ts` registry
   (`?inline`). Author each as `--ls-*`-only, scoped per `data-theme`.
4. Extend `AppearanceTab` (`Settings.tsx:322`) with the "Themes" grid + Default
   card + hint, driven by the registry and the selection signal.
5. **Harness verification (this is the acceptance gate)** — see below.
6. Docs: FEATURES (theming section — gallery + how it composes with custom.css),
   README highlight bullet if warranted, BACKLOG (close the "gallery UI" line, keep
   Radix/community-scrape as Deferred), CHANGELOG. If a community theme is bundled,
   add `THIRD-PARTY-THEMES.md` with license + attribution.

## Harness verification (acceptance gate)

Per the working agreement, verify visually myself before handing to Martin — this
is the "or do it with your harness" path he asked for. Using the headless
screenshot harness (`docs/SCREENSHOTS.md`):

- For **each** bundled theme × **each** mode it declares: screenshot a
  representative page (blocks + a link/ref + inline code + a tag) with the theme
  applied, and confirm background / text / link / border / code actually recolor
  (not just the toggle state changing). Eyeball each image.
- Confirm **Default** returns to stock Tine (theme node empty → identical to today).
- Confirm the **cascade**: with a scratch `logseq/custom.css` that sets, say,
  `--ls-primary-background-color: hotpink`, the user CSS wins over the selected
  gallery theme (proves `#tine-custom-css` loads after `#tine-theme`).
- The passing screenshots double as the card **thumbnails** (generate small crops;
  don't hand-draw). A harness hook may be needed to force Settings→Appearance open
  and apply a theme id (mirror existing hooks like `__FORCE_WELCOME__` /
  `__tineOpenAudio`); add `__tineApplyTheme(id)` if the state isn't reachable from
  the mock backend.

## Risks / decisions

- **Cascade order is the whole correctness story.** If `#tine-theme` ends up after
  `#tine-custom-css`, the gallery silently overrides the user's own CSS — a
  regression. The insertion-order guarantee + the hotpink cascade test above are
  the guard. Worth a one-paragraph note in the theming docs (not a full ADR unless
  the managed-layer idea grows).
- **Partial themes.** A `--ls-*` theme that also relies on OG selectors will
  recolor but not fully restyle. That's why the badge exists and why the core set
  is Tine-authored pure-recolor (always `full`). Don't ship anything that looks
  broken; "partial" must still look intentional.
- **License (community themes only).** Bundling third-party CSS redistributes it —
  gate on MIT/permissive + attribution, or drop it. The Tine-authored core set has
  no such concern (we wrote it; palette values aren't copyrightable).
- **Light/dark asymmetry.** Themes covering one mode declare it and are badged;
  don't fabricate the missing mode.

## Acceptance

- Settings → Appearance shows a Themes grid; clicking a card **instantly** recolors
  the whole app; clicking Default reverts to stock — verified by before/after
  screenshots in the mode(s) each theme declares.
- With no selection and no user custom.css, Tine is byte-identical to today
  (`#tine-theme` empty, transparent).
- A user `logseq/custom.css` still overrides the selected gallery theme (cascade
  test passes).
- The gallery contains only themes whose recolor was screenshot-confirmed; each
  card's badge honestly reflects its coverage.
- Nothing is written to the user's graph by selecting/deselecting a theme.
