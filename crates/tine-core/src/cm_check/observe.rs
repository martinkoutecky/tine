//! The model's reader (Lean `Observations.read`) over Tine's parser: one
//! document parse gives the header, the preamble and the outline; each row's
//! sites are its block projection's reference evidence (the walk every
//! reference surface and the rename writer read), the page-name arguments the
//! query parsers read in its query macros, and the walk's rewrite targets
//! (links in other macros' arguments, Org `file:` link stems). Nothing here
//! recognizes Logseq structure by itself.

use super::{
    Atom, Block, Content, Context, Header, HeaderRole, NativeForm, Part, QueryMacro, Site, SiteKind,
};
use crate::block_regions::{BlockRegions, Property};
use crate::config::FileNameFormat;
use crate::doc::DocBlock;
use crate::model::Format;
use crate::reference_evidence::{ProjectedPageRef, RewriteTarget};
use std::ops::Range;
use std::sync::Arc;

#[cfg(test)]
thread_local! {
    /// Whole-document parses `observe` ran on this thread (tests reset it).
    pub(super) static DOCUMENT_PARSES: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// How one file is read: its format and the graph's file-name codec (an Org
/// `file:` link's stem names a page through it).
#[derive(Clone, Copy)]
struct Reader {
    org: bool,
    names: FileNameFormat,
}

/// Lean `Observations.read`: how the parser reads `text` as a page file of
/// `format`, coordinates kept for the expected side and erased by the
/// comparison. One whole-document parse; then one block projection per row,
/// as the page cache itself projects every block, and a second walk of a
/// block that can hold a macro or an Org file link. O(text bytes).
pub fn observe(text: &str, format: Format, names: FileNameFormat) -> Content {
    #[cfg(test)]
    DOCUMENT_PARSES.with(|n| n.set(n.get() + 1));
    let doc = if format == Format::Org {
        crate::org::parse_org(text)
    } else {
        crate::doc::parse(text)
    };
    observe_parsed(text, &doc, format, names)
}

/// [`observe`] of `text` whose whole-document parse `doc` the caller already
/// holds (the store parses a candidate once for this check and its save).
pub fn observe_parsed(
    text: &str,
    doc: &crate::doc::Document,
    format: Format,
    names: FileNameFormat,
) -> Content {
    let reader = Reader {
        org: format == Format::Org,
        names,
    };
    let org = reader.org;
    let mut content = Content {
        header: Vec::new(),
        blocks: Vec::new(),
    };
    let mut roots = doc.roots.as_slice();
    if let Some(pre) = doc.pre_block.as_deref() {
        let regions = crate::block_regions::parse_document(pre, org);
        let properties: Vec<&Property> = regions.page_properties().collect();
        if let Some(end) = properties.iter().map(|p| p.line.1).max() {
            let block = projected(&pre[..end], org);
            content.header = header_rows(&block, reader, properties.iter().copied());
        }
        if let Some(prose) =
            tine_outline::page_preamble::preamble_content_range_in(pre, org, &regions)
        {
            // The parser's text: its line endings are LF (`pre` is a prefix).
            let text = crate::doc::normalize_line_endings(text);
            let block = projected(preamble(&text, pre, prose.0), org);
            content.blocks.push(Block {
                depth: 0,
                body: whole_row(&block, reader),
                under_header: false,
            });
        }
    } else if let Some((first, rest)) = roots.split_first() {
        if crate::page_header::page_header_text(None, Some(first.raw())).is_some() {
            let projection = first.projection();
            let properties = projection.regions.properties.iter().filter(|p| p.primary);
            content.header = header_rows(first, reader, properties);
            walk(&first.children, 1, true, reader, &mut content.blocks);
            roots = rest;
        }
    }
    walk(roots, 0, false, reader, &mut content.blocks);
    content
}

/// The preamble prose as the model reads it: from where the parser's prose
/// starts to the first outline block of the parser's text `text`, so the
/// blank lines before that block stay (the parser's pre-block drops them).
/// When `pre` is not a prefix of `text`, the parser's own range.
fn preamble<'t>(text: &'t str, pre: &'t str, start: usize) -> &'t str {
    match text.strip_prefix(pre) {
        Some(rest) => {
            let blank = &rest[..rest.len() - rest.trim_start().len()];
            let end = pre.len() + blank.rfind('\n').map_or(0, |at| at + 1);
            &text[start..end]
        }
        None => &pre[start..],
    }
}

fn projected(raw: &str, org: bool) -> DocBlock {
    let mut block = DocBlock::new(raw);
    block.set_org(org);
    block
}

fn walk(blocks: &[DocBlock], depth: usize, under_header: bool, r: Reader, out: &mut Vec<Block>) {
    for block in blocks {
        out.push(Block {
            depth,
            body: whole_row(block, r),
            under_header,
        });
        walk(&block.children, depth + 1, under_header, r, out);
    }
}

fn whole_row(block: &DocBlock, r: Reader) -> Vec<Atom> {
    atoms(block.raw(), &block_sites(block, r), 0..block.raw().len())
}

/// One header entry per page property of `block`, whose raw holds them.
fn header_rows<'p>(
    block: &DocBlock,
    r: Reader,
    properties: impl Iterator<Item = &'p Property>,
) -> Vec<Header> {
    let raw = block.raw();
    let sites = block_sites(block, r);
    let mut rows = Vec::new();
    for p in properties {
        let line = p.line.0..p.line.0 + p.line.slice(raw).trim_end_matches(['\n', '\r']).len();
        let key = crate::doc::property_key_norm(&p.key);
        let form = match (r.org, p.directive) {
            (false, _) => NativeForm::Markdown,
            (true, true) => NativeForm::Directive,
            (true, false) => NativeForm::Drawer,
        };
        let value = p.value_range.0..p.value_range.1;
        let (role, text) = if key == "title" {
            (HeaderRole::Title, title_row(raw, &sites, line, value))
        } else {
            let text = atoms(raw, &sites, line);
            let aliases = crate::page_header::is_alias_key(&key);
            match aliases && text_has(&text, SiteKind::PropertyValue) {
                true => (HeaderRole::Aliases, as_alias_names(text)),
                false if aliases => (HeaderRole::Other, text),
                false => (HeaderRole::Property, text),
            }
        };
        rows.push(Header {
            role,
            form,
            key,
            text,
        });
    }
    rows
}

