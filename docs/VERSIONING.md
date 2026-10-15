# Branches, tags and releases

How Tine's git history is organised, for people who build from source or follow
the Beta. Discussion: [#682](https://github.com/martinkoutecky/tine/discussions/682).

## Two lines

| Line | Branch | Release tags | Installed app |
|---|---|---|---|
| **Beta** | `og` (to be renamed `beta`) | `vX.Y.Z-beta.N` | Tine Beta (separate app identity, installs next to stable) |
| **Stable** | `master` | `vX.Y.Z` | Tine |

All work lands on the Beta line first. Work happens on short-lived topic
branches that are merged into `og` and deleted once merged. Nothing flows from
`master` back to `og`: every commit on stable is already on Beta.

To follow the latest Beta code, check out `og` and `git pull`.

## Beta releases

A Beta release is a commit on `og`, tagged `vX.Y.Z-beta.N` when it is published.
Betas come often; anything merged since the last one ships in the next.

The `beta` tag is the one tag we move: it always points at the latest Beta
release, because installed Betas fetch their updates from
`releases/download/beta/latest.json`. Once the in-app updater reads from a
different URL, the tag goes away and `og` is renamed `beta` (a branch and a tag
with the same name would be ambiguous).

## Stable releases

A stable release `vX.Y.Z` is cut by **one merge commit** that brings `master`
to a Beta tag that has soaked for at least five days with no open severe bug.
After the merge, `master`'s tree equals that Beta tag, so the history shows
exactly which Beta became which stable release.

## Fixes between stable releases

An urgent fix that reproduces on stable can ship before the next stable cut, as
a patch release (for example 0.7.3). It is first fixed on Beta, then copied to
`master` with `git cherry-pick -x`, which records the Beta commit it came from
in the commit message.

We cherry-pick here instead of merging because merging `og` would also bring
every unsoaked change on the Beta line. The next stable merge absorbs these
copies, so the two lines meet again at every stable cut.

## Rules we keep

- `og` and `master` are never force-pushed.
- Version tags never move; only `beta` does, for the updater.
- Every published release has a tag.

## Issue labels

`fixed-in-beta` means the fix is merged on the Beta line (`og`) and will ship
in the next Beta release. The issue stays open until a release containing the
fix is published, then it is closed with a link to that release.
