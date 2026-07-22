# Popular-plugin compatibility snapshot

This is a prioritization aid, not a compatibility promise. It asks whether the
user-visible behavior of currently popular Logseq and Obsidian plugins fits Tine
plugin API 0.2. Download rank is a demand signal; it does not make a feature belong
in Tine core or justify ambient authority.

Sources were read on 2026-07-12 from the official
[Logseq marketplace popularity data](https://github.com/logseq/marketplace/blob/master/popular.json)
and Obsidian's official [plugin statistics](https://github.com/obsidianmd/obsidian-releases/blob/master/community-plugin-stats.json)
and [catalogue](https://github.com/obsidianmd/obsidian-releases/blob/master/community-plugins.json).
Logseq's checked-in popularity snapshot reports generation on 2026-01-02; ranks may
therefore lag the marketplace. Counts are deliberately omitted because cumulative
download totals across the two ecosystems are not directly comparable.

Disposition vocabulary:

- **Fits**: a faithful useful port can use API 0.2 now.
- **Reduced**: a clearly described subset fits; the legacy plugin as a whole does not.
- **Host primitive**: one or more narrow reusable host-rendered/read operations are missing.
- **Core**: Tine already owns or should own the semantic feature.
- **Privileged**: the feature inherently needs authority ordinary plugins must not receive.

## Logseq top ten non-theme plugins

| Rank | Plugin | API 0.2 disposition | Minimum honest Tine shape |
|---:|---|---|---|
| 1 | Journals calendar | Host primitive | Bounded journal-date read plus host-owned calendar panel and navigation. |
| 2 | Bullet Threading | Fits | Existing `thread-lines` decoration; settings select all blocks or the active ancestry. |
| 3 | Tabs | Core | Pane/window navigation policy belongs with Tine's multi-window architecture, not a guest-owned DOM tab bar. |
| 4 | TODO List | Host primitive | Bounded task query, task marker mutations, and a host-owned list surface. |
| 5 | Markmap | Host primitive | Current-page outline snapshot plus a host-rendered interactive tree contribution. |
| 6 | Tags | Host primitive | Bounded tag index/query plus host-owned list/search UI. |
| 7 | Markdown Table Editor | Reduced | Focused-block table transformations can be commands; full cell-aware editor integration needs a table editor contribution. |
| 8 | GPT-3 OpenAI | Privileged | A separately reviewed secret-and-network broker; never raw network or environment access. |
| 9 | PDF Export | Core | Export/printing is document semantics and already belongs in Tine's core export path. |
| 10 | Heatmap | Host primitive | Bounded journal/activity aggregation plus a host-owned calendar heatmap. |

## Obsidian top ten plugins

| Rank | Plugin | API 0.2 disposition | Minimum honest Tine shape |
|---:|---|---|---|
| 1 | Excalidraw | Core / privileged | A deliberately integrated canvas document type and asset lifecycle, not arbitrary DOM injection. |
| 2 | Templater | Reduced | Deterministic focused-block template variables can fit; filesystem, JavaScript, shell, and arbitrary vault traversal do not. |
| 3 | Dataview | Core / host primitive | Extend Tine's query language and host renderers rather than expose the graph database or DOM. |
| 4 | Tasks | Host primitive | Same bounded task query/list/mutation surface as Logseq TODO List. |
| 5 | Advanced Tables | Reduced | Keyboard commands that rewrite the focused Markdown table fit; interactive cell editing needs a host contribution. |
| 6 | Calendar | Host primitive | Same bounded journal calendar surface as Logseq Journals calendar. |
| 7 | Git | Privileged | Filesystem, credentials, process/network access, and conflict UX require a named privileged integration or core feature. |
| 8 | Style Settings | Core analogue | Declarative Tine plugin settings and inert token-theme manifests cover the safe analogue; arbitrary CSS variables/selectors do not. |
| 9 | Kanban | Core | Tine's query boards already own this view; improvements should target the core board model. |
| 10 | Iconize | Host primitive | A bounded packaged-icon asset type and host-owned icon decoration point, with no remote font or arbitrary CSS loading. |

## What this says about the launch API

API 0.2 is already useful for commands, safe focused-block transformations, and a
small visual vocabulary. It is intentionally not broad enough to port most of the
largest legacy plugins literally. The repeated missing spine is **bounded semantic
reads plus host-rendered views**: calendar, list, tree, and icon contributions. That
is a plausible future API family. Raw DOM, database, filesystem, process, secret, and
network access are not substitutes for it.