fn text_has(text: &[Atom], kind: SiteKind) -> bool {
    text.iter()
        .any(|a| matches!(a, Atom::Site(s) if s.kind == kind))
}

/// An alias header's members are the model's alias names.
fn as_alias_names(text: Vec<Atom>) -> Vec<Atom> {
    text.into_iter()
        .map(|atom| match atom {
            Atom::Site(s) if s.kind == SiteKind::PropertyValue => Atom::Site(Site {
                kind: SiteKind::AliasName,
                ..s
            }),
            atom => atom,
        })
        .collect()
}

/// A `title` entry: the value is one title atom whose parts are its text and
/// the sites inside it (Lean `Atom.title`, `titleCoherent`).
fn title_row(
    raw: &str,
    sites: &[(Range<usize>, Site)],
    line: Range<usize>,
    value: Range<usize>,
) -> Vec<Atom> {
    let parts = atoms(raw, sites, value.clone())
        .into_iter()
        .map(|atom| match atom {
            Atom::Site(s) => Part::Site(s),
            Atom::Text(t) => Part::Text(t),
            Atom::Title(s, _) => Part::Text(s.native),
        })
        .collect();
    let native = raw[value.clone()].to_owned();
    let whole = Site {
        kind: SiteKind::Title,
        context: Context::Ordinary,
        name: native.clone(),
        slot: 0..native.len(),
        native,
        within: None,
        inner: Vec::new(),
    };
    super::compact(vec![
        Atom::Text(raw[line.start..value.start].to_owned()),
        Atom::Title(whole, parts),
        Atom::Text(raw[value.end..line.end].to_owned()),
    ])
}

