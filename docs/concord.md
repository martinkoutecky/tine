# Concord — Tine alongside your other tools

**Concord** is Tine's contract for living in a graph that other software also
touches — an external editor, Syncthing, Dropbox, git, Fossil, a USB stick.
Tine does not bring its own sync; you bring any transport you like, and Concord
is Tine's side of the bargain: external disk changes appear in Tine as fast as
in a code editor; nothing either side wrote is ever silently lost; whatever
genuinely conflicts becomes a calm, block-level review resolved in place inside
the page — and the files on disk stay plain, valid Logseq Markdown throughout,
with zero invented markers or metadata. Concord is being delivered in phases;
this document describes what is implemented today and grows with each phase.

## Freshness

### Clean pages reload automatically

When a file changes on disk and its page has no unsaved edits in Tine, the
loaded page is replaced with the disk version silently — no dialog, no page
flicker, no action needed. The file watcher notices the change (default on
desktop and Android: the platform's native inotify-backed watcher; a 3-second
polling scan remains available for filesystems where native events are
unreliable — see the `watch_mode` setting), re-reads just the changed files,
and refreshes any pane that shows them. Tine's own saves are recognized and do
not bounce back as external changes.

A page with **unsaved edits** is never clobbered by a disk change. Tine proves
per page that the file really diverged from what your editor started from; a
real divergence becomes an in-page Concord review where you choose per block
which side wins (or keep both), and both sides remain recoverable.

### While you are editing: deferred, not dropped

If a file changes on disk while you are *actively editing* that very page —
caret in a block, a block move mid-drag, a page rename in progress — Tine does
not yank the page out from under you. The reload is **deferred**: Tine records
it and replays it automatically the moment the blocking state clears (you click
away, the move settles, the rename finishes). At replay time the safety checks
run again from scratch — if you typed into the page in the meantime, the reload
is not applied; the normal conflict protocol takes over so your keystrokes are
never silently overwritten.

### When your filesystem tells us nothing: returning to the window

Some setups deliver no filesystem event at all — a network mount, a sync client
that writes through a path the kernel does not report, an app the operating
system suspended while you were elsewhere. Tine therefore also checks **when you
come back to the window**: returning to Tine replays any reload that was deferred
while you were editing, and asks the watcher for one full pass over the graph's
text files. Anything that changed is then handled exactly as a live change would
be, with the same protections — a page you are editing is still deferred, never
yanked. Before new input is admitted, Tine waits for the native scan receipt,
its frontend applications, and a bounded final check of pages that are visible,
edited, dirty, or otherwise active. This never scans the graph a second time.
The check is throttled, so alt-tabbing repeatedly costs nothing.

### Being asked instead of shown

By default a clean page updates silently, the way a code editor does. If you
would rather be told, turn on **Settings → Backups & recovery → Always ask
before applying an external change**. The page then keeps showing the version
you were reading and offers **Reload from disk** / **Keep mine** in a small bar
above the content — never a dialog, never blocking.

The switch only affects the case that was silent. A page with unsaved edits, or
one you are actively editing, behaves identically whether the switch is on or
off: it is proven, deferred or refused exactly as described above. Choosing
*Keep mine* writes nothing; the next time you save that page, Tine notices the
file moved on and opens the in-page Concord review.

### Your version-control tool's own churn is ignored

A repository living inside your graph folder generates a great deal of
filesystem noise that has nothing to do with your notes — index locks, object
files, ref updates, a `git gc`. Tine ignores events under `.git`, `.hg`, `.svn`,
`.jj`, `.bzr`, Syncthing's `.stfolder`/`.stversions`, and `node_modules`
entirely: they cannot contain graph text, so they never wake the watcher and
never cost a rescan. Everything else, including `logseq/config.edn` and pages in
custom folders, is watched as before.

### Reporting slow or missed external changes

An external change should show up in Tine well under a second with the OS
watcher (a few seconds in `poll` mode). If you see multi-second delays or
changes that never arrive, Tine keeps **latency receipts** you can attach to a
bug report — always on, with no setup:

1. Reproduce the slow external change once or twice.
2. Open the devtools console (right-click → *Inspect Element* → *Console*).
3. Run:

   ```js
   await __tineWatcherLatency()
   ```

4. Copy the output into your report.

The result is the last 64 external-change batches, oldest first. For each
batch: which graph, the watch mode (`inotify`/`poll`), how many pages changed,
whether a full rescan was forced, how many reconcile errors occurred, and the
per-stage timings — `event_to_reconcile_ms` (OS callback to the start of
re-reading, including the coalescing window), `reconcile_ms` (re-read, parse,
and notify the UI), and `event_to_emit_ms` (total, callback to UI). The same
receipt is also written as a `watcher-latency` line to Tine's log output; if
you run with `TINE_DEBUG=1` (see README → Troubleshooting) it is captured in
`tine-debug.log`, which you can send instead. Receipts contain file counts and
timings only — never note content.

