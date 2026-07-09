# Track E - GH #61 PDF highlight links on Windows 11

Date: 2026-07-09

Scope: diagnosis on Linux plus candidate fixes for the Windows 11 report. I did not reproduce the Windows runtime behavior; every Windows-specific runtime statement below is a hypothesis that needs a Windows run.

Issue: https://github.com/martinkoutecky/tine/issues/61

## Shared PDF/asset path

- Inline links render through `src/render/inline.tsx:349`. Before this pass, image syntax always used `AssetImage` unless `mediaKind()` classified audio/video; `.pdf` returns no media kind, so `![](../assets/file.pdf)` became an `<img>` path and showed a broken image. This pass special-cases image-syntax PDFs at `src/render/inline.tsx:361` and renders the PDF link path instead.
- PDF filename extraction now normalizes backslashes and strips the graph `assets/` prefix in `src/render/inline.tsx:389` and `src/render/inline.tsx:537`. This matters because Rust intentionally accepts only an asset-relative filename for `read_asset`.
- Frontend `openPdf()` sets the target consumed by `src/App.tsx` and mounts `src/components/PdfViewer.tsx:33` with `filename`, `label`, and optional `page`.
- Backend `read_asset` is `src-tauri/src/commands.rs:440`, which delegates to `Graph::read_asset` at `crates/tine-core/src/model.rs:2647`. `Graph::read_asset` calls `top_level_asset_name(name)` before `assets_path().join(name)`, so absolute paths, traversal, and names containing path separators are rejected. That is correct security behavior; the frontend must send `file.pdf`, not `../assets/file.pdf` or `..\assets\file.pdf`.
- Backend `open_asset` is `src-tauri/src/commands.rs:584`. It joins the provided name under the graph assets directory, canonicalizes both sides, and rejects paths escaping assets at `src-tauri/src/commands.rs:592`. This is also correct but has the same frontend input requirement.
- Tine-generated hls pages use POSIX-style paths in `crates/tine-core/src/pdf.rs:303` through `crates/tine-core/src/pdf.rs:310`: `file:: [label](../assets/pdf)` and `file-path:: ../assets/pdf`. Highlight blocks carry `hl-page`, `hl-color`, `ls-type:: annotation`, and `id` in `crates/tine-core/src/pdf.rs:413` through `crates/tine-core/src/pdf.rs:424`.

## 1. PDF path rendered as a broken image

Exact code path:

- Parsed inline PDF image syntax enters `renderLink()` at `src/render/inline.tsx:349`.
- The old `s.image` branch rendered every non-audio/video destination with `AssetImage`, which uses blob image loading. A PDF blob in an `<img>` is not renderable and shows a broken-image icon.
- The asset path helper was POSIX-only: it searched for literal `assets/` and the PDF label/path code split only on `/`. A Windows-style `..\assets\paper.pdf` could therefore be passed downstream as a path-like name instead of `paper.pdf`.
- Rust `read_asset` would then reject that path-like name because `crates/tine-core/src/model.rs:2648` requires a top-level asset name.

Most likely root cause:

- Code-proven: PDF image syntax was routed to the image renderer.
- Windows hypothesis: Logseq-compatible imported links or hls metadata may preserve backslashes or case variants such as `..\assets\Foo.pdf`; previous frontend extraction would not normalize those before calling `openPdf()` / `read_asset()`.

Candidate fix:

