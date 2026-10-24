# Page image icons (GH #346)

## Property (D-20261018-IC1, Martin)

The page icon is the page property **`tine.icon::`**, falling back to **`icon::`**
when the page has no non-blank `tine.icon::`. When both exist, `tine.icon::` wins
(by name, not by position in the file). Both accept emoji, short text and a single
image link. The policy lives in one place, `crates/tine-core/src/page_icon_property.txt`
(`tine.icon icon`, highest precedence first): Rust (the store's icon index, which answers
unloaded pages and aliases) includes it and TypeScript imports the same file (loaded
pages, title, menu, visible property). A refused `tine.icon::` image link shows no icon;
it does not fall through to `icon::` (the property that is present decides).

Why not `icon::` alone: OG Logseq inserts the `icon::` value as raw text before every
page title, so an image link there prints as markup in OG. `tine.icon::` is an
ordinary property in OG, so an image link in it costs OG users nothing. `icon::` stays
as the fallback so OG-authored emoji icons keep working; it accepts the same values
(including image links) as `tine.icon::`.

The **Set page icon…** menu edits the property that currently supplies the icon (a page
with only `icon::` keeps a single, OG-visible property) and writes `tine.icon::` for a
page with none. **Remove page icon** clears both. The `tine.icon::` row stays visible and
editable in the page-properties block (the hiding of `tine.*` keys applies to block
chips and the sheet-cell/properties-panel editors, not to the page header; only
`tine.tag-table` is hidden there).

The property accepts a single image link:

```markdown
tine.icon:: ![](../assets/logo.png)
```

```org
#+tine.icon: [[file:../assets/logo.png]]
```

The link resolves relative to its page file. Only local static PNG, JPG and JPEG
files no larger than 1 MiB (1,048,576 bytes) inside the graph's `assets/`
directory qualify (extensions ignore case). Larger files show no icon.
This uses the same transport containment as block images:
`crates/tine-graph-features/src/assets.rs::named_asset` constructs `Area::Assets`
identities and `validate_read_name` refuses traversal. Store containment still
checks symlinks; this feature does not widen that boundary.

Other graph folders, escapes, schemes (except Org's local `file:` link syntax),
other formats, animated PNGs, missing, unreadable and undecodable files show no
icon: refused image links never appear as raw text on icon surfaces. Ordinary
text and emoji icons keep their existing behavior. The property row still shows
the value as written. No remote image request is made. No files are downloaded,
copied, cached persistently, resized or written. The existing bounded in-memory
asset transport is shared with block images: one Blob per asset path across
surfaces, including active leases outside the idle cache. Icon reads pass the
1 MiB bound to the existing backend before allocation; already-cached larger
block images are also refused as icons. This limits encoded bytes per icon;
decoded browser memory still depends on the source image dimensions.

Images use the emoji's box with object-fit contain on every existing icon
surface. Link width/height attributes are ignored, including in the page title.
Property edits and asset replacement follow the existing live update paths.
For a name claimed by several ordinary files, the icon comes from the same
winning file as name resolution (matching filename first, then `.md`, filename,
and full path). A winner without an icon shows none. Holding a page or reloading
the graph configuration does not transfer its icon to a competing file.
Namespace macros use the same reactive lookup as titles and lists. Any change
event under `assets/` invalidates every cached icon image, including alternate
symlink/case spellings; mounted surfaces share the next read per asset path.

OG Logseq 0.10.9 keeps icon values unparsed (`graph_parser/property.cljs` and
`text.cljs::parse-property`) and inserts them as text (`components/page.cljs::page-title`
and `all-pages`). It shows the link text, without changing or losing the value.
The property choice is Martin's decision (D-20261018-IC1), replacing the earlier provisional `icon::`.