## External revisions (bulk changes)

A `git checkout`, `fossil update`, branch switch, or a first big sync can
replace dozens or hundreds of files at once under a running Tine. Small
changes keep the per-file behavior described above, unchanged. When one burst
touches **more than 32 pages**, Tine treats it as a single *external
revision* instead of hundreds of independent edits:

- The watcher reconciles the whole burst in one pass against a consistent
  snapshot of the graph, rather than file by file.
- The interface is told once, not once per page: visible pages refresh
  immediately through the same safety checks as any external change (a page
  you are editing at that moment is deferred, exactly as above — never
  yanked); everything else reloads lazily the next time you open it, which
  always reads the fresh state.
- You see one calm summary line — a toast like **"128 pages updated
  externally"**, with a conflict count appended (e.g. **"· 2 conflicts to
  review"**) if any of the changed pages had unsaved edits that genuinely
  diverged. No dialogs, nothing to click through.

Pages with unsaved edits keep every guarantee from the Freshness section:
divergence is proven per page, the in-page review appears only after the
backend's own guarded refusal, and both sides stay recoverable.

## Transport artifacts and VCS markers

Tine does not sync your Direct Files graph itself — you bring the transport
(Syncthing, Dropbox, Seafile, git, Fossil, a USB stick). When a transport
cannot merge two versions of a file it leaves an artifact behind, in its own
format. Tine recognizes those artifacts, keeps them from polluting your page
list, and never rewrites or renames them.

### Conflict copies (sync tools)

When the same page was edited on two devices, file-sync tools keep one version
under the page's name and write the other as a renamed *conflict copy*. Tine
recognizes the exact naming shapes these tools generate:

| Tool | Generated name |
| --- | --- |
| Syncthing | `Page.sync-conflict-YYYYMMDD-HHMMSS-DEVICEID.md` (the device id is up to 7 characters `A–Z`/`2–7`; very old Syncthing versions omitted it) |
| Dropbox | `Page (conflicted copy …).md` / `Page (Alice's conflicted copy …).md` |
| Seafile | `Page (SFConflict user@example.com 2026-08-01-10-00-00).md` |

What happens to a recognized conflict copy:

- It is **not indexed as a page** — it never appears in the page list, search,
  or autocomplete, so it cannot duplicate the real page (or hijack the page's
  identity through a `title::` property).
- It is surfaced in **Settings → Backups & recovery → Sync conflict copies**,
  where you can review a block-by-block diff against the current page and
  merge either side (or both) per block, or discard the copy (recoverable from
  trash).
- Tine never renames, rewrites, or deletes a conflict copy on its own.

Only the exact generated shapes above are treated as conflict copies. A real
page whose name merely resembles one — say `Foo.sync-conflict-notes.md` —
stays an ordinary page.

Deliberately **not** recognized, because their names are indistinguishable
from ordinary page names:

- **OneDrive** conflict copies (`Page-COMPUTERNAME.md`): no sentinel word, no
  timestamp — any page name with a dash suffix would false-positive and be
  hidden from your graph.
- **Google Drive** duplicates (`Page (1).md`): the same suffix Drive uses for
  ordinary duplicate uploads.

Copies from these tools appear as ordinary (duplicate) pages; merge them by
hand.

### VCS merge conflict markers (git, Fossil)