- Implemented: `src/render/inline.tsx:361` detects `.pdf` before `AssetImage`; `src/render/inline.tsx:389` normalizes `\` to `/`; `src/render/inline.tsx:537` finds `assets/` case-insensitively after normalizing separators.
- Tests: `src/render/astRender.test.tsx:92` asserts image-syntax PDFs render as `pdf-link`, not `inline-image`; `src/render/astRender.test.tsx:99` asserts Windows-style backslash asset paths produce the same PDF link filename.

Windows check to confirm:

- On Windows 11, import or create a Logseq hls page containing both `![](..\assets\paper.pdf)` and `[paper](..\assets\paper.pdf)`.
- Confirm no broken-image icon is rendered for the PDF image syntax.
- Click both rendered PDF links and confirm the viewer opens the PDF. In devtools/logs, the frontend should call `read_asset` with `paper.pdf`, not `..\assets\paper.pdf`, `../assets/paper.pdf`, or an absolute drive-letter path.

## 2. Clicking a PDF-highlight block reference does not open/jump the PDF

Exact code path:

- Inline block refs render through `BlockRefView` in `src/render/inline.tsx:992`.
- Plain click handling at `src/render/inline.tsx:1025` through `src/render/inline.tsx:1039` resolves the target block and navigates to the source block/page (`focusBlock()` or `openPageAtBlock()`), while shift-click opens the source block in the sidebar.
- Annotation metadata detection exists in `src/editor/annotation.ts:15`: it recognizes `ls-type:: annotation`, reads `hl-color`, and parses `hl-page`.
- The only current PDF-open affordance for an annotation block is the rendered swatch in `src/components/AnnotationBody.tsx:16` through `src/components/AnnotationBody.tsx:20`; it looks up the owning hls page's file with `pdfFileForPage()` and calls `openPdf(file, file, hlPage)`.
- `pdfFileForPage()` at `src/editor/annotation.ts:26` currently reads `file-path::` with `/file-path::\s*(\S+)/` and then `split("/")`. That is fragile for filenames with spaces and for Windows-style backslashes.

Most likely root cause:

- Code-proven: generic block-ref click has no annotation-aware branch. Clicking `((highlight-id))` opens the referenced block/page, not the PDF.
- Possible cascade from problem 1: even when the user clicks the annotation swatch or a page-number ref path that eventually calls `openPdf()`, a Windows-style `file-path:: ..\assets\paper.pdf` can fail filename extraction.

Do they share a root?

- Problem 2 has an independent missing handler: `BlockRefView` does not treat annotation block refs as PDF links.
- It may also share the path-normalization root with problem 1 for the existing swatch/page-number open path.

Candidate fix, not implemented in this pass:

- Add an annotation-aware plain-click branch in `BlockRefView` after `grp()` resolves but before the default source-block navigation. If the referenced block has `ls-type:: annotation` and a valid `hl-page`, resolve the owning hls page's `file-path::` or `file:: [label](...)`, normalize separators and `assets/` prefix with the same helper used by PDF links, then call `openPdf(filename, label, hlPage)`.
- Preserve current modifiers: shift-click should continue opening the source highlight block in the sidebar; ctrl/meta should continue opening the route in the other pane.
- Replace `pdfFileForPage()` parsing with a shared helper that supports backslashes and spaces. It should parse the whole property value, trim it, handle `file:: [label](../assets/foo bar.pdf)` as well as `file-path:: ../assets/foo bar.pdf`, then return the top-level asset filename.

Windows check to confirm:

- On Windows 11, click a `((highlight-id))` block reference. Plain click should open the PDF and jump to `hl-page`.
- Shift-click the same ref; it should still open the source highlight block/sidebar path used in the reporter's repro sequence.
- Click the annotation swatch/title/page-number affordance from an hls page whose `file-path::` contains backslashes and a filename with spaces; it should still open `filename.pdf` at the page.

## 3. Runaway memory/CPU freeze after hls page page-number ref opens blank PDF viewer

Exact code path:

- `PdfViewer` loads highlights and bytes in `onMount()` at `src/components/PdfViewer.tsx:479`, then calls `pdfjs.getDocument({ data: bytes }).promise` at `src/components/PdfViewer.tsx:499`.
- Page layout is virtualized: `buildLayout()` at `src/components/PdfViewer.tsx:181` creates sized wrappers only, and `IntersectionObserver` starts rendering visible pages at `src/components/PdfViewer.tsx:223`.
- Raster allocation is guarded in `renderPage()` at `src/components/PdfViewer.tsx:237`: pages are rendered only when visible, DPR is capped at 2 at `src/components/PdfViewer.tsx:286`, and offscreen canvases are evicted past `CANVAS_CAP = 24` declared at `src/components/PdfViewer.tsx:93`.
- Before this pass, load/render failures in `readAsset`, `getDocument`, first-page read, render, or text extraction had no visible terminal failure state. A failed PDF open could leave a blank viewer and async errors instead of a bounded error UI.

Most likely root cause:

- I did not find a normal successful-load path that bypasses the existing virtualization/canvas cap.
- Top hypothesis needing Windows confirmation: the Windows path bug from problem 1 causes the viewer to request an invalid asset name or invalid bytes; the pre-fix viewer then enters a blank failed-load state. The observed memory climb is likely in the pdf.js/WebView failure/retry/render path triggered by that bad load, not in normal page virtualization.
- Alternative hypothesis needing Windows confirmation: a malformed page-number ref requests a valid PDF at a bad page target, causing repeated open/render attempts before the viewer reaches a stable page. Current `onMount()` clamps invalid/missing `props.page` to page 1 at `src/components/PdfViewer.tsx:524`, so this is less likely than a bad asset load.

Candidate fix:

- Implemented a fail-safe guard without changing virtualization/caps. `failPdf()` at `src/components/PdfViewer.tsx:158` sets a terminal load error, disconnects the observer, clears timers, cancels render tasks, and nulls `pdfDoc`.
- Implemented guarded failure exits for `readAsset`, empty bytes, `pdfjs.getDocument`, first-page read, page render, page text read, and text-content extraction at `src/components/PdfViewer.tsx:487` through `src/components/PdfViewer.tsx:515` and `src/components/PdfViewer.tsx:256` through `src/components/PdfViewer.tsx:405`.
- Implemented visible error UI at `src/components/PdfViewer.tsx:1102` and styles at `src/styles/app.css:6652`.
- Test: `src/components/PdfViewer.test.tsx:34` mocks pdf.js document rejection and asserts an error is displayed and no `.pdf-page` wrappers are created.

Windows check to confirm:

- Reproduce the reporter sequence on Windows 11 with the original graph: shift-click highlight ref, open the source highlight block, click its hls title, click a page-number ref.
- If the asset/path still fails, the viewer must show `Couldn't open this PDF: ...` and stop; memory should remain stable for at least 60 seconds and there should be no repeated `read_asset` / `getDocument` loop.
- With a valid PDF asset path, opening to a page-number ref should render visible pages only; memory should remain bounded while scrolling and the existing 24-canvas cap should still evict offscreen pages.

## Result of this pass

Implemented safe Linux-correct fixes for the broken-image PDF routing, Windows-style separator normalization in PDF asset links, and fail-safe PDF load/render errors. I did not implement the annotation-aware block-ref click handler because that needs a small behavior change and a Windows/manual OG-parity check; the candidate patch shape is documented above.

Verification run on Linux:

- `rtk ./node_modules/.bin/vitest run --config vitest.render.config.ts src/components/PdfViewer.test.tsx`
- `rtk npm run test:render`
- `rtk npm run build`
- `rtk npm test`

Top #1 -> #3 hypothesis: yes, problem 1 is the most likely root that cascades into problem 3 on Windows. A path that is rendered or opened as `..\assets\file.pdf` instead of normalized to `file.pdf` would be rejected by the backend asset guard, and the pre-fix viewer did not fail safely. Problem 2 partly shares that path root, but also has an independent missing annotation-aware block-ref click handler.
