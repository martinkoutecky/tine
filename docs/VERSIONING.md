# Branches, tags and releases

How Tine's git history is organised, for people who build from source or follow
the Beta. Discussion: [#682](https://github.com/martinkoutecky/tine/discussions/682).

## Two lines

| Line | Branch | Release tags | Installed app |
|---|---|---|---|
| **Beta** | `og` (to be renamed `beta`) | `vX.Y.Z-beta.N` | Tine Beta (separate app identity, installs next to stable) |
| **Stable** | `master` | `vX.Y.Z` | Tine |

All work lands on the Beta line first. Work happens on short-lived topic
branches that are merged into `og` and deleted once merged. Every commit on
stable is also on Beta: a fix that stable needs before the next stable cut is
merged into both lines from one branch (below).

To follow the latest Beta code, check out `og` and `git pull`.

## Beta releases

A Beta release is a commit on `og`, tagged `vX.Y.Z-beta.N` when it is published.
Betas come often; anything merged since the last one ships in the next.

Tags should not move. The `beta` tag is the one exception, for now: installed
Betas fetch their updates from `releases/download/beta/latest.json`, so it has
pointed at the latest Beta release. After 0.8.0-beta.2 the Beta update manifest
moves to a fixed address on tine.page. `beta` then moves one last time, to the
first Beta that reads the new address, so existing installs reach it, and is
then frozen and later removed. `og` is renamed `beta` once the tag is gone (a
branch and a tag with the same name would be ambiguous).

## Stable releases

A stable release `vX.Y.Z` is cut by **one merge commit** that brings `master`
to a Beta tag that has soaked for at least five days with no open severe
regression compared to stable. (A severe bug that stable has too does not block
the cut.)

After the cut, `master` differs from that Beta tag only in:

- the channel identity: app name, identifier and update address, written by
  `node scripts/set-app-identity.mjs release` into `src-tauri/app-identity.json`,
  `src-tauri/tauri.conf.json`, `src-tauri/gen/android/app/build.gradle.kts` and
  `src-tauri/gen/android/app/src/main/res/xml/shortcuts.xml`
  ([app-identity.md](app-identity.md));
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
- Version tags never move. `beta` is the one exception until the updater
  leaves it (above).
- Every published release has a tag.

## Issue labels

`fixed-in-beta` means the fix is merged on the Beta line (`og`) and will ship
in the next Beta release. The issue stays open until a release containing the
fix is published, then it is closed with a link to that release.
