# Page image icons (GH #346)

The existing `icon::` property accepts a single image link:

```markdown
icon:: ![](../assets/logo.png)
```

```org
#+icon: [[file:../assets/logo.png]]
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
Namespace macros use the same reactive lookup as titles and lists. Any change
event under `assets/` invalidates every cached icon image, including alternate
symlink/case spellings; mounted surfaces share the next read per asset path.

OG Logseq 0.10.9 keeps icon values unparsed (`graph_parser/property.cljs` and
`text.cljs::parse-property`) and inserts them as text (`components/page.cljs::page-title`
and `all-pages`). It shows the link text, without changing or losing the value.
Using `icon::` is the manager's provisional choice pending Martin's confirmation.