/// The atoms of `range` of `raw`: its text with the outermost `sites` (in
/// source order, as [`block_sites`] gives them) that lie in it. Visits only
/// the sites that start in `range`.
fn atoms(raw: &str, sites: &[(Range<usize>, Site)], range: Range<usize>) -> Vec<Atom> {
    let mut out = Vec::new();
    let mut at = range.start;
    let first = sites.partition_point(|(r, _)| r.start < range.start);
    for (r, s) in sites[first..]
        .iter()
        .take_while(|(r, _)| r.start < range.end)
    {
        super::step();
        // A site that ends past `range` belongs to no atom of it; one that
        // starts inside the previous one overlaps it (no nesting the parser
        // reports).
        if r.start < at || range.end < r.end {
            continue;
        }
        out.push(Atom::Text(raw[at..r.start].to_owned()));
        out.push(Atom::Site(s.clone()));
        at = r.end;
    }
    out.push(Atom::Text(raw[at..range.end].to_owned()));
    super::compact(out)
}

/// Every site of `block` with its range in the block's raw, the outermost in
/// source order, each holding the sites inside it ([`nest`]).
fn block_sites(block: &DocBlock, r: Reader) -> Vec<(Range<usize>, Site)> {
    let raw = block.raw();
    let projection = block.projection();
    let source = projection.reference_source();
    let mut sites = reference_sites(raw, source, &projection.regions, r);
    if raw.contains("{{") {
        query_sites(raw, &projection.regions, r.org, &mut sites);
    }
    if raw.contains("{{") || (r.org && raw.contains("[[")) {
        for target in crate::reference_evidence::rename_targets(raw, r.org) {
            target_sites(raw, target, r, &mut sites);
        }
    }
    sites.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
    // The evidence names a nested link once per level, with one range.
    sites.dedup();
    nest(sites)
}

/// `sites` in source order (an enclosing site first) as a forest: each site
/// inside another (its range within the other's) is that site's inner site,
/// its range made relative. One pass with a stack of the open sites. The
/// link the parser also reports for a tag `#[[name]]` (its `[[name]]`, under
/// the tag's name) is how the tag is written, not a site of its own.
fn nest(sites: Vec<(Range<usize>, Site)>) -> Vec<(Range<usize>, Site)> {
    fn close(open: &mut Vec<(Range<usize>, Site)>, out: &mut Vec<(Range<usize>, Site)>) {
        let (r, s) = open.pop().expect("an open site");
        match open.last_mut() {
            Some((p, parent)) => {
                let at = r.start - p.start..r.end - p.start;
                let encodes = parent.kind == SiteKind::Tag
                    && at == (1..parent.native.len())
                    && s.name == parent.name;
                if !encodes {
                    parent.inner.push((at, s));
                }
            }
            None => out.push((r, s)),
        }
    }
    let (mut out, mut open) = (Vec::new(), Vec::new());
    for (r, s) in sites {
        while open
            .last()
            .is_some_and(|(p, _): &(Range<usize>, Site)| p.end < r.end || p.end <= r.start)
        {
            close(&mut open, &mut out);
        }
        open.push((r, s));
    }
    while !open.is_empty() {
        close(&mut open, &mut out);
    }
    out
}

/// A level of a nested link: its `[[…]]` range in `node` around the name
/// range `body`.
fn level(body: &Range<usize>) -> (Range<usize>, Range<usize>) {
    (body.start - 2..body.end + 2, 2..body.len() + 2)
}

/// Where a link node writes its name `name`: `[[name]]`, Org `[[name][…]]`,
/// a label `][name]]`, else the name itself.
fn link_slot(node: &str, name: &str) -> Range<usize> {
    let found = |spelled: String| node.find(&spelled);
    let at = found(format!("[[{name}]]"))
        .or_else(|| found(format!("[[{name}][")))
        .map(|at| at + 2)
        .or_else(|| found(format!("][{name}]]")).map(|at| at + 2))
        .or_else(|| found(name.to_owned()))
        .unwrap_or(0);
    at..(at + name.len()).min(node.len())
}

