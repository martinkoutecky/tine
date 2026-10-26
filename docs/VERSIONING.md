# Branches, tags and releases

How Tine's git history is organised, for people who build from source or follow
development or Beta. Discussion: [#682](https://github.com/martinkoutecky/tine/discussions/682).

## Development, Beta and stable

| Line | Branch | Release tags | Installed app |
|---|---|---|---|
| **Development** | `develop` | Candidate Beta tags before the soak | Tine Beta (maintainer development builds) |
| **Beta** | `release/X.Y` while a candidate soaks | `vX.Y.Z-beta.N` | Tine Beta (separate app identity, installs next to stable) |
| **Stable** | `master` | `vX.Y.Z` | Tine |

All work lands on the development line first. Work happens on short-lived topic
branches that are merged into `develop` and deleted once merged. Every commit on
stable is also on development: a fix that stable needs before the next stable cut is
merged into both lines from one branch (below).

To follow development, check out `develop` and `git pull`. To follow a Beta
soak, check out its `release/X.Y` branch. The development branch was named
`og` until October 2026; older records retain that name. Checkout directories
and app identities keep their existing names (see [app-identity.md](app-identity.md)).

## Beta releases

A candidate Beta is tagged `vX.Y.Z-beta.N` on `develop`. Its soak starts a
`release/X.Y` branch from that tag; later Betas of that version ship from the
soak branch. Development continues on the next version. Soak fixes start from
`release/X.Y` and merge into `develop` first, then into the soak branch.
After the stable cut, the soak branch is deleted until the next candidate.

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
development), is merged into `develop` first, and then the same
branch is merged into `master`. One commit sits in both histories, so
`git branch --contains <commit>` shows where a fix shipped.

(Before 2026-10-15 such fixes were copied to stable with `git cherry-pick -x`,
for example in 0.7.3. The next stable cut absorbs those copies.)

## Rules we keep

- `develop`, `release/X.Y` and `master` are never force-pushed.
- Tags never move. (`beta` moved with each Beta up to 0.8.0-beta.3 and is
  frozen there, above.)
- Every published release has a tag.

## Commit messages

A commit that fixes or implements an issue cites it as `GH #N` in its
message, and so does the merge commit that brings its branch into `develop` or
`master`, so the commit shows up in the issue's thread. We use a mention, not
a closing keyword such as `fixes #N`: a closing keyword would close the issue
as soon as the fix reaches `master`, before the release comment with the
version and our confidence (below). Commits from before 2026-10-16 are
inconsistent about this and are not rewritten.

## Issue labels

`fixed-in-development` means the fix is on `develop`; it ships in the first
Beta of that development version. `fixed-in-beta` means the fix is on the
current Beta soak branch (`release/X.Y`) and will ship in its next Beta.
The issue stays open until a release containing the
fix is published, then it is closed with a link to that release.
