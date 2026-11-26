# Outside PDF reads (GH #732)

On Linux, Windows and macOS, Tine's own Markdown `![label](file:///absolute/path.pdf)`
renderer and Org `[[file:///absolute/path.pdf][label]]` renderer open the PDF in the
ordinary companion-pane reader. This follows OG 6e7afa8eb `block.cljs`
`show-link?` / `media-link`. Plain Markdown links keep the system opener. iOS and
Android keep their existing outside-link behavior and cannot call this reader.

A relative file link to a PDF opens the same reader from the same Markdown `!` or
Org position: `file:../x.pdf`, `../x.pdf`, `..\x.pdf`, `docs/x.pdf`, `./x.pdf` and
`file:x.pdf`. It resolves with the one file-link resolver (`relativeFileLink.ts`,
GH #457: from the graph root, authored `..` steps kept, lexical, no filesystem
probe), and the reader receives the resulting absolute `file://` URL
(`relativePdfFileUrl` in `externalPdfPath.ts`), so every rule below applies to it
unchanged. Spellings of one file share one reader tab. These keep their existing
opener: an `assets/` link and a bare `x.pdf` (the graph-asset reader, with
annotations), any path that resolves into the graph's `assets/` (so
`file:assets/x.pdf` keeps the file opener), a plain Markdown link, an absolute path
without `file://`, any non-PDF link, a link with `#`/`?` after the name, and a link
on a graph whose root is unknown. OG 6e7afa8eb resolves a relative PDF under the graph's `assets/` after
stripping leading dots (`resolve-asset-real-path-url`) and has no graph-root form;
Tine keeps its #457 graph-root rule, so `../x.pdf` names the file above the graph.

Rendering a link performs no file read, prefetch or thumbnail. Only clicking the
host renderer's link supplies a one-use, binding-scoped reader source for its new
PDF view. The source captures the URL and native binding at click time; a later
change to the rendered link cannot retarget it. The source is memory-only and
consumed when that view mounts. Restoring
a session cannot authorize a read; an unavailable source shows an explanation in
the normal PDF error surface asking the user to open the original link again.
Another view can share an already-open document without another native read.
Clicking the original link again refreshes an existing outside-PDF tab in place,
preserving its page and zoom and allowing a previously restored/error tab to retry.

The async native command captures the graph binding and performs filesystem work
on the blocking pool. It parses an absolute `file://` URL, canonicalizes exactly
that path, opens one regular file read-only, checks its metadata against the
existing 256 MiB viewer limit and checks the first five bytes for `%PDF-` before
reading the remainder. A bounded read enforces the limit again if the file grows.
It neither lists directories nor opens related files, writes files, copies into
`assets/`, nor widens the asset protocol's scope. Symlinks resolve to the one
canonical regular file. Windows drive and UNC URLs use the platform file-URL
conversion; percent escapes are decoded once. Remote file hosts are unsupported
on POSIX. Spaces, Unicode, literal percent/hash/question marks use URL encoding.

| Refusal | In-scope reason | Visible recovery |
| --- | --- | --- |
| Missing file / permission denied | External move, rename, permission change or disk failure | Reader error with the path; fix the link/access and reopen |
| Directory / FIFO / device / socket | Malformed or imported link must not hang or read a special file | Reader error with the path; select a PDF file |
| Missing `%PDF-` header | Malformed content or wrong file | Reader error with the path; select a PDF |
| More than 256 MiB, including growth while reading | Bound native and viewer memory | Reader error with the path; use the system viewer |
| No source after session restore or graph retirement | A saved route is not a user file-open authorization | Open the original link again |

Outside PDFs have no annotation identity. The viewer uses an inert annotation
state, never loads graph sidecars or the asset inventory, and disables text
highlighting, area capture (including modifier dragging), notes-page actions and
all annotation writes. The toolbar and overflow menu explain the read-only limit
through en/zh-CN catalog messages. Search, zoom, page navigation, outline and
ordinary native text selection/copy remain available. Reading state belongs to
the existing device session; opening, scrolling and zooming change no graph bytes.

Plugins receive no new capability or raw native access. Plugin decorations are
host-rendered text/badges; they cannot supply the link-open closure. Raw HTML is
sanitized and cannot attach event handlers or data attributes that authorize a
read. There is no delegated DOM selector that turns a `file:` link into a PDF
source. A plugin can propose graph text through its existing explicit edit
capability; only a later user click on Tine's rendered link opens that file.

Proof lives in `externalPdfPath.test.ts`, `externalPdfLink.test.tsx`,
`externalPdfLink.mobile.test.tsx`, `relativeFileLink.test.tsx`,
`PdfViewer.external.test.tsx`, `external_pdf_tests.rs`, `externalPdfBoundary.test.ts`, the unchanged
`plugins/capabilityBoundary.test.ts`, and `scripts/e2e-external-pdf.mjs`.