/// The sites the reference evidence reports.
fn reference_sites(
    raw: &str,
    source: crate::reference_evidence::ReferenceSource<'_>,
    regions: &BlockRegions,
    r: Reader,
) -> Vec<(Range<usize>, Site)> {
    // Which evidence items are linkable property values, by identity: one
    // set lookup per reference, not a search of the list.
    let values: std::collections::HashSet<*const ProjectedPageRef> =
        crate::reference_evidence::linkable_property_references(source, regions)
            .map(std::ptr::from_ref)
            .collect();
    let mut sites: Vec<(Range<usize>, Site)> = Vec::new();
    let mut nested: Option<Range<usize>> = None;
    for p in source.explicit {
        let node = &raw[p.range.clone()];
        if p.rule == "explicit_nested_link" {
            // The evidence names every level of the node, the enclosing one
            // included (`nested_reference_names`: each `[[…]]` body), once
            // per name: each level is a link site named by its body.
            if nested.as_ref() == Some(&p.range) {
                continue;
            }
            let levels = crate::block_regions::nested_reference_ranges(node);
            if !levels.is_empty() {
                nested = Some(p.range.clone());
                for body in &levels {
                    let (at, slot) = level(body);
                    let at = p.range.start + at.start..p.range.start + at.end;
                    let name = Some(&node[body.clone()]);
                    sites.push((at.clone(), site(SiteKind::Link, &raw[at], slot, name)));
                }
                continue;
            }
        }
        if p.rule == "explicit_link" && r.org && node.starts_with("[[file:") {
            // An Org file link: its label names the page (a site of its own);
            // its path's stem is a rewrite target (`target_sites`).
            if let Some(split) = node[2..node.len() - 2].find("][") {
                let at = p.range.start + 2 + split + 2..p.range.end - 2;
                let label = &raw[at.clone()];
                let s = site(SiteKind::Link, label, 0..label.len(), Some(&p.name));
                sites.push((at, s));
            }
            continue;
        }
        super::step();
        let value = values.contains(&std::ptr::from_ref(p));
        let (kind, slot) = match p.rule {
            "explicit_property_key" => (SiteKind::PropertyKey, 0..node.len()),
            "explicit_task_marker" => (SiteKind::Task, 0..node.len()),
            "explicit_priority" => (SiteKind::Priority, 0..node.len()),
            // A tag `#[[name]]`, or `#[[a [[b]]]]`, which the parser names by
            // its whole bracketed text.
            "explicit_tag" if node.starts_with("#[[") && node[1..] != p.name => {
                (SiteKind::Tag, 3..node.len() - 2)
            }
            "explicit_tag" if node.starts_with('#') => (SiteKind::Tag, 1..node.len()),
            "explicit_tag" => (SiteKind::Tag, 0..node.len()),
            "implicit_linkable_property" => (SiteKind::PropertyValue, 0..node.len()),
            "explicit_link" | "explicit_nested_link" => (SiteKind::Link, link_slot(node, &p.name)),
            "explicit_embed" => (SiteKind::Embed, link_slot(node, &p.name)),
            _ => continue,
        };
        let kind = if value && kind != SiteKind::PropertyKey {
            SiteKind::PropertyValue
        } else {
            kind
        };
        // A property key's name is its authored spelling (the model reads
        // `Old::` as `Old`); every other site's is the parser's name.
        let name = (kind != SiteKind::PropertyKey).then_some(p.name.as_str());
        let mut s = site(kind, node, slot, name);
        if p.rule == "explicit_tag" && s.context == Context::Ordinary {
            s.context = if node.starts_with('#') {
                Context::MdTag
            } else {
                Context::OrgHeadlineTag
            };
        }
        sites.push((p.range.clone(), s));
    }
    sites
}

