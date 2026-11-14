# Settings discovery

Each setting has a descriptor: stable ID, section/tab, catalog label and
description, optional search vocabulary and disclosure level. Settings controls
consume these descriptors for their visible labels, descriptions and
`data-setting-id` targets. Search enumerates the descriptors; it has no separate
list of setting labels. Controls can supply rich description slots and current
formatting arguments without redefining the catalog keys.

Static descriptors live in `src/components/settingDefinitions.ts`. Style knobs,
media editors, built-in gallery themes, installed and community themes, and
installed plugin settings use the descriptor adapters in `settingsDescriptor.ts`
over their existing registries. Search reads a reactive snapshot, so installing
or removing a package changes discovery without reopening Settings. Choosing a
plugin setting opens that exact plugin version's form before locating the row.

Search matches localized labels and descriptions, English originals and aliases.
A one-character term matches only the displayed-language text; longer terms can
match English too. Other inactive translations are not separately indexed.
Theme and plugin text is author-supplied and remains verbatim. Name matches rank
before description matches; supplemental description vocabulary keeps that rank.

Choosing a result ends search, reveals its Advanced section when needed and
scrolls to its stable ID, including controls that mount after an asynchronous
read. Labels are never navigation keys. Keyboard shortcuts retain their own
command/key filter.

Themes is a separate category on every screen; static and package theme
descriptors target it. On touch screens up to 480 CSS px wide, Settings fills
the screen within the safe areas and opens a category list. A category or search
result opens its pane; Back returns to the list, then dismisses Settings. The
iOS edge swipe uses the same Back path. Desktop and tablet Settings keep the
category navigation beside the pane. This navigation state is not persisted.

`docs/SETTINGS-INVENTORY.md` is generated with
`node scripts/generate-settings-inventory.mjs`; `--check` and
`settingsInventory.test.ts` enforce generation. The inventory includes gated
static controls and describes device-specific package contributions. Component
tests enumerate rendered rows and prove their descriptor labels find the same
IDs in English and Chinese; native acceptance is
`scripts/e2e-settings-descriptors.mjs`.