When a `git merge`/`git pull` (or Fossil update/merge) cannot reconcile two
versions of a file, the VCS writes both versions *into the file*, separated by
column-0 marker lines — `<<<<<<<`, `|||||||`, `=======`, `>>>>>>>` (git), or
Fossil's verbose `<<<<<<< BEGIN MERGE CONFLICT …` / `####### SUGGESTED
CONFLICT RESOLUTION …` / `>>>>>>> END MERGE CONFLICT …` lines. The VCS then
relies on finding those exact markers, at column 0, to know the conflict is
still unresolved.

An outliner that re-saves such a file destroys that: the markers are not
bullet lines, so a normal save re-indents them as continuation text (or drops
them), git stops recognizing the conflict, and one side of the merge can be
lost without anyone choosing it.

Tine therefore **quarantines** marker-bearing files:

- The page **stays readable** — Tine renders the file as-is, markers and all,
  with a banner explaining the state.
- **Every save to it is refused** (including force-save) with a message naming
  the markers found. Tine never rewrites a file that carries unresolved merge
  markers.
- Affected files are listed in **Settings → Backups & recovery → VCS merge
  conflicts**.
- You can resolve it **inside Tine**, block by block, on the page itself (see
  *Resolving conflicts* below) — or where it belongs in your VCS or an external
  editor. Either way, as soon as the markers are gone from disk the page becomes
  editable again automatically; nothing needs to be reset in Tine.

A page that merely *documents* merge conflicts is not quarantined: markers
quoted inside code fences (or indented inside a bullet) don't count, and a
lone `=======` divider line never triggers the quarantine on its own.

## Resolving conflicts

### The conflict queue

Everything that needs your judgement — a retained live draft whose file changed,
a sync tool's conflict copy, or a page carrying VCS merge markers — appears in
**one queue**, shown as a quiet `N conflicts` badge at the bottom of the sidebar.
Clicking it walks you to the next conflicted page.

Disk artifacts are derived afresh on every scan. A live draft has no disk
artifact to derive from, so Tine preserves its exact draft, base, and reviewed
disk revision in app-private recovery state until resolution. Neither kind writes
markers or metadata into your graph. Deleting Tine's app data cannot lose a
conflict-copy or VCS-marker artifact, but it can discard an unresolved live draft;
resolve those before clearing app data.

It is deliberately calm. A conflict is a thing waiting for you, not an
interruption: no modal opens and nothing is blocked. A newly delivered sync
copy raises one persistent, actionable notice and joins the badge; if its page
is already open, the in-page resolver appears there. Leaving a page with
conflicts outstanding gets a one-line note, never a dialog you must answer.

### Resolving on the page

Open a conflicted page and the two (or three) versions are shown **at the
page**, above the outline, block by block:

- The two sides are named by whatever produced them — a git ref like `HEAD` and
  the incoming branch, or the Syncthing device/timestamp tag — and coloured
  consistently in the legend, the columns, and the buttons.
- Each differing block gets its own choice: keep one side, keep the other,
  **keep both** — or, when both sides edited provably different parts of the
  block's text, accept a **merged** version that combines the two edits.
- `N conflicts` with ↑ / ↓ walks between the blocks that need a decision.

The review lives at the top of the page, but it cannot scroll out of sight:
once it leaves the view, a slim one-line notice stays pinned to the top of
the pane — ignorable, never invisible. Tapping it unrolls the same review in
place, without losing your reading position or any choices already made;
tapping again (or Escape, or scrolling back up) folds it away.

**A suggested resolution is pre-selected.** Where a common ancestor is known
(the base ledger, or the ancestor a `diff3`/Fossil marker block carries with it),
a block only one side changed arrives with that side already chosen and labeled
*suggested* — so the normal gesture is glance-and-confirm rather than
hunt-and-compare. **Apply all suggested** re-applies that opening position after
you have experimented.

A block **both** sides changed can still arrive with a suggestion: when the
two edits touch provably different parts of the text against the known
ancestor — one device fixed the start of a sentence while the other appended
to its end — Tine offers the combined text as a pre-selected **Merged** row.
It is a suggestion like any other: shown, labeled, and applied only when you
confirm. Edits that overlap (or whose combination Tine cannot vouch for) get
no merged offer.

Where the edits overlap but the merge tool wrote its own `####### SUGGESTED
CONFLICT RESOLUTION` sections (Fossil), that text is offered in the same
Merged slot instead, labeled **Merged (tool)**. Because it is the tool's text
rather than something Tine computed, **Apply all suggested** leaves such rows
at your current choice — the pre-selection stands until you touch it, but
only your per-row click (or the initial confirmation) accepts the tool's
wording.

Where no ancestor answers the question, or the edits genuinely collide, the
pre-selection is **keep both**, which loses nothing. Keeping both writes the two versions as **adjacent sibling blocks**:
ordinary outline Markdown that every other tool can read. Tine never invents a
marker or a property to record that a block was contested.

Nothing is applied until you click **Apply resolution**.

### What resolving does

- For a **conflict copy**: the merged result is written to the page, the copy
  moves to the recoverable trash (Settings → Backups & recovery), and the exact
  committed page replaces the reviewed editor before typing is admitted again.
  If that editor still has an edit or save in flight, Tine first lands it and
  refreshes the comparison; it never applies decisions made against the older
  winner or lets the stale editor save over the merge.
- For a **marker-bearing page**: the merged result is written *without any
  markers* — the file becomes ordinary Markdown again and the save quarantine
  lifts by itself, because there is no longer anything to quarantine. This is
  the only circumstance in which Tine ever rewrites a file carrying merge
  markers, and only as the direct result of the resolution you just confirmed.
  A byte-exact copy of the pre-resolution file (markers and all) goes to the
  recoverable trash first, so the sides you did not choose stay recoverable
  in Settings → Backups & recovery.
- For a **retained live draft**: the merged result is revision-guarded against
  the exact disk version shown in the review and written through the ordinary
  Direct Files writer. A newer unseen disk edit refuses the apply and refreshes
  the review. After success, Tine installs the exact page it wrote and removes
  the app-private capsule.

Both paths run through the same guarded write Tine uses everywhere: the page is
locked, the file must still be byte-for-byte what you reviewed (if your VCS or
sync tool moved it in the meantime, the write is refused and the review reloads),
`.org` pages that would not survive a round trip are refused rather than risked,
and the losing side stays recoverable.

### Where each surface lives

Resolution happens **only on the page**. Live draft conflicts appear there
automatically and in the sidebar queue. **Settings → Backups & recovery** is the
*inventory* for disk artifacts: it lists the conflict copies and marker-bearing
files in your graph, offers **Review in page…** to take you to the one you pick,
and keeps the two actions the page cannot offer — **Discard copy**, and the case
of a copy whose original page no longer exists at all.

(Earlier versions also had a block-by-block merge dialog inside Settings. It is
gone: two surfaces over the same conflict opened with different pre-selections,
which is exactly the kind of quiet disagreement Concord exists to prevent.)

## Tine does not touch files it has nothing to say about

Tine never rewrites bytes it did not semantically change. Opening a graph,
reading a page, or re-saving a page you did not edit leaves every file
byte-identical — same indentation (tabs or spaces, as you wrote them), same
trailing newline (or absence of one), same line endings, same blank lines. This
matters most if you keep your graph in git: a spurious rewrite is a diff you did
not make, and behind a sync tool it is a wake for every device.

The same rule applies to files Tine writes for you. Re-saving a PDF's
annotation page (`hls__…`) with an unchanged highlight set writes nothing at
all, and adding a highlight leaves the rest of that page's formatting — including
notes you typed under an annotation — exactly as it was.

One deliberate exception, unchanged: navigating to a block by reference stamps
an `id::` property on it, because that is how a durable block reference is
expressed in the Logseq file format.

### Journal files named by title

A journal file whose name is not its date — `Jun 18th, 2026.md` rather than
`2026_06_18.md`, usually left behind by a date-format change or another tool —
cannot be matched back to its day, so that day looks empty in the journal feed.
Renaming it fixes that, and Tine will do it, but **only when you ask**: the files
are listed under **Settings → Backups & recovery → Journal files named by
title** with one button to rename them all. A snapshot is taken first, so the
original names remain in Backups & recovery, and a file whose date name is
already taken is left alone rather than overwritten.

(Earlier versions performed this rename automatically at every graph open. It is
a repair you did not ask for, applied to files you own — your version-control
tool would see it as a batch of renames the moment Tine started.)

## The base ledger

Tine remembers, for every page, **the last version it agreed on with the
disk** — the text it last read from or wrote to the file. That remembered
version is the common ancestor of any later divergence, which is what turns a
conflict from a guessing game into a mostly-answered question: comparing each
side against the ancestor tells Tine *who changed what*.

When you review a conflict, Tine uses that ancestor when it has one:

- A block only **you** changed arrives with *your* version pre-selected.
- A block only the **other device** changed arrives with *its* version
  pre-selected.
- A block **both** sides changed is a real conflict. If the two edits touch
  provably different parts of the text, a combined **Merged** version is
  offered pre-selected; otherwise there is no pre-selection and you decide.

Pre-selected rows are labeled *suggested*, and the toolbar says when
suggestions are in play. Nothing is ever merged automatically — you review the
rows and click merge, exactly as before; the suggestions just mean the common
case is glance-and-confirm instead of hunt-and-compare.

Practical notes:

- The ledger lives in Tine's app data folder (`concord-ledger/`), **outside
  your graph** — your sync tool never sees it, and it never touches your
  files.
- **Deleting it is always safe.** Tine falls back to the plain two-column diff
  (no suggestions) until the ledger repopulates through normal editing.
- It fills in as you work: pages saved or reloaded since the feature arrived
  have a remembered version; untouched pages simply have none yet.
## Unavailable physical pages and refused close

GH #540/#541: the close warning names dirty, saving, and conflicted page owners.
Declining discard opens the current graph's recovery inventory. Retry uses the
ordinary flush path; copying is read-only and never acknowledges a save. The
complete recovery copy serializes the retained PageDto without dropping fields.
The inventory is transient, retired on graph rebound, and separate current and
recovered drafts remain separate when they differ.

A pinned page-load error does not hide an exact-path live capsule. It exposes
the retained draft and existing conflict resolver. An unloaded capsule may be
resolved without inventing an editable empty page; same-name other-file owners
are refused. Native durable review represents a missing current file explicitly
and resolution rechecks that absence before no-replace publication. No graph
file is created merely by viewing recovery.

The review API uses `conflict_rev = "absent"` for a missing disk side, distinct
from the content hash of an existing empty file. Present files retain their
snapshot identity through publication, so a late replacement or byte change
refuses the stale merge. This changes no persisted capsule format.
