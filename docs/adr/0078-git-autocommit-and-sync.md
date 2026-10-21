# 0078 — Git graphs: Tine autocommits, and syncs fast-forward only through the page host

- **Status:** Proposed (design accepted by Martin 2026-10-18/19: D-20261018-G1…G7, D-20261019-G8/G9; the unit-cost
  line below awaits his acceptance)
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
| D-20261018-G5 | an autocommit includes every change git sees under the graph root, honouring `.gitignore`; Tine applies pulls itself through the audited writer; sync off under eol/filter conversion; Tine never writes `.gitignore` unasked | follow |
| D-20261018-G6 | lands in whatever release the timing gives; no release waits for it | follow (card hBpb2k's "after model v0" is stale) |
| D-20261018-G7 | libgit2 (git2-rs, `default-features = false`) on all five platforms; HTTPS through Tine's own Rust HTTP/TLS stack as a registered smart transport; no OpenSSL | follow |
| D-20261019-G8 | Android TLS through the OS trust store (rustls-platform-verifier); Tine owns repack; batched cadence; libgit2 fsync on; SSH later, pure Rust, only when asked (supersedes G7's libssh2) | follow |
| D-20261019-G9 | autocommit when no page saved for 60 s and ≥ 10 min (a setting) since the last one; at once on background/quit, before a pull, and around structural operations; repack at 50 loose commits on the git thread, skipped while a `git gc` lock exists | follow |
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
webview hands them to the graph's binding when it binds and on every change. Nothing is written to the repository
or its config. Refused with a reason in Settings: bare repository, merge/rebase/cherry-pick in progress, detached
HEAD (commit only, no sync), eol/filter conversion of graph text files (`core.autocrlf`, `.gitattributes`
`text`/`eol`/`filter`: git's bytes would differ from Tine's). A held `index.lock` is "busy, retry", not an error.
Sync additionally needs the graph root to be the work tree root: a fast-forward of a bigger repository changes
files outside the graph, which Tine does not own.

### 2. What an autocommit is
Every change git sees under the graph root (pathspec = the graph root), honouring `.gitignore`: Tine's saves,
other programs' edits, deletions. It is built on a private copy of the index taken while Tine holds
`index.lock` (the protocol `git_run.rs` already uses): stage the graph root's changes into the copy, write its
tree, take only the graph root's subtree into `HEAD`'s tree (a user's staged changes outside the graph are never
committed), commit, move the branch by compare-and-swap, then rename the copy over the index. Author and
committer: the repository's identity, else "Your name" with `user@tine.invalid` (`git_run.rs`'s fallback, which
seen's "me" matches). Message `Tine: autosave`; no trailers (G3: generic). Nothing to commit → no commit.

### 3. When
Per G9: when no page has saved for 60 s and the interval (default 10 min) has passed since the last autocommit;
at once when the last window closes or the app goes to the background, before every pull, and around a
structural operation (rename, merge, delete or move of a page, restore from backup): commit pending changes
first, run the operation, then commit its files with its verb (`Rename page "A" to "B"`). A failed operation
leaves only the first commit. Autocommits take the CLI runs' lock, so they never fall between a run write's
snapshot and its agent commit; a run whose tip an autocommit moved is not squashed (CL1 already refuses a moved
tip), and the reply says so.

All git work runs on one dedicated thread per graph binding, never an async worker (spike finding: libgit2
blocks, and a push cannot be cancelled). libgit2 runs with `GIT_OPT_ENABLE_FSYNC_GITDIR`. After 50 loose
commits the thread repacks with libgit2's pack builder, skipped while `gc.pid` or a `*.lock` it would replace
exists.

### 4. Applying a pull (the one-way door)
Sync = fetch, then fast-forward, then push. The fast-forward never runs git's checkout. For each path changed
between `HEAD` and the fetched commit, inside the graph root:

- a page (anything the page host owns) goes into one `HostOp` with `Effect::Write(new blob)` or `Delete`,
  `Expected::Bytes(HEAD blob)` or `Absent`, `Input::Refuse`. The host admits it only if every page still holds
  exactly the `HEAD` bytes and none has unsaved input;
- any other file (assets, `logseq/config.edn`, files Tine does not parse) is written by the store's checked
  atomic writer (temp, fsync, rename, directory sync, with the pre-rename check that the file still holds the
  `HEAD` blob), a new site in the write-protocol census.

Only when every path has landed do the branch ref (compare-and-swap from the old tip) and the index move. A
crash or refusal part-way leaves some files at the new version and the ref at the old one: `git status` shows
them as changes, and the next pull skips every file already at its new blob, so the pull is idempotent. Because
an autocommit runs first, the working tree is clean at the start; a change that appears after that autocommit is
caught by the `Expected` check.

Refusals (contract refusal table rows, each with its in-scope threat):

| Refusal | In-scope threat | Recovery |
| --- | --- | --- |
| `pull-held: unsaved` (a pulled page has unsaved input) | concurrent edit in a window | retried at the next sync once the page saves; nothing written |
| `pull-held: changed` (a file no longer holds its `HEAD` bytes) | an external editor or Syncthing changed it after the autocommit | the next cycle autocommits it, then usually diverges |
| `diverged` (the remote is not a descendant of `HEAD`, or a push is rejected) | multi-device divergence | phase 1 stops and says so; nothing in the graph changes; phase 2 merges with Concord |
| `busy` (`index.lock`, `gc.pid`) | another git command at work | retried later |
| `refused: conversion` / bare / detached / mid-merge | user's repository configuration | Settings explains; commit-only where possible |

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

Until slice 3, iOS and Android show "Git is not available on this device yet" in Settings, named explicitly in
the platform `cfg` and pinned by a guard test (AGENTS §2).

## Unit cost (I-25)

Measured in the spike (`specs/campaigns/2026-10-git-sync/spike/measure/*.txt`, caman, libgit2 1.9 via git2 0.21).

- **Per edit:** nothing. Edits are never committed one by one.
- **Per autocommit:** one zlib blob per changed file (a 543 B page: 342 B on disk; 60 changed pages ≈ 32 KB), the
  rewritten trees on the changed paths, one commit (160 B). The trees dominate: Logseq keeps all pages in one
  flat `pages/` folder, so its tree object is rewritten whole on every commit: **247 KB loose at 10 000 pages**,
  25 KB at 1 000. Files: one loose object per new object (blobs + 2 trees + commit), plus ref and reflog, and the
  index rewritten whole (880 KB at 10 000 pages).
- **Rate:** at most one autocommit per interval of editing (default 10 min) plus one per structural operation:
  ≤ 6 per editing hour, ≤ 1.5 MB loose per editing hour at 10 000 pages. Repacking at 50 commits deltifies the
  trees: 100 one-page autocommits on 10 000 pages packed to 13.3 MB in 2.6 s, of which ≈ 12 MB is the initial
  commit's pages, so ≈ 13 KB per packed commit (same size as `git gc`).
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

1. **Unit cost:** accept 247 KB loose per autocommit (≈ 13 KB once repacked) and ≈ 240 KB upload per push at
   10 000 flat pages, with the interval as the user's lever?
2. **Credentials (slice 2):** vision 9(b) says OS keychain; quick share (ADR 0076, your S1) chose a private
   app-data file instead (keychains are uneven on Android/iOS; the trust model does not require one). Follow 0076
   for git tokens too?