/// The page-name arguments of each query macro in `raw` (D-20261031-CMQ): what
/// the OG DSL parse reads as a page name, and each TQL page-name literal. A
/// macro in code (in the text or in a property's value), and a macro the query
/// readers do not take as written, has none.
fn query_sites(raw: &str, regions: &BlockRegions, org: bool, out: &mut Vec<(Range<usize>, Site)>) {
    use crate::query::macro_text::{query_macro_extents, split_trailing_map, FormFamily};
    let mut value_code = crate::refs::rename::PropertyValueCode::new(raw, regions, org);
    for extent in query_macro_extents(raw) {
        if regions.literal_at(extent.start) || value_code.contains(extent.start) {
            continue;
        }
        let family = FormFamily::for_macro_name(&extent.name);
        let argument_end = extent.end - 2;
        let argument_start = argument_end - extent.argument.len();
        let (form, _) = split_trailing_map(&extent.argument, family);
        let form_start =
            argument_start + extent.argument.len() - extent.argument.trim_start().len();
        if raw.get(argument_start..argument_end) != Some(extent.argument.as_str())
            || !raw[form_start..].starts_with(form.as_str())
            || !crate::query::query_source_within_limit(&form)
            || !crate::query::query_nesting_within_limit(&form)
        {
            continue;
        }
        let names = match family {
            FormFamily::Tql => crate::query::tql::page_name_literals(&form),
            FormFamily::Edn if crate::query::is_advanced(&form) => Vec::new(),
            FormFamily::Edn => crate::query::og::page_name_arguments(&form),
        };
        let within = Arc::new(QueryMacro {
            name: extent.name,
            argument: extent.argument,
        });
        for (range, name) in names {
            let at = form_start + range.start..form_start + range.end;
            let native = &raw[at.clone()];
            let mut s = site(SiteKind::Query, native, 0..native.len(), Some(&name));
            if s.context == Context::Ordinary {
                s.context = Context::QueryArg;
            }
            s.within = Some((Arc::clone(&within), at.start - argument_start));
            out.push((at, s));
        }
    }
}

/// The sites of one rewrite target the evidence walk reports.
fn target_sites(raw: &str, target: RewriteTarget, r: Reader, out: &mut Vec<(Range<usize>, Site)>) {
    match target {
        RewriteTarget::MacroLink(p) => {
            let node = &raw[p.range.clone()];
            let mut push = |at: Range<usize>, slot: Range<usize>, name: Option<&str>| {
                let mut s = site(SiteKind::MacroArg, &raw[at.clone()], slot, name);
                if s.context == Context::Ordinary {
                    s.context = Context::MacroArgument;
                }
                out.push((at, s));
            };
            let levels = match p.rule {
                "explicit_nested_link" => crate::block_regions::nested_reference_ranges(node),
                _ => Vec::new(),
            };
            if levels.is_empty() {
                push(p.range.clone(), link_slot(node, &p.name), Some(&p.name));
            }
            // Every level of a nested link (the walk names each, once per
            // name; [`block_sites`] keeps one of each).
            for body in &levels {
                let (at, slot) = level(body);
                let at = p.range.start + at.start..p.range.start + at.end;
                push(at, slot, Some(&node[body.clone()]));
            }
        }
        RewriteTarget::FileLink(range) => {
            // `[[file:dir/stem.ext]]` or `[[file:…][label]]`: the stem of the
            // path's last component names a page through the file-name codec.
            let inner = range.start + 2..range.end.saturating_sub(2);
            let Some(body) = raw.get(inner.clone()).and_then(|t| t.strip_prefix("file:")) else {
                return;
            };
            let path = &body[..body.find("][").unwrap_or(body.len())];
            let slash = path.rfind('/').map_or(0, |at| at + 1);
            let file = &path[slash..];
            let stem = &file[..file.rfind('.').unwrap_or(file.len())];
            if stem.is_empty() {
                return;
            }
            let start = inner.start + "file:".len() + slash;
            let name = crate::model::decode_page_name(stem, r.names);
            let s = site(SiteKind::OrgFile, stem, 0..stem.len(), Some(&name));
            out.push((start..start + stem.len(), s));
        }
    }
}

fn site(kind: SiteKind, native: &str, slot: Range<usize>, name: Option<&str>) -> Site {
    Site {
        kind,
        // A site whose spelling spans a line break cannot be respelled in
        // place: the line's layout is the writer's (Lean `splitLine`).
        context: if native.contains('\n') {
            Context::SplitLine
        } else {
            Context::Ordinary
        },
        name: name.unwrap_or(native).to_owned(),
        native: native.to_owned(),
        slot,
        within: None,
        inner: Vec::new(),
    }
}
