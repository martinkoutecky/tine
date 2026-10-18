# Page image icons (GH #346)

The existing `icon::` property accepts a single image link:

```markdown
icon:: ![](../assets/logo.png)
```

```org
#+icon: [[file:../assets/logo.png]]
```

The link resolves relative to its page file. Only local static PNG, JPG and JPEG
files inside the graph's `assets/` directory qualify (extensions ignore case).
This uses the same transport containment as block images:
`crates/tine-graph-features/src/assets.rs::named_asset` constructs `Area::Assets`
identities and `validate_read_name` refuses traversal. Store containment still
checks symlinks; this feature does not widen that boundary.

Other graph folders, escapes, schemes (except Org's local `file:` link syntax),
other formats, animated PNGs, missing and unreadable files retain the existing
text/emoji fallback. No remote image request is made. No files are downloaded,
copied, cached persistently, resized or written. The existing bounded in-memory
asset transport is shared with block images.

Images use the emoji's box with object-fit contain on every existing icon
surface. Link width/height attributes are ignored, including in the page title.
Property edits and asset replacement follow the existing live update paths.

OG Logseq 0.10.9 keeps icon values unparsed (`graph_parser/property.cljs` and
`text.cljs::parse-property`) and inserts them as text (`components/page.cljs::page-title`
and `all-pages`). It shows the link text, without changing or losing the value.
Using `icon::` is the manager's provisional choice pending Martin's confirmation.
