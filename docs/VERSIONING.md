# Branches, tags and releases

How Tine's git history is organised, for people who build from source or follow
the Beta. Discussion: [#682](https://github.com/martinkoutecky/tine/discussions/682).

## Two lines

| Line | Branch | Release tags | Installed app |
|---|---|---|---|
| **Beta** | `og` | `vX.Y.Z-beta.N` | Tine Beta (separate app identity, installs next to stable) |
| **Stable** | `master` | `vX.Y.Z` | Tine |

All work lands on the Beta line first. Work happens on short-lived topic
branches that are merged into `og` and deleted once merged. Every commit on
stable is also on Beta: a fix that stable needs before the next stable cut is
merged into both lines from one branch (below).

To follow the latest Beta code, check out `og` and `git pull`.

## Beta releases

A Beta release is a commit on `og`, tagged `vX.Y.Z-beta.N` when it is published.
Betas come often; anything merged since the last one ships in the next.

Each Beta is published as its own GitHub release at that tag, marked as a
prerelease. From 0.8.0-beta.3 on, installed Betas read their updates from a
fixed address, `https://tine.page/updates/beta/latest.json` (AppImages: the
`amd64`/`aarch64.AppImage.zsync` files next to it). After a Beta is published,
the release workflow redeploys tine.page, which copies the newest Beta's
update files from its release (`scripts/pages-beta-manifest.mjs`).

Betas up to 0.8.0-beta.2 read their updates from the release at the old moving
tag `beta`. That tag moves one last time, to 0.8.0-beta.3, and then stays
frozen: its release holds only 0.8.0-beta.3's update files and links to its downloads,
so any older Beta, whenever it next checks, updates to 0.8.0-beta.3 and from
there follows tine.page. It is kept for good; removing it would strand Betas
that have not updated yet (`scripts/retire-beta-tag.mjs` makes the move).

## Stable releases

A stable release `vX.Y.Z` is cut by **one merge commit** that brings `master`
to a Beta tag that has soaked for at least five days with no open severe
regression compared to stable. (A severe bug that stable has too does not block
the cut.)

After the cut, `master` differs from that Beta tag only in:

- the channel identity: app name, identifier and update address, written by
  `node scripts/set-app-identity.mjs release` into `src-tauri/app-identity.json`,
  `src-tauri/tauri.conf.json`, `src-tauri/gen/android/app/build.gradle.kts` and
  `src-tauri/gen/android/app/src/main/res/xml/shortcuts.xml`, plus the stable
  updater endpoint and the deletion of the Beta config seed that
  [app-identity.md](app-identity.md) prescribes for the flip (made on
  `release/X.Y` in its release metadata, so the cut itself is a pure tree copy);
- the version: version strings, the dated changelog heading and the release
  notes.

The cut checks this with `git diff --stat <beta tag> master` before tagging;
anything else in that diff stops the cut. The history then shows exactly which
Beta became which stable release.

## Fixes between stable releases

An urgent fix that reproduces on stable can ship before the next stable cut, as
a patch release. Its branch starts from `master` (so it carries nothing from
Beta), is merged into `og` first, where Beta testers get it, and then the same
branch is merged into `master`. One commit sits in both histories, so
`git branch --contains <commit>` shows where a fix shipped.

(Before 2026-10-15 such fixes were copied to stable with `git cherry-pick -x`,
for example in 0.7.3. The next stable cut absorbs those copies.)

## Rules we keep

- `og` and `master` are never force-pushed.
- Tags never move. (`beta` moved with each Beta up to 0.8.0-beta.3 and is
  frozen there, above.)
- Every published release has a tag.

## Commit messages

A commit that fixes or implements an issue cites it as `GH #N` in its
message, and so does the merge commit that brings its branch into `og` or
`master`, so the commit shows up in the issue's thread. We use a mention, not
a closing keyword such as `fixes #N`: a closing keyword would close the issue
as soon as the fix reaches `master`, before the release comment with the
version and our confidence (below). Commits from before 2026-10-16 are
inconsistent about this and are not rewritten.

## Issue labels

`fixed-in-beta` means the fix is merged on the Beta line (`og`) and will ship
in the next Beta release. The issue stays open until a release containing the
fix is published, then it is closed with a link to that release.
