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
use crate::reference_evidence::{RenameReadings, RewriteTarget, SlottedPageRef};
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
struct Reader<'a> {
    readings: &'a RenameReadings,
    org: bool,
    names: FileNameFormat,
}

/// Lean `Observations.read`: how the parser reads `text` as a page file of
/// `format`, coordinates kept for the expected side and erased by the
/// comparison. One whole-document parse; block projections and their optional
/// rename evidence share each distinct raw block's first parse. O(text bytes).
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

/// [`observe`] of a text whose whole-document parse `doc` the caller already
/// holds (the store parses a candidate once for this check and its save).
/// Everything is read from `doc`; the text argument is unused and goes with
/// the dead query override (manager ruling Q5, with the measurement harness).
pub fn observe_parsed(
    _text: &str,
    doc: &crate::doc::Document,
    format: Format,
    names: FileNameFormat,
) -> Content {
    observe_with_readings(doc, format, names, &RenameReadings::default())
}

/// Input observation sharing file-local reader evidence with the span writer.
pub(crate) fn observe_with_readings(
    doc: &crate::doc::Document,
    format: Format,
    names: FileNameFormat,
    readings: &RenameReadings,
) -> Content {
    let reader = Reader {
        readings,
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
        let prose = tine_outline::page_preamble::preamble_content_range_in(pre, org, &regions);
        if !properties.is_empty() {
            // The header is everything before the prose (the one boundary,
            // an Org drawer's `:END:` included).
            let block = projected(&pre[..prose.map_or(pre.len(), |p| p.0)], org);
            content.header = header_rows(&block, reader, properties.iter().copied());
            other_header_text(&block, reader, &regions, &mut content.header);
        }
        // The prose is the block the outline makes of it, exactly its text
        // (`page_properties::preamble_block`, the one range the page shows and
        // promotes): the newlines that end it separate it from the first
        // block (SPEC P4: line endings and blank lines are not observed).
        if let Some(prose) = prose {
            let block = projected(&pre[prose.0..prose.1], org);
            content.blocks.push(Block {
                depth: 0,
                body: whole_row(&block, reader),
                under_header: false,
            });
        }
    } else if let Some((first, rest)) = roots.split_first() {
        if crate::page_header::page_header_text(None, Some(first.raw())).is_some() {
            reader.readings.read(first);
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

fn projected(raw: &str, org: bool) -> DocBlock {
    let mut block = DocBlock::new(raw);
    block.set_org(org);
    block
}

fn walk(
    blocks: &[DocBlock],
    depth: usize,
    under_header: bool,
    r: Reader<'_>,
    out: &mut Vec<Block>,
) {
    for block in blocks {
        out.push(Block {
            depth,
            body: whole_row(block, r),
            under_header,
        });
        walk(&block.children, depth + 1, under_header, r, out);
    }
}

fn whole_row(block: &DocBlock, r: Reader<'_>) -> Vec<Atom> {
    atoms(block.raw(), &block_sites(block, r), 0..block.raw().len())
}

/// One header entry per page property of `block`, whose raw holds them.
fn header_rows<'p>(
    block: &DocBlock,
    r: Reader<'_>,
    properties: impl Iterator<Item = &'p Property>,
) -> Vec<Header> {
    let raw = block.raw();
    let sites = block_sites(block, r);
    let mut next_site = 0;
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
        let sites = ordered_sites(&sites, &mut next_site, &line);
        let (role, text) = if key == "title" {
            (HeaderRole::Title, title_row(raw, sites, line, value))
        } else {
            let text = atoms(raw, sites, line);
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

/// O6's "any other header text": each non-blank line of the header `block`
/// (the pre-block before its prose) that is neither a property line nor
/// inside an Org property drawer, e.g. Org prose before a directive (a
/// Markdown header holds only page-property lines). It has no key and stays
/// in line order among the property rows.
fn other_header_text(
    block: &DocBlock,
    r: Reader<'_>,
    regions: &BlockRegions,
    rows: &mut Vec<Header>,
) {
    let raw = block.raw();
    let mut properties = regions.page_properties().peekable();
    let mut drawers = regions.property_regions.iter().peekable();
    let mut property_rows = std::mem::take(rows).into_iter();
    let sites = std::cell::OnceCell::new();
    let mut next_site = 0;
    let mut at = 0;
    for line in raw.split_inclusive('\n') {
        super::step();
        // Both lists are parser-ordered. Each range and each property row
        // advances once; no search from the start and no Vec insertion.
        while properties.peek().is_some_and(|p| p.line.1 <= at) {
            super::step();
            properties.next();
            rows.extend(property_rows.next());
        }
        while drawers.peek().is_some_and(|g| g.1 <= at) {
            super::step();
            drawers.next();
        }
        // A Markdown property node's line that is no page property (a `#`
        // key, a folded `#+` line) is header text, never silently unread.
        let covered = properties.peek().is_some_and(|p| p.line.0 <= at)
            || (r.org && drawers.peek().is_some_and(|g| g.0 <= at));
        let text = line.trim_end_matches(['\n', '\r']);
        if !text.trim().is_empty() && !covered {
            let sites = sites.get_or_init(|| block_sites(block, r));
            let line = at..at + text.len();
            let sites = ordered_sites(sites, &mut next_site, &line);
            rows.push(Header {
                role: HeaderRole::Other,
                form: if r.org {
                    NativeForm::Directive
                } else {
                    NativeForm::Markdown
                },
                key: String::new(),
                text: atoms(raw, sites, line),
            });
        }
        at += line.len();
    }
    rows.extend(property_rows);
}

/// Sites starting in successive, ordered header lines, with a cursor rather
/// than a fresh search per property. Each site is visited at most once.
fn ordered_sites<'s>(
    sites: &'s [(Range<usize>, Site)],
    next: &mut usize,
    line: &Range<usize>,
) -> &'s [(Range<usize>, Site)] {
    while *next < sites.len() && sites[*next].0.start < line.start {
        super::step();
        *next += 1;
    }
    let first = *next;
    while *next < sites.len() && sites[*next].0.start < line.end {
        super::step();
        *next += 1;
    }
    &sites[first..*next]
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
        nested: false,
        rule: None,
        context: Context::Ordinary,
        name: native.clone(),
        slot: 0..native.len(),
        native,
        within: None,
        label: None,
        org: false,
        file_link: None,
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
fn block_sites(block: &DocBlock, r: Reader<'_>) -> Vec<(Range<usize>, Site)> {
    let raw = block.raw();
    let source = r.readings.read(block);
    let projection = block.projection();
    let mut sites = reference_sites(raw, &source.explicit, &projection.regions, r);
    if raw.contains("{{") {
        query_sites(raw, &projection.regions, r.org, &mut sites);
    }
    for target in &source.targets {
        target_sites(raw, target.clone(), r, &mut sites);
    }
    sites.sort_by_key(|(r, _)| (r.start, std::cmp::Reverse(r.end)));
    // The evidence names a nested link once per level, with one range.
    sites.dedup();
    for (_, site) in &mut sites {
        site.org = r.org;
    }
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
                parent.inner.push((at, s));
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

/// One observation of a file label, whether it is a reference/property
/// member or a macro argument. Org references leave the file stem as a
/// separate site; macro arguments observe the whole link node.
fn label_site(
    kind: SiteKind,
    raw: &str,
    p: &SlottedPageRef,
    r: Reader<'_>,
) -> Option<(Range<usize>, Site)> {
    let label = crate::refs::FileLabel::read(raw, p, r.org)?;
    let node = &raw[p.range.clone()];
    let extent = if r.org && kind != SiteKind::MacroArg {
        label.label.clone()
    } else {
        0..node.len()
    };
    let slot = label
        .run
        .as_ref()
        .map_or(0..0, |run| run.start - extent.start..run.end - extent.start);
    let mut s = site(kind, &node[extent.clone()], slot, Some(&p.name));
    s.nested = p.slot.nested;
    s.rule = Some(p.rule);
    s.label = Some(Arc::new(label));
    Some((extent, s))
}

/// The sites the reference evidence reports.
fn reference_sites(
    raw: &str,
    source: &[SlottedPageRef],
    regions: &BlockRegions,
    r: Reader<'_>,
) -> Vec<(Range<usize>, Site)> {
    let values: std::collections::HashSet<*const SlottedPageRef> =
        crate::reference_evidence::linkable_property_references_in(source.iter(), regions, |p| {
            &p.reference
        })
        .map(|(_, p)| std::ptr::from_ref(p))
        .collect();
    let mut sites: Vec<(Range<usize>, Site)> = Vec::new();
    for p in source {
        // A property member in a macro is observed as its macro argument.
        if p.slot.encoding || p.slot.in_macro {
            continue;
        }
        let node = &raw[p.range.clone()];
        let value = values.contains(&std::ptr::from_ref(p));
        if p.rule == "explicit_link" {
            let kind = if value {
                SiteKind::PropertyValue
            } else {
                SiteKind::Link
            };
            if let Some((extent, s)) = label_site(kind, raw, p, r) {
                sites.push((p.range.start + extent.start..p.range.start + extent.end, s));
                continue;
            }
        }
        super::step();
        let kind = match p.rule {
            "explicit_property_key" => SiteKind::PropertyKey,
            "explicit_task_marker" => SiteKind::Task,
            "explicit_priority" => SiteKind::Priority,
            "explicit_tag" => SiteKind::Tag,
            "implicit_linkable_property" => SiteKind::PropertyValue,
            "explicit_link" | "explicit_nested_link" => SiteKind::Link,
            "explicit_embed" => SiteKind::Embed,
            _ => continue,
        };
        let slot = p.slot.run().unwrap_or(&p.slot.extent);
        let slot = slot.start - p.range.start..slot.end - p.range.start;
        // An embed member keeps its native `{{embed [[..]]}}` form: its site
        // is the embed's, wherever it stands.
        let kind = if value && !matches!(kind, SiteKind::PropertyKey | SiteKind::Embed) {
            SiteKind::PropertyValue
        } else {
            kind
        };
        // A property key's name is its authored spelling (the model reads
        // `Old::` as `Old`); every other site's is the parser's name.
        let name = (kind != SiteKind::PropertyKey).then_some(p.name.as_str());
        let mut s = site(kind, node, slot, name);
        s.nested = p.slot.nested;
        s.rule = Some(p.rule);
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
    for query in crate::query::page_names::macro_page_names(raw, regions, org) {
        let argument_start = query.argument_start;
        let within = Arc::new(QueryMacro {
            name: query.name,
            argument: query.argument,
            tokens: (query.names.iter())
                .map(|(at, name)| {
                    (
                        at.start - argument_start,
                        raw[at.clone()].to_owned(),
                        name.clone(),
                    )
                })
                .collect(),
        });
        for (at, name) in query.names {
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
fn target_sites(
    raw: &str,
    target: RewriteTarget,
    r: Reader<'_>,
    out: &mut Vec<(Range<usize>, Site)>,
) {
    match target {
        RewriteTarget::MacroLink(p) => {
            let node = &raw[p.range.clone()];
            let slot = p.slot.run().unwrap_or(&p.slot.extent);
            let slot = slot.start - p.range.start..slot.end - p.range.start;
            let mut s = label_site(SiteKind::MacroArg, raw, &p, r)
                .map(|(_, site)| site)
                .unwrap_or_else(|| site(SiteKind::MacroArg, node, slot, Some(&p.name)));
            s.nested = p.slot.nested;
            s.rule = Some(p.rule);
            if s.context == Context::Ordinary {
                s.context = Context::MacroArgument;
            }
            out.push((p.reference.range, s));
        }
        RewriteTarget::FileLink(range) => {
            // The stem of the path's last component names a page through
            // the file-name codec.
            let Some(stem) = raw
                .get(range.clone())
                .and_then(crate::model::org_file_link_stem)
            else {
                return;
            };
            let at = range.start + stem.start..range.start + stem.end;
            let stem = &raw[at.clone()];
            let name = crate::model::decode_page_name(stem, r.names);
            let mut s = site(SiteKind::OrgFile, stem, 0..stem.len(), Some(&name));
            s.file_link = Some(Arc::from(&raw[range]));
            out.push((at, s));
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
        label: None,
        org: false,
        file_link: None,
        nested: false,
        rule: None,
        inner: Vec::new(),
    }
}
