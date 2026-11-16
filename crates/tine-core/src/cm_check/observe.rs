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
            let block = projected(preamble(text, pre, prose.0), org);
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
/// starts to the first outline block, so the blank lines before that block
/// stay (the parser's pre-block drops them). When `pre` is not a prefix of
/// `text` (a normalized line ending), the parser's own range.
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
    };
    super::compact(vec![
        Atom::Text(raw[line.start..value.start].to_owned()),
        Atom::Title(whole, parts),
        Atom::Text(raw[value.end..line.end].to_owned()),
    ])
}

/// The atoms of `range` of `raw`: its text with the `sites` there. A site
/// inside one already taken (a link that only encodes its enclosing tag) is
/// that site's.
fn atoms(raw: &str, sites: &[(Range<usize>, Site)], range: Range<usize>) -> Vec<Atom> {
    let mut out = Vec::new();
    let mut at = range.start;
    for (r, s) in sites {
        if r.start < at || r.start < range.start || range.end < r.end {
            continue;
        }
        out.push(Atom::Text(raw[at..r.start].to_owned()));
        out.push(Atom::Site(s.clone()));
        at = r.end;
    }
    out.push(Atom::Text(raw[at..range.end].to_owned()));
    super::compact(out)
}

/// Every site of `block`, with its range in the block's raw, in source order
/// (an enclosing site before the sites inside it).
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
    sites
}

/// The innermost name ranges of a nested link `node` (the parser names every
/// level; the writer respells the inner name and the outer one follows).
fn innermost(node: &str) -> Vec<Range<usize>> {
    let names = crate::block_regions::nested_reference_ranges(node);
    names
        .iter()
        .filter(|a| {
            !names
                .iter()
                .any(|b| b != *a && a.start <= b.start && b.end <= a.end)
        })
        .cloned()
        .collect()
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
    let values: Vec<&ProjectedPageRef> =
        crate::reference_evidence::linkable_property_references(source, regions).collect();
    let mut sites: Vec<(Range<usize>, Site)> = Vec::new();
    let mut nested: Option<Range<usize>> = None;
    for p in source.explicit {
        let node = &raw[p.range.clone()];
        if p.rule == "explicit_nested_link" {
            if nested.as_ref() == Some(&p.range) {
                continue;
            }
            nested = Some(p.range.clone());
            for inner in innermost(node) {
                let at = p.range.start + inner.start..p.range.start + inner.end;
                let s = site(SiteKind::Link, &raw[at.clone()], 0..inner.len(), None);
                sites.push((at, s));
            }
            continue;
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
        let value = values.iter().any(|v| std::ptr::eq(*v, p));
        let (kind, slot) = match p.rule {
            "explicit_property_key" => (SiteKind::PropertyKey, 0..node.len()),
            "explicit_task_marker" => (SiteKind::Task, 0..node.len()),
            "explicit_priority" => (SiteKind::Priority, 0..node.len()),
            "explicit_tag" if node.starts_with("#[[") => (SiteKind::Tag, 3..node.len() - 2),
            "explicit_tag" if node.starts_with('#') => (SiteKind::Tag, 1..node.len()),
            "explicit_tag" => (SiteKind::Tag, 0..node.len()),
            "implicit_linkable_property" => (SiteKind::PropertyValue, 0..node.len()),
            "explicit_link" => (SiteKind::Link, link_slot(node, &p.name)),
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
        for (range, name) in names {
            let at = form_start + range.start..form_start + range.end;
            let native = &raw[at.clone()];
            let mut s = site(SiteKind::Query, native, 0..native.len(), Some(&name));
            if s.context == Context::Ordinary {
                s.context = Context::QueryArg;
            }
            s.within = Some(Box::new(QueryMacro {
                name: extent.name.clone(),
                argument: extent.argument.clone(),
                at: at.start - argument_start,
            }));
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
            if p.rule == "explicit_nested_link" {
                for inner in innermost(node) {
                    let at = p.range.start + inner.start..p.range.start + inner.end;
                    push(at, 0..inner.len(), None);
                }
            } else {
                push(p.range.clone(), link_slot(node, &p.name), Some(&p.name));
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
    }
}
