# 0078 — Git graphs: Tine autocommits, and syncs fast-forward only through the page host

- **Status:** Accepted for autocommit (slice 1), rev 1 (data-safety review folded in: `tine-agents/subagent-tasks/notes/review-adr-0078-git.md`; design accepted by Martin 2026-10-18/19: D-20261018-G1…G7, D-20261019-G8/G9; the autocommit unit cost accepted by Martin 2026-10-26, D-20261026-GC).
  Git sync (slice 2) is in 0.9 (D-20261028-SG); §4 rev 2 follows storage model s3.7 (repairs P1–P4; the residual is a stated limit, D-20261028-GU). Its push cost and credentials (open questions 1 and 2) are undecided.
- **Date:** 2026-10-21

## Prior decisions

Sources: the decision journal, vision 9(b), the roadmap, `SETTLED-DECISIONS.md`, the git-sync campaign
DESIGN (rev 0 + §8–§9) and the seen design (rev 1). Dossier with line numbers:
`tine-agents/subagent-tasks/notes/git-autosync-prior-decisions.md`.

| Record | Says | Here |
| --- | --- | --- |
| D-20261004-V9, vision 9(b) | opt-in git sync, off by default, all platforms, built-in library; phase 1 fast-forward only with a "diverged" stop, phase 2 Concord page-by-page merge; pulled files reach disk only through the guarded save path; login is an HTTPS token in the OS keychain | follow, except the keychain: open question 2 |
| BACKLOG #33 (OM5) | Tine never requires or creates a repository; git features only on graphs that already are repositories; Syncthing stays supported | follow |
| 10/06 CLI attribution, CZ4, CL1–CL6 | the CLI commits each run; a run first snapshots the user's pending edits under the user's name; revert is Concord's, never `git revert`; `run end` squashes only an unpushed, unmoved run | follow; autocommit and run commits share one lock (§3) |
| D-20261018-G1 | sync policy per repository, automatic or on demand; any git remote | follow |
| D-20261018-G2 | Tine itself autocommits on git graphs | follow |
| D-20261018-G3 | generic messages; a structural operation commits at its boundaries with its verb as message; no op log, no diff-derived messages | follow |
| D-20261018-G4 | `author::` stays | untouched |
| D-20261018-G5 | an autocommit includes every change git sees under the graph root, honouring `.gitignore`; Tine applies pulls itself through the audited writer; sync off under eol/filter conversion; Tine never writes `.gitignore` unasked | follow, with the scoped D-20261026-GI exception in §2 |
| D-20261029-GT | like `.syncthing.*` (D-20261026-GI): add `logseq/.tine-*` to `.gitignore` once with the same toast and Settings line, untrack already-committed copies (they stay on disk), a pull ignores those paths; rejected: excluding them only in staging | follow (§2, §4) |
| D-20261028-CRLF | offer a repository-local autocrlf repair only when raw tracked bytes match the index and no other conversion blocks sync; otherwise commit first, then re-checkout | follow; pull keeps refusing conversion |
| D-20261018-G6 | lands in whatever release the timing gives; no release waits for it | follow (card hBpb2k's "after model v0" is stale) |
| D-20261018-G7 | libgit2 (git2-rs, `default-features = false`) on all five platforms; HTTPS through Tine's own Rust HTTP/TLS stack as a registered smart transport; no OpenSSL | follow |
| D-20261019-G8 | Android TLS through the OS trust store (rustls-platform-verifier); Tine owns repack; batched cadence; libgit2 fsync on; SSH later, pure Rust, only when asked (supersedes G7's libssh2) | follow |
| D-20261019-G9 | autocommit when no page saved for 60 s and ≥ 10 min (a setting) since the last one; at once on background/quit, before a pull, and around structural operations; repack at 50 loose commits on the git thread, skipped while a `git gc` lock exists | follow, except background on Android/iOS: capped at one autocommit per interval (D-20261026-GC, manager 2026-10-28; deviation to confirm with Martin) |
| D-20261017-SB2, LD1, C13 | seen reads git; only commits by others count; "me" = the repository's `user.email`, "Your name" as fallback | follow: autocommits use exactly that identity |
| D-17, D-10, I-25, trust model | Direct Files only; a new mode is specified with budgets and approved before code; every refusal names its in-scope threat | follow: this record, the refusal table, the unit-cost line |

## Context

On a graph that is a git work tree, users want history and multi-device sync without a terminal (vision 9(b),
Brikas' disc 290). Tine already writes `.git` for command-line runs (`src-tauri/src/owner/git_run.rs`, CL4:
not a storage mode): commits built from exact bytes, the branch moved by compare-and-swap, the index entry
updated under git's own lock protocol. Autosync differs in two ways that make it a storage mode under D-17:
it commits files Tine did not write, and a pull changes the graph's files.

The one-way door is the pull. `git checkout`/`merge` would write the working tree outside the audited path:
no base-revision guard against an open editor buffer, no page lock, no draft. Everything else (cadence,
messages, the settings shape) can change later without touching anyone's data.

## Decision

### 1. Detection and policy
A graph whose root is a git work tree root, or lies inside one, offers in Settings → Graph → Git: **Off**
(default), **Commit automatically**, **Commit and sync on demand**, **Commit and sync automatically**. The
choice, and the G9 interval, are device settings in the existing `tine-settings.json`, keyed by graph path; the
backend reads them when the graph binds and applies a change from Settings to the running worker at once. Nothing
is written to the repository or its config. Refused with a reason in Settings: bare repository,
merge/rebase/cherry-pick in progress, detached HEAD (no branch to advance), content conversion of graph files
(`.gitattributes` `filter`, `ident`, `working-tree-encoding`: the committed blob would not be what git expects to
check out; eol normalisation (`core.autocrlf`, `text`/`eol`) refuses only sync, per G5, because a commit stores
Tine's exact bytes and a later normalising checkout changes only line endings), sparse checkout (`core.sparseCheckout` or any
skip-worktree entry under the graph root: libgit2's add would commit the excluded files as deleted), and on macOS
and iOS a repository without `core.precomposeunicode = true` (decomposed names would commit every diacritic page as
a delete plus an add). A held `index.lock` is "busy, retry"; after three busy cycles spanning 30 minutes Settings
names the file and says to remove it if no git command is running (a crash can leave it; the exception below
allows an explicit click to remove a qualifying `index.lock`).
Slice 3: the lock named is the one actually held, `index.lock` or a ref lock a crashed commit left on `HEAD`, the
branch or `packed-refs` (`held_lock`; before, Settings always named `index.lock`, even when a branch lock was the cause).
Slice 3, D-20261029-GSL (Martin approved 2026-10-29): when a commit or sync finds `index.lock` held,
Tine offers **Remove the stale lock** in
the app's notice (under every committing policy) and in Settings if (a) Tine runs no git operation itself
(`RUN_LOCK` is free in this process; every Tine path that takes `index.lock` holds it), (b) the lock is a regular, empty file
(Tine's own lock is always empty; a git program writes the new index into its lock, and `git commit -a` holds such
a written lock while its editor is open), and (c) its modification time is at least 10 minutes old and not in
the future. The click passes back the offered identity (size, modification and creation times); under `RUN_LOCK`
Tine rechecks (a)–(c) and the identity, renames the lock aside, checks the renamed file is still the same one
(otherwise links it back), deletes it, logs `git-stale-lock-removed`, and the worker commits (and syncs) at once.
Tine never removes a lock unasked, and never a ref, `HEAD`, `packed-refs` or `shallow` lock
(`src-tauri/src/owner/git_stale_lock.rs`).
The offer also applies the click's guard, clean repository state and attached-HEAD checks, so a merge or
detached HEAD does not offer an action that must refuse.

Known limit: a process the OS froze or suspended while it held an empty lock (Android freezer, iOS suspension,
e.g. Tine beside Tine Beta, or a git app paused mid-checkout) is indistinguishable by age. Each Tine process has
its own `RUN_LOCK`. To limit the cascade, `IndexLock` records the created handle's identity (Unix dev+ino;
Windows volume serial + file ID) and refuses to install or unlink after observing a missing or replaced lock.
This protects a resumed Tine from overwriting the index or deleting its successor's lock; it cannot protect
a frozen third-party git client without such a check. That remains a gap in the approval's "never for a lock
another git program holds" requirement. Identity errors or zero IDs fail closed without a timestamp fallback;
unstable FUSE/NFS identities can refuse an otherwise safe commit. Native check-to-mutation races remain.
Network filesystem/peer clock skew can also defeat the age test; filesystems without hard links preserve a
raced replacement at the named aside path for recovery (see the storage contract's refusal/limit table).
Sync additionally needs the graph root to be the work tree root: a fast-forward of a bigger repository changes
files outside the graph, which Tine does not own.

### 2. What an autocommit is
Follow-up D-20261026-GI: with autocommit enabled, at binding start and when enabling it,
Tine checks git's ignore rules for a representative `.syncthing.*` name. If not ignored,
it appends `.syncthing.*` to **the graph root's** `.gitignore`, including for a graph nested
inside a larger repository. Existing bytes and line endings survive; the new line ends
with a newline. Follow-up D-20261029-GT adds a second line the same way, `logseq/.tine-*`
(Tine's own per-device files: the trash), counted as present only when git ignores both a
representative `logseq/.tine-ignore-check` and a real trash file (a file below an ignored
folder counts as ignored, as in git; so a negation such as `!logseq/.tine-trash/**` still
gets the line, which comes last and wins; staging excludes these files regardless), and
remembered under its own key
`git-tine-own-ignore-checked`; lines due together go into one write and one notice, and a
device that already had the Syncthing attempt gets only the new line, once. The line is
graph-relative, so in the graph root's `.gitignore` it matches only the graph's own
`logseq/` folder, also for a graph nested in a larger repository. These two lines are the
only exception to autocommit never writing graph files: it
uses Store's guarded non-page transaction (create-if-absent or revision-checked replace,
lock, temp, fsync, rename). Store accepts only the fixed root `.gitignore` identity in
addition to its existing file areas; this does not widen graph discovery or backups.

The existing device-local graph notices record remembers `git-syncthing-ignore-checked`
**before** the attempt, even if the rule was already covered or the write subsequently
fails. One attempt per device and canonical graph path is the honest rule: removing the
line, disabling/re-enabling, rebinding or restarting never causes another append. A crash
after recording the attempt may leave no append; a failed write is not automatically
retried. Without a durable marker Tine does not write the graph. Settings' existing git
status surface reports the added file and rule (and that it may be removed), or a setup
failure, for the current binding. Clearing device-local app data or moving the graph
resets this memory, just like other graph notices.

Staging independently removes `.syncthing.*` and `logseq/.tine-*` paths under the graph
root from its private index copy, including pre-staged and previously tracked ones, and
skips matching working files. `logseq/.tine-*` files an older Tine committed leave `HEAD`
in a commit of their own (`Tine: stop tracking its own device files (logseq/.tine-*); they
stay on disk`), made just before the next commit with a real change and never alone: two
devices untracking the same files would otherwise diverge over that commit alone. A pull
compares `H` and `U` without `logseq/.tine-*` (§4): it neither writes those paths nor
refuses over them (before, a trash copy in `U` was `pull-refused`, and a committed trash
copy made the next pull `diverged`: REG-SB1-GIT-TINE-OWN-TRASH). The pull still sets the
index's `logseq/.tine-*` entries to exactly `U`'s, before the index install that precedes
the ref move, so publishing `U` leaves nothing staged (an ordinary `git commit` records
nothing of them); their files are not written, so a terminal `git status` may list them as
modified or deleted until the next real autocommit untracks them. The namespace is literal:
any file named `logseq/.tine-*` counts as Tine's own, whoever wrote it. Tine's byte-exact
`logseq/.tine-` prefix governs staging, untracking and the pull; on a case-insensitive
repository (`core.ignorecase`) git's matching of the `.gitignore` line is wider (it also
matches `logseq/.TINE-x`), which, like any ignore rule, only keeps such an untracked file
from being added: a tracked one stays tracked and is pulled as ordinary content. This keeps temps out even when ignore setup fails or a narrower rule leaves some
directories visible. It never deletes their working files. Conflict copies
(`*.sync-conflict-*`) remain user data and are committed unless the user's rules ignore
them. No new per-edit file or transport record: setup adds 13 bytes (`.syncthing.*\n`)
and 15 bytes (`logseq/.tine-*\n`), one more each with CRLF, plus a missing separator
newline, and one device-local notice key per line once; the untrack is one commit per
graph (a nested graph cleans up separately) each time tracked own files appear, normally
once, again only if an older peer re-adds its own files;
one-block and 60-block page edits both add zero bytes/files/transport beyond §Unit cost.

Every change git sees under the graph root, honouring `.gitignore`: Tine's saves, other programs' edits,
deletions. The graph root is matched as a literal path prefix in the staging callback, never as a pathspec pattern
(a folder named `Notes [work]` is a glob). Skipped: Tine's own in-flight temp files (`atomic_file::temp_path`'s
shape) and untracked nested repositories (never added as gitlinks). Like `git commit -a`, an intermediate staged
version of a file is replaced by the working tree's. A file the user untracked (`git rm --cached`) but did not
ignore is still a change git sees, so it is committed again: `.gitignore` is how a file stays out (review F2;
honouring the untrack would only delete it once). A file whose `.gitattributes` sets `filter`, `ident` or
`working-tree-encoding` refuses the whole commit, naming it (git-lfs, git-crypt: its raw bytes must never be
committed); every staged file is checked, not only sample paths (review B2). A missing index while `HEAD` has
commits refuses (`run git reset`): staging into an empty index would record every file as deleted (review B1); an
index with unresolved conflicts refuses as such. It is built on a private copy of the index taken while Tine holds
`index.lock` (the protocol `git_run.rs` already uses): stage the graph root's changes into the copy, write its
tree, take only the graph root's subtree into `HEAD`'s tree (a user's staged changes outside the graph are never
committed), commit, install the copy as the index (fsync the copy, rename it over the index, sync `.git`), then
move the branch by compare-and-swap, all under the lock. Index first (s3.7 repairs P1/P1b, `SPEC-s37-gitpull.md`):
the ref moved first, a crash between the two, or a power cut that undid the unsynced index rename (libgit2 fsyncs
the ref and its folder), left the old index under the new `HEAD`, and the user's next plain `git commit` recorded a
reversion. An index left ahead of an unmoved `HEAD` (crash, lost compare-and-swap) only stages what the next
commit records; an index that cannot be installed refuses the commit. Run commits use the same helper. Windows
performs identity checks without directory flush for this `.git` sync (D-20261028-WDD keeps the best-effort flush
to app-data folders; a flush on a cloud or network folder could hang under `index.lock`), so there the index
rename's survival of a power cut rests on the file system, with no witness. Author and
committer: the repository's identity, else "Your name" with `user@tine.invalid` (`git_run.rs`'s fallback, which
seen's "me" matches; per field, `git_run::user_identity`: repository `user.name` else "Your name" else `Tine user`,
repository `user.email` else `user@tine.invalid`). Message `Tine: autosave`; no trailers (G3: generic). Nothing to commit → no commit.

