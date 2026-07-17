# Excalidraw assets — light integration (render + "Edit in Excalidraw")

Status: **IMPLEMENTED** (Jul 8 2026) — superseded by the generic external
media-editor registry built for GH #38 (`src/mediaEditors.ts`). Excalidraw is one
registry entry; the "Edit in Excalidraw" affordance, the configurable command
(Settings → Files → Diagram editors), the render path, and the focus-refresh all
come from that shared seam. This document is kept for the design rationale and the
export-with-*Embed scene* user workflow it documents. Only remaining gap vs. this
spec: §2.4 (a bare `.excalidraw` scene-JSON attachment chip carrying the edit
action) — the registry currently surfaces the edit button on rendered *images*
only; add it to the attachment chip if a user asks.

Original status: spec, not started (backlog P2). Decided Jul 5 2026.

## 1. Goal & non-goals

Serve the *freehand sketching* slice of the whiteboard/excalidraw user requests
without embedding an editor. The core trick is Excalidraw's own export format:
when exporting a drawing as SVG or PNG with **"Embed scene"** checked,
Excalidraw embeds the full editable scene JSON *inside the image file*
(SVG: metadata payload; PNG: a `tEXt` chunk). Convention-named
`foo.excalidraw.svg` / `foo.excalidraw.png` files are therefore
**simultaneously a preview and the source**: any markdown app renders them as
an ordinary image, and Excalidraw re-opens them for editing losslessly. (This
is exactly how the popular Obsidian excalidraw plugin interoperates.)

So the workflow Tine supports:

1. User draws at excalidraw.com (offline-capable PWA) or a desktop Excalidraw.
2. Exports with *Embed scene* into the graph's `assets/` as
   `name.excalidraw.svg` (preferred; crisp) or `.excalidraw.png`.
3. References it like any image: `![sketch](../assets/name.excalidraw.svg)`.
4. Tine renders it as an image; **right-click → "Edit in Excalidraw"** opens it
   externally; on re-export/overwrite, Tine refreshes the rendered image.

**Non-goals (decided, do not revisit without Martin):**
- **No embedded Excalidraw editor.** It is a React component (Tine is Solid),
  a multi-MB dependency, a WebKitGTK canvas-perf risk, and its scene JSON has
  no markdown round-trip — an embedded foreign document, against Tine's core
  invariant.
- **No parsing/interpretation of the scene JSON.** The file is an opaque asset.
- **No OG `whiteboards/*.edn` handling here.** The block-arrangement half of
  whiteboard requests is the breadth-spec **canvas face**
  (`docs/breadth-grid-spec.md` §10 v3), and a one-way `.edn` importer is a
  rider there. Tine must simply continue to leave `whiteboards/` untouched.

## 2. Behavior

### 2.1 Rendering (verify-first)

`*.excalidraw.svg` and `*.excalidraw.png` referenced via standard image syntax
MUST render as ordinary images. This is *expected to already work* (they are
valid SVG/PNG; assets are served as blob URLs) — step one is to verify in the
harness, not to build. Watch for two gotchas:

- double-extension handling anywhere MIME type / "is image" is inferred from
  the file extension (the suffix is `.svg`/`.png`, so a *last*-extension check
  is correct and a *first*-extension check is a bug);
- SVGs render inside `<img>` (no script execution — fine for untrusted files;
  do NOT inline the SVG markup into the DOM, which would execute scripts and
  is the same class of risk as GH #16 raw-HTML).

### 2.2 "Edit in Excalidraw" affordance

- **Where:** the context menu of a rendered image asset (and of an asset
  attachment chip) whose filename matches
  `/\.excalidraw(\.(svg|png))?$/i`.
- **Label:** `Edit in Excalidraw`.
- **Action:** open the file externally via the existing external-open path
  (the env-scrubbed + detached spawn used for media — reuse it, don't fork a
  new spawn site). Default = system opener (`xdg-open`/OS equivalent).
- **Setting (small):** `Settings → Files → Excalidraw command` — optional
  custom command template (e.g. a desktop Excalidraw binary); empty = system
  opener. Persist via the Rust backend like other settings so native launch
  actions and independent WebViews share one atomic value.
- **Discoverability of the workflow itself:** the menu item appears only on
  matching filenames; docs/FEATURES.md documents the export-with-embed-scene
  convention (that's where users learn to produce the right files). Users
  whose system opener has no association get their browser → excalidraw.com,
  where the file can be opened/dropped — acceptable v1 floor.

### 2.3 Refresh on external change

After the user edits and re-exports (overwrites) the asset, the rendered image
must update:

- If the graph file-watcher does not already cover `assets/`, extend it (or
  add a cheap fallback: re-check asset mtimes on window focus — the user has
  necessarily alt-tabbed away to edit).
- Blob URLs cache the bytes at creation: on a detected change, recreate the
  blob / bump a version param so `<img>` actually reloads. Verify with a real
  overwrite, not just a cache-warm render.

### 2.4 Bare `.excalidraw` files

A bare `.excalidraw` (scene JSON, no image) has no preview. Render its
reference as the standard attachment chip (filename + icon), carrying the same
"Edit in Excalidraw" action. Docs steer users to the `.excalidraw.svg`
convention instead.

## 3. Verification (definition of done)

1. Harness screenshot: a page referencing a sample `sample.excalidraw.svg`
   (commit a small fixture) renders the drawing inline; same for `.png`.
2. Context menu shows "Edit in Excalidraw" on the fixture, and NOT on a plain
   `foo.svg`.
3. Real-app smoke test (asset IPC + external spawn are not exercisable in the
   mock backend): open action launches the opener; overwriting the file on
   disk refreshes the rendered image without a restart.
4. OG cross-check: the same `![…](….excalidraw.svg)` reference renders in OG
   too (it's just an image) — round-trip stays intact.
5. docs/FEATURES.md entry (the export-with-embed-scene workflow) in the same
   chunk.