The conversion check is shared with CLI run commits and applies to each exact repository-relative path,
including nested graph roots. Runs refuse before flushing or saving an affected page and check again before
creating raw Git objects; neither writer executes external filters (L7s5-01). Both writers use the same
branch-bound index-lock helper: once locked, `HEAD` must still name the captured branch before any ref move
or index change. A checkout mismatch refuses explicitly, with no retry on the new branch; a run's saved
write is reported uncommitted (L7s5-02). Concurrent commits on the same branch retain compare-and-swap retries.

### 3. When
Per G9: when no page has saved for 60 s and the interval (default 10 min) has passed since the last autocommit;
at once when the last window closes or the app goes to the background (on Android and iOS at most once per interval,
slice 3, D-20261026-GC), before every pull, and around a
structural operation (rename, merge, delete or move of a page, restore from backup): commit pending changes
first, run the operation, then commit its files with its verb (`Rename page "A" to "B"`). A failed operation
leaves only the first commit. Autocommits take the CLI runs' lock, so they never fall between a run write's
snapshot and its agent commit; a run whose tip an autocommit moved is not squashed (CL1 already refuses a moved
tip), and the reply says so. Pulls and structural brackets hold the same lock from their first commit to their
last ref move. `tine-index-*.tmp` copies a crash left in `.git` are removed by the next holder of `index.lock`,
never otherwise (another holder's copy is live). On quit, the flush waits for pending commits until shortly
before its budget ends; then a commit still in flight aborts at its next file or before it moves the branch,
releasing `index.lock`, and none starts (review N2).

All git work runs on one dedicated thread per graph binding, never an async worker (spike finding: libgit2
blocks, and a push cannot be cancelled). libgit2 runs with `GIT_OPT_ENABLE_FSYNC_GITDIR`. After 50 loose
commits (or 6 700 loose objects) the thread repacks incrementally, like `git repack -d` without `-a`: libgit2's
pack builder writes one new pack of the loose objects, the pack, its index and the pack directory are fsynced,
and then only the loose objects that pack holds are deleted (git's `prune-packed`). Existing packs are never
rewritten, so a repack costs what was added since the last one, not the history (review N1). Unreachable loose
objects are packed, never dropped: they may be the user's `git add`, stash or reflog history, or a concurrent
`git commit`'s objects before its ref moves; the user's own `git gc` prunes them as usual. Skipped while
`gc.pid` exists (a `git gc` is running). Pack and index fsyncs use libgit2's writable
creation handles before close/rename; reopening the final read-only files for fsync
fails on Windows (`FlushFileBuffers` requires write access). A failed repack keeps
its loose objects and remains pending, including when the loose-object threshold
triggered it. Commits continue; Settings shows deferred cleanup separately from a
commit refusal, without a toast. Retry on a successful commit cycle after at least
10 minutes; clear the status when cleanup succeeds.

### 4. Applying a pull (the one-way door)
Sync = fetch, then fast-forward, then push. The fast-forward never runs git's checkout. Let `H` be `HEAD` and `U`
the fetched upstream tip (the remote-tracking ref, which fetch already persists), with `U` a strict descendant of
`H`. The order below is the storage model s3.7 (`docs/storage-model/spec/SPEC-s37-gitpull.md`): the literal
order of this ADR's first version failed four of its five checked properties, and repairs P1–P4 are part of the
design. The changed paths between `H` and `U` inside the graph root are applied in this order:

1. **Pre-check.** Every changed entry is a regular file (mode 100644/100755, no mode-only change); every
   non-page file holds its `H` blob, or is absent, or (on resume) already holds its `U` blob; no changed path's
   index entry is a staged blob other than its `H` or `U` blob (git's own fast-forward refuses there too). A
   symlink, gitlink, file↔directory change, mode-only change, foreign staged blob or an ignored local file in
   the way refuses the pull by name before anything is written.
   All page operations are planned and all non-page transactions run read-only
   validation before the first write. Publication repeats the guards; a later
   refusal after files changed is `pull-stopped`, naming the path and saying
   files were already updated.
2. **Pages** (anything the page host owns) go through `HostOp`s with `Effect::Write(new blob)` or `Delete`,
   `Expected::Bytes(H blob)` or `Absent`, `Input::Refuse`, and the existing edit kinds (`ReplacePage`,
   `CreatePage`, `DeletePage`, `RenamePage`; the first version's new `GitPull` kind is dropped, so the closed
   edit-kind vocabulary shared with the frontend does not change). Each upstream rename
   (git's rename detection) is its own op with `moves`, as the host allows one move per op; case-only renames
   follow the #609 contract. The remaining pages are one op. The host admits an op only if every page still holds
   exactly the `H` bytes and none has unsaved input. A path that already holds its `U` blob is skipped.
3. **Other files** (assets, files Tine does not parse) are written by the store's checked atomic writer (temp,
   fsync, rename, directory sync, pre-rename check against the `H` blob), parents created as needed; deletions by
   a checked delete. `logseq/config.edn` goes through the config writer's own lock and live publication. All are
   write-protocol census sites. The watcher reports these as outside changes, which is what they are.
4. **Durability barrier (P2)**, holding `index.lock`: every changed path is fsynced together with its directory
   (a deleted page: its directory and the trash), including paths skipped as already at `U`. A failed sync ends
   the cycle `pull-incomplete` with nothing moved.
5. **Index, then ref (P1, P1b).** Still holding `index.lock`, the index entries of the changed paths move to
   `U`'s blobs under `git_run.rs`'s staged-kept rule and the index is installed durably (copy fsynced, renamed,
   `.git` synced; Windows: identity checks without directory flush). Then `index.lock` is taken again and the
   branch moves from `H` to `U` by compare-and-swap. A crash between the two leaves `HEAD = H` over an index at
   `U`: the next cycle resumes, and a plain `git commit` meanwhile records `H` plus the pull's changes, never a
   reversion. A refused compare-and-swap leaves the index at `U`, which matches the working tree.

**Resume.** A crash, an `Uncertain`/`Unwritten` answer or a refusal before step 5 completes leaves some paths at
`U` and the ref at `H`. Before **every** autocommit (pre-pull, cadence, background/quit, structural brackets: P3),
if a fetched `U` is a strict descendant of `H` and every path that differs from `H` holds its `U` blob or is in
this process's applied-path set (P4), the autocommit is skipped and the next sync resumes the pull from step 1.
Otherwise (a genuine local change) the autocommit runs and the pull ends as `diverged`: nothing is lost.

The host records P4 at each guarded publication, even when the caller already
received `Uncertain`. Host admission checks that same receipt set under the
publication-owning driver state, including before a case-only entry move.
A receipt arriving after planning holds the stale operation without writing
the receipted path; the next cycle replans and skips it. An unchanged case-only
destination is already published by the entry move and needs no content write.
Case-only candidates use the store's Unicode fold; actual alias/absence checks
and guarded no-replace publication still decide whether the entry can move.
An admitted pull pins its `H`, `U` and branch in memory:
later fetches may advance the tracking tip, but pull and P3 finish that same `U`
first. A tracking tip that no longer descends from the pending `U` stops as
`diverged` with a force-push reason, retaining the pending provenance.

**Stated limit (D-20261028-GU).** If the user edits a pulled path back to exactly its `H` bytes while the pull is
held, and the process then crashes, loses power or switches graph before step 5 (or another program, such as a
sync service, delivers `U` and then `H`), the resumed pull writes `U` over that text. The overwritten text is
`H`'s blob and stays in git history. Within one process the applied-path set prevents it.

Refusals (contract refusal table rows, each with its in-scope threat):

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| `pull-held: unsaved` (a pulled page has unsaved input) | concurrent edit in a window | retried at the next sync once the page saves; earlier operations may already have updated files |
| `pull-held: changed` (a file no longer holds its `H` bytes, or a prior publication made the plan stale; HostOp `Changed`) | an external editor, Syncthing or an earlier Uncertain pull operation changed it after planning | the next cycle replans, skipping receipted paths; a genuine local change is autocommitted, then diverges |
| `pull-held: busy` (HostOp `Busy`, `index.lock`, `gc.pid`) | other work holds the pages, or another git command runs | retried later; the stale-lock notice after 30 min (§1) |
| `pull-incomplete` (HostOp `Unwritten`, `Superseded`, `Uncertain`, `DraftFailed`, a failed non-page write or durability barrier) | disk error, full disk, a page edited back before it published | the ref stays at `H`; the resume rule finishes it |
| `pull-refused: <path> <reason>` (symlink, submodule, type or mode change, a staged change other than `H`'s or `U`'s, ignored local file in the way) | the user's repository content | Settings names the path; the user resolves it in git |
| `pull-stopped: <path> <reason>` (a refusal after publication began, including a staged-blob race) | another git program or external writer changes a guard after planning | Settings names the path and says files were already updated; resolve it in git, then retry the pinned target |
| `diverged` (the remote is not a descendant of `HEAD`, a force push replaced a pending target, the captured branch changed, or a push is rejected) | multi-device divergence or another git program | phase 1 stops and says so; a mid-apply branch race may leave earlier file updates in place; phase 2 merges with Concord |
| `refused: conversion` | `core.autocrlf` or attributes would rewrite checkout bytes | For truthy autocrlf, hash each tracked regular file without filters against its index oid. Missing/unreadable/nonregular files or gitlinks count as differing paths, with a count and sample in the notice. If all match and no `text`/`eol`/`crlf`/filter/encoding conversion remains, offer **Turn off line-ending conversion for this repository**. Under `RUN_LOCK` and `index.lock`, repeat the probe and write only `core.autocrlf=false` at local config level; no graph files change. A higher-level worktree override refuses the fix. Otherwise link to Guide **Git line endings on Windows**: commit edits, verify a clean status, set local autocrlf false, then `git reset --hard HEAD` to re-check out that saved commit. Sync still refuses conversion. |
| `refused: sparse / precompose / bare / detached / mid-merge` | user's repository configuration | Settings explains; commit-only where possible |

Push never forces and never rebases.

### 5. Transport and credentials
HTTPS through a libgit2 smart transport registered over Tine's Rust HTTP/TLS stack (spike `gitspike/src/transport.rs`),
token auth; system trust store on every platform (Android via rustls-platform-verifier, G8). SSH only when a user
asks. Credential storage: open question 2.

### 6. Slices
1. Autocommit (policy Off / Commit automatically), cadence, structural brackets, repack, Settings. Desktop.
2. Sync on desktop: transport, credentials, fetch, the §4 apply, push, the diverged notice.
3. Android and iOS: libgit2 builds, trust store, lifecycle triggers (open, foreground, background).
4. Phase 2 (Concord merge of a diverged history): a separate decision.

Slice 3 (D-20261028-GM) removed the mobile "Git is not available on this device yet" arm: git, sync and seen are
one implementation on all five targets, pinned by `git_has_no_platform_arm`; the HTTPS transport names its TLS
stack per target and refuses to compile for any other (AGENTS §2). Lifecycle on Android and iOS: at open the
worker starts as on desktop; going to the background flushes the pages, then each worker commits what is
pending (at most one autocommit per interval, D-20261026-GC: backgrounding is no exemption from §Unit cost's rate; held-back edits
are saved and commit at the next allowed moment or the next launch) and, under **Commit and sync automatically**, pushes what the last fetch shows the remote lacks, with no
fetch or pull (a push writes nothing to the local repository); coming back to the foreground runs a full cycle at
once under that policy, without forcing a commit (unlike **Sync now**). Until then no automatic cycle runs, however
long the app stays away: the background edge sets an away state that only the foreground edge clears, a worker that
starts meanwhile (a bind ending after the switch) starts away, the frontend reports an app already hidden when it
mounts, a **Sync now** asked for meanwhile waits for the foreground, and the frontend drops a background edge that
a newer foreground edge overtook during its page flush (review F1, F4; round 2 B1, FU1). The
pull's own pre-pull autocommit (§3) is outside the cap, so a foreground cycle can add one commit after a background
one (review F7, a follow-up). Limit: a commit holds `index.lock` (and briefly the branch's ref lock); a push takes it
only for its local checks, not during the network transfer (review F3). The OS ending the app midway can leave either, and
§1's busy-then-name rule applies (measured in `git_mobile_tests.rs`, which CI also runs on an Android emulator with
the graph on shared storage). Android's shared storage (MediaProvider FUSE) reports another owner for every file,
so libgit2's repository-owner check is off on Android only (accepted under the trust model, 2026-10-28: it excludes an attacker with write access
to the files). On that storage a case-only rename succeeds but keeps the old spelling, so a pull that renames a page
by capitalization alone stays `pull-incomplete` and writes nothing (a stated limit for 0.9.0-beta.1, D-20261029-GS1; a two-step rename follows in a later 0.9 Beta).

## Unit cost (I-25)

Measured in the spike (`specs/campaigns/2026-10-git-sync/spike/measure/*.txt`, caman, libgit2 1.9 via git2 0.21).

- **Per edit:** nothing. Edits are never committed one by one.
- **Per autocommit:** one zlib blob per changed file (a 543 B page: 342 B on disk; 60 changed pages ≈ 32 KB), the
  rewritten trees on the changed paths, one commit (160 B). The trees dominate: Logseq keeps all pages in one
  flat `pages/` folder, so its tree object is rewritten whole on every commit: **247 KB loose at 10 000 pages**,
  25 KB at 1 000. Files: one loose object per new object (blobs + 2 trees + commit), plus ref and reflog, and the
  index rewritten whole (880 KB at 10 000 pages). Index install: one fsync of the index copy and one of `.git`
  (Windows: none, identity checks only) per commit (s3.7 P1b; counted from the code, `IndexLock::install`).
- **Rate:** at most one autocommit per interval of editing (default 10 min) plus one per structural operation:
  ≤ 6 per editing hour, ≤ 1.5 MB loose per editing hour at 10 000 pages. The incremental repack at 50 commits
  deltifies the trees: on 10 000 flat pages, 50 one-page autocommits (200 loose objects) pack to 265 KB in
  0.33 s, ≈ 5.3 KB per commit, and the pack written before is untouched, so `.git` grows linearly
  (`measure_incremental_repack`, an ignored test in `git_autocommit_tests.rs`; caman, 2026-10-21). The spike's
  whole-history repack measured ≈ 13 KB per commit; slice 1 does not rewrite history.
- **Transport:** push ≈ 240 KB per pushed commit at 10 000 pages (25 KB at 1 000; libgit2 sends the tree
  undeltified); fetch of the same commit ≈ 1 KB (the server deltifies). A 60-page edit: push 309 KB, fetch 33 KB.
- **Enabling:** the first autocommit of a graph never committed before stores every file (10 000 pages: 12 s
  with fsync, 2 s without; 42 MB `.git` before repack).
- **Memory:** no history walk (seen bounds its own reads to 200 commits per page).

## Consequences

- Git graphs gain history and sync without a terminal; graphs that are not repositories see nothing.
- A pull can be held by one unsaved page. That is the price of never overwriting an edit.
- History grows with the flat `pages/` tree; the 10-minute interval is the lever, and it is a setting.
- A user's own commits on another device count as "mine" for seen (OJ4b); an autocommit's interval delays what
  seen shows across devices by up to the interval.
- A second external writer (Tine's pull) joins the storage model; the write-protocol census and the contract's
  refusal table each gain rows.

## Open questions for Martin

1. **Unit cost** (autocommit part answered 2026-10-26, D-20261026-GC: accepted; the push part stays open): accept 247 KB loose per autocommit (≈ 5.3 KB once repacked) and ≈ 240 KB upload per push at
   10 000 flat pages, with the interval as the user's lever?
2. **Credentials (slice 2):** vision 9(b) says OS keychain; quick share (ADR 0076, your S1) chose a private
   app-data file instead (keychains are uneven on Android/iOS; the trust model does not require one). Follow 0076
   for git tokens too?
