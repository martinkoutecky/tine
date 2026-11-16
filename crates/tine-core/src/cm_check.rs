//! The conceptual model's P2 check for a rename or merge (model r6,
//! D-20261116-CM536): what each changed file must read as, computed from how
//! the input read, and the comparison with how the output reads.
//!
//! Translation validation: [`expected`] is a transcription of the Lean
//! `expectedContent` (`cm-cut6/lean/src/CutObserve.lean`), function by
//! function; every function names its Lean source. It works on observations
//! only and shares no code with the text rewriter or the merge writer; the
//! name-level rules (what a name becomes, the day pin, the key and member
//! spellings) are the single functions in `refs.rs` / `refs_rename.rs` that
//! the writer also uses, so this check does not catch an error in them (the
//! witness-matrix tests do).
//!
//! The interface: [`observe`] (text to [`Content`], given the graph's
//! file-name codec, which reads an Org `file:` link's stem; [`observe_parsed`]
//! when the caller holds the parse), [`expected`],
//! [`matches`], [`expected_path`] and [`skip_file`]. Pure: no I/O, no graph
//! state.

use std::borrow::Cow;
use std::collections::HashMap;

use crate::config::FileNameFormat;
use crate::model::Format;
use crate::query::macro_text::FormFamily;

mod observe;
pub use observe::{observe, observe_parsed};

#[cfg(test)]
mod site_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod witness_tests;

/// Lean `SiteKind`, all of them: every reference rule the evidence walk names
/// and every rewrite target the rename writer respells has one (pinned by
/// `site_tests::every_rewriter_rule_has_an_observed_kind`). `Query` is a
/// query macro's page-name argument, `MacroArg` a page link inside another
/// macro's argument, `OrgFile` the stem of an Org `file:` link.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SiteKind {
    Link,
    Tag,
    Embed,
    PropertyKey,
    PropertyValue,
    Task,
    Priority,
    Query,
    MacroArg,
    OrgFile,
    Title,
    AliasName,
}

/// Lean `Context`, the contexts [`observe`] produces.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Context {
    Ordinary,
    MdTag,
    OrgHeadlineTag,
    SplitLine,
    QueryArg,
    MacroArgument,
}

/// Lean `Site`. `slot` is where the name is written inside `native`; like the
/// Lean `start`/`stop` it is a coordinate, erased before any comparison, and
/// so is `within`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Site {
    pub kind: SiteKind,
    pub context: Context,
    pub name: String,
    pub native: String,
    pub slot: std::ops::Range<usize>,
    /// For a `Query` site, the macro its token is written in: whether a
    /// spelling is legal there depends on the whole macro (Lean
    /// `Observations.spell` sees the site; this is the part of the file the
    /// spelling rule reads).
    pub within: Option<Box<QueryMacro>>,
}

/// The query macro around a `Query` site: its name, its argument as written,
/// and where the site's token starts in that argument.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct QueryMacro {
    pub name: String,
    pub argument: String,
    pub at: usize,
}

/// One part of a title: Lean `String ⊕ Site`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Site(Site),
}

/// Lean `Atom`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Atom {
    Text(String),
    Site(Site),
    Title(Site, Vec<Part>),
}

/// Lean `HeaderRole`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HeaderRole {
    Title,
    Property,
    Aliases,
    Other,
}

/// Lean `NativeForm`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NativeForm {
    Markdown,
    Directive,
    Drawer,
}

/// Lean `Header`, with its key (see [`header_key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Header {
    pub role: HeaderRole,
    pub form: NativeForm,
    /// The parser's property key, normalized as the parser compares keys
    /// (`doc::property_key_norm`).
    pub key: String,
    pub text: Vec<Atom>,
}

/// Lean `Block`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Block {
    pub depth: usize,
    pub body: Vec<Atom>,
    pub under_header: bool,
}

/// Lean `Content`: one file as the parser reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Content {
    pub header: Vec<Header>,
    pub blocks: Vec<Block>,
}

/// Lean `Semantic`: what P2 compares.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Semantic {
    pub header: Vec<Header>,
    pub alias_names: Vec<String>,
    pub alias_declarations: usize,
    pub blocks: Vec<Block>,
}

/// The rename every site is checked against: Lean `old` and
/// `Plan.destination`, with the graph's journal format (the day pin) and
/// file-name codec (Lean `Observations.encode`). One name map serves every
/// site kind, query arguments included (manager ruling Q-e).
pub struct Rename<'a> {
    destination: &'a str,
    pages: HashMap<String, String>,
    days: Option<&'a crate::date::JournalFormat>,
    names: FileNameFormat,
}

impl<'a> Rename<'a> {
    pub fn new(
        old: &'a str,
        destination: &'a str,
        days: Option<&'a crate::date::JournalFormat>,
        names: FileNameFormat,
    ) -> Self {
        let pages = HashMap::from([(crate::refs::page_key(old), destination.to_owned())]);
        Self {
            destination,
            pages,
            days,
            names,
        }
    }

    /// Lean `nameMap` / `mappedName`: a day stays, else `nsSubst`; the one
    /// answer is `refs::renamed_site_name`.
    fn mapped_name<'s>(&'s self, name: &'s str) -> Cow<'s, str> {
        crate::refs::renamed_site_name(name, &self.pages, self.days).unwrap_or(Cow::Borrowed(name))
    }

    /// Lean `Date13.target P D [] s`: a day, else the page key.
    fn identity(&self, name: &str) -> Result<crate::date::JournalDate, String> {
        self.days
            .and_then(|days| days.parse(name))
            .ok_or_else(|| crate::refs::page_key(name))
    }

    /// Lean `titlePrefix`: bytes of the title's spelling that the old name
    /// (or its namespace prefix) occupies; 0 for a day or no match.
    fn title_prefix(&self, name: &str) -> usize {
        match crate::refs::renamed_site_name(name, &self.pages, self.days) {
            // `renamed_page_name` is `destination ++ name[prefix..]`.
            Some(new) => name.len() - (new.len() - self.destination.len()),
            None => 0,
        }
    }
}

/// What a check is asked about one file the plan keeps (Lean `expectedContent`).
pub enum Expect<'a> {
    /// A file that is neither the merge's source nor its survivor; `skip` is
    /// [`skip_file`].
    File { content: &'a Content, skip: bool },
    /// A merge's survivor, with the source it absorbs.
    Survivor {
        source: &'a Content,
        target: &'a Content,
    },
}

/// The model's spelling of `name` at `site` (Lean `Observations.spell`), from
/// the writer's own spelling functions; `None` is unspellable.
///
/// A query argument takes the query's own spelling, and it is unspellable
/// when the macro with that token respelled is no longer one macro the parser
/// reads (the writer's `macro_safe` and `recognizable_macro`); a link in
/// another macro's argument is unspellable with a `}` (it would end the
/// macro); an Org file stem is the name through the graph's file-name codec.
fn spell(site: &Site, name: &str, names: FileNameFormat) -> Option<String> {
    let replaced = || {
        let mut out = site.native.clone();
        out.replace_range(site.slot.clone(), name);
        out
    };
    let tag = || {
        if site.native.starts_with("#[[") {
            replaced()
        } else {
            crate::refs::rename::tag_for(name)
        }
    };
    match (site.context, site.kind) {
        (Context::SplitLine, _) => None,
        (Context::OrgHeadlineTag, _) => crate::refs::rename::headline_tag_ok(name).then(replaced),
        (_, SiteKind::Query) => query_spelling(site, name),
        (_, SiteKind::MacroArg) => (!name.contains('}')).then(replaced),
        (_, SiteKind::OrgFile) => Some(crate::model::encode_page_name(name, names)),
        (_, SiteKind::Tag) => Some(tag()),
        (_, SiteKind::PropertyValue | SiteKind::AliasName) => {
            Some(if site.native.starts_with("[[") {
                replaced()
            } else if site.native.starts_with('#') {
                tag()
            } else {
                crate::refs::rename::member_spelling(name)
            })
        }
        _ => Some(replaced()),
    }
}

/// A query argument's token respelled for `name`, if its macro still reads
/// as that macro.
fn query_spelling(site: &Site, name: &str) -> Option<String> {
    use crate::query::macro_text::{macro_safe, recognizable_macro};
    let q = site.within.as_deref()?;
    let family = FormFamily::for_macro_name(&q.name);
    let token = match family {
        FormFamily::Tql => crate::query::print::sql_string(name),
        FormFamily::Edn => crate::refs::rename::og_token_spelling(&site.native, name),
    };
    let mut argument = q.argument.clone();
    argument.replace_range(q.at..q.at + site.native.len(), &token);
    (macro_safe(&argument, family).is_ok() && recognizable_macro(&q.name, &argument).is_ok())
        .then_some(token)
}

/// Lean `siteResult` (the site only; dispositions are reports, not P2).
fn site_result(rename: &Rename, skip: bool, s: &Site) -> Site {
    let n = rename.mapped_name(&s.name);
    // A day maps to itself (`renamed_site_name`), so `n = s.name` covers it.
    if matches!(s.kind, SiteKind::Task | SiteKind::Priority) || n == s.name || skip {
        return s.clone();
    }
    let k = if s.kind == SiteKind::PropertyKey {
        crate::refs::rename::property_key_spelling(&n)
    } else {
        Some(n.into_owned())
    };
    match k.and_then(|k| spell(s, &k, rename.names).map(|raw| (k, raw))) {
        Some((k, raw)) => Site {
            slot: 0..raw.len(),
            name: k,
            native: raw,
            ..s.clone()
        },
        None => s.clone(),
    }
}

/// Lean `compact`: adjacent text atoms merge; an empty text atom goes.
fn compact(text: Vec<Atom>) -> Vec<Atom> {
    let mut out: Vec<Atom> = Vec::with_capacity(text.len());
    for atom in text {
        match (out.last_mut(), atom) {
            (_, Atom::Text(t)) if t.is_empty() => {}
            (Some(Atom::Text(last)), Atom::Text(t)) => last.push_str(&t),
            (_, atom) => out.push(atom),
        }
    }
    out
}

/// Lean `compactParts`.
fn compact_parts(parts: Vec<Part>) -> Vec<Part> {
    compact(parts.into_iter().map(part_atom).collect())
        .into_iter()
        .map(|atom| match atom {
            Atom::Text(t) => Part::Text(t),
            Atom::Site(s) => Part::Site(s),
            Atom::Title(s, _) => Part::Text(s.native),
        })
        .collect()
}

/// Lean `partText`.
fn part_text(part: &Part) -> &str {
    match part {
        Part::Text(t) => t,
        Part::Site(s) => &s.native,
    }
}

/// Lean `partAtom`.
fn part_atom(part: Part) -> Atom {
    match part {
        Part::Text(t) => Atom::Text(t),
        Part::Site(s) => Atom::Site(s),
    }
}

/// Lean `dropParts`: drop the first `n` bytes of the parts.
fn drop_parts(mut n: usize, parts: &[Part]) -> Vec<Part> {
    let mut out = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        if n == 0 {
            out.extend_from_slice(&parts[i..]);
            break;
        }
        let text = part_text(part);
        if text.len() <= n {
            n -= text.len();
        } else {
            // Lean `byteString (drop n)`: the prefix ends inside this part.
            out.push(Part::Text(text[n..].to_owned()));
            out.extend_from_slice(&parts[i + 1..]);
            break;
        }
    }
    out
}

/// Lean `titleParts`.
fn title_parts(rename: &Rename, skip: bool, s: &Site, parts: &[Part]) -> Vec<Part> {
    if skip {
        return parts.to_vec();
    }
    let n = rename.title_prefix(&s.name);
    let head = (n != 0).then(|| Part::Text(rename.destination.to_owned()));
    head.into_iter()
        .chain(drop_parts(n, parts).into_iter().map(|part| match part {
            Part::Text(t) => Part::Text(t),
            Part::Site(t) => Part::Site(site_result(rename, false, &t)),
        }))
        .collect()
}

/// Lean `mapAtom`.
fn map_atom(rename: &Rename, skip: bool, atom: &Atom) -> Atom {
    match atom {
        Atom::Text(t) => Atom::Text(t.clone()),
        Atom::Site(s) => Atom::Site(site_result(rename, skip, s)),
        Atom::Title(s, parts) => {
            let parts = compact_parts(title_parts(rename, skip, s, parts));
            let raw: String = parts.iter().map(part_text).collect();
            let whole = Site {
                slot: 0..raw.len(),
                name: raw.clone(),
                native: raw,
                ..s.clone()
            };
            Atom::Title(whole, parts)
        }
    }
}

/// Lean `mapContent` (with `mapHeader` and `mapBody`): Lean `transformed`
/// once the caller has decided `skip`.
fn map_content(rename: &Rename, skip: bool, c: &Content) -> Content {
    let map = |text: &[Atom]| text.iter().map(|a| map_atom(rename, skip, a)).collect();
    Content {
        header: c
            .header
            .iter()
            .map(|h| {
                let text: Vec<Atom> = map(&h.text);
                // A respelled key site renames the entry's key.
                let key = text_sites(&text)
                    .find(|s| s.kind == SiteKind::PropertyKey)
                    .map_or_else(|| h.key.clone(), |s| crate::doc::property_key_norm(&s.name));
                Header {
                    key,
                    text,
                    ..h.clone()
                }
            })
            .collect(),
        blocks: c
            .blocks
            .iter()
            .map(|b| Block {
                body: map(&b.body),
                ..b.clone()
            })
            .collect(),
    }
}

/// Lean `eraseSite`.
fn erase_site(s: &Site) -> Site {
    Site {
        slot: 0..0,
        within: None,
        ..s.clone()
    }
}

/// Lean `eraseAtom`.
fn erase_atom(atom: &Atom) -> Atom {
    match atom {
        Atom::Text(t) => Atom::Text(t.clone()),
        Atom::Site(s) => Atom::Site(erase_site(s)),
        Atom::Title(s, parts) => Atom::Title(
            erase_site(s),
            parts
                .iter()
                .map(|p| match p {
                    Part::Text(t) => Part::Text(t.clone()),
                    Part::Site(t) => Part::Site(erase_site(t)),
                })
                .collect(),
        ),
    }
}

/// Lean `eraseHeader` / `eraseBlock` (with `normalizeBody`): both compact.
fn erase_text(text: &[Atom]) -> Vec<Atom> {
    compact(text.iter().map(erase_atom).collect())
}

/// Lean `atomSites` / `textSites`.
fn text_sites(text: &[Atom]) -> impl Iterator<Item = &Site> {
    text.iter().flat_map(|atom| {
        let (whole, parts): (Option<&Site>, &[Part]) = match atom {
            Atom::Text(_) => (None, &[]),
            Atom::Site(s) => (Some(s), &[]),
            Atom::Title(s, parts) => (Some(s), parts),
        };
        whole
            .into_iter()
            .chain(parts.iter().filter_map(|p| match p {
                Part::Site(s) => Some(s),
                Part::Text(_) => None,
            }))
    })
}

/// Lean `textString`.
fn text_string(text: &[Atom]) -> String {
    text.iter()
        .map(|atom| match atom {
            Atom::Text(t) => t.as_str(),
            Atom::Site(s) | Atom::Title(s, _) => s.native.as_str(),
        })
        .collect()
}

/// Lean `headerKey`: the entry's property key as the parser reads it, which
/// follows the model text (O6, "keys are compared as the parser reads them")
/// and the corrected Lean (manager ruling Q-c, 2026-11-16). The r6 Lean takes
/// the first property-key site's name, which is "" for every key that has no
/// site (an OG built-in key such as `icon`), so two such entries would clash.
/// A renamed key site renames the key ([`map_content`]).
fn header_key(h: &Header) -> &str {
    &h.key
}

/// Whether an aliases header declares a member (Lean, inside `semantic`).
fn declares_alias(h: &Header) -> bool {
    h.role == HeaderRole::Aliases && text_sites(&h.text).any(|s| s.kind == SiteKind::AliasName)
}

/// Lean `meaningfulHeader`.
fn meaningful_header(h: &Header) -> bool {
    let text: String = text_string(&h.text)
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    !text.is_empty() && text != "-" && text != "*"
}

/// Lean `semantic` (with Lean `aliases`).
fn semantic(c: &Content) -> Semantic {
    Semantic {
        header: c
            .header
            .iter()
            .filter(|h| meaningful_header(h) && !declares_alias(h))
            .map(|h| Header {
                text: erase_text(&h.text),
                ..h.clone()
            })
            .collect(),
        alias_names: c
            .header
            .iter()
            .filter(|h| h.role == HeaderRole::Aliases)
            .flat_map(|h| text_sites(&h.text))
            .filter(|s| s.kind == SiteKind::AliasName)
            .map(|s| s.name.clone())
            .collect(),
        alias_declarations: c.header.iter().filter(|h| declares_alias(h)).count(),
        blocks: c
            .blocks
            .iter()
            .map(|b| Block {
                body: erase_text(&b.body),
                ..b.clone()
            })
            .collect(),
    }
}

/// Lean `uniqueNames`: one name per reading identity, the first kept.
fn unique_names(rename: &Rename, names: Vec<String>) -> Vec<String> {
    let mut kept: Vec<String> = Vec::new();
    for n in names {
        if !kept
            .iter()
            .any(|k| rename.identity(k) == rename.identity(&n))
        {
            kept.push(n);
        }
    }
    kept
}

/// Lean `allocate`: a property whose key is not yet taken joins and takes
/// its key; every other header entry is left over. Input order kept.
fn allocate(mut keys: Vec<String>, headers: &[Header]) -> (Vec<Header>, Vec<Header>) {
    let (mut joined, mut left) = (Vec::new(), Vec::new());
    for h in headers {
        if h.role == HeaderRole::Property && !keys.iter().any(|k| k == header_key(h)) {
            keys.push(header_key(h).to_owned());
            joined.push(h.clone());
        } else {
            left.push(h.clone());
        }
    }
    (joined, left)
}

/// Lean `demoteAtom`.
fn demote_atom(atom: &Atom) -> Vec<Atom> {
    match atom {
        Atom::Site(s) if s.kind == SiteKind::Title => vec![Atom::Text(s.native.clone())],
        Atom::Title(_, parts) => parts.iter().cloned().map(part_atom).collect(),
        atom => vec![atom.clone()],
    }
}

/// Lean `leftoverPayload`.
fn leftover_payload(headers: &[Header]) -> Vec<Atom> {
    let mut out = Vec::new();
    for (i, h) in headers.iter().enumerate() {
        if i > 0 {
            out.push(Atom::Text("\n".into()));
        }
        out.extend(h.text.iter().flat_map(demote_atom));
    }
    out
}

/// Lean `leftoverBody`.
fn leftover_body(format: Format, headers: &[Header]) -> Vec<Atom> {
    let org = format == Format::Org;
    let mut body = vec![Atom::Text(
        if org { "\n#+BEGIN_QUOTE\n" } else { "\n" }.into(),
    )];
    body.extend(leftover_payload(headers));
    if org {
        body.push(Atom::Text("\n#+END_QUOTE".into()));
    }
    compact(body)
}

/// Lean `allocation`.
fn allocation(source: &Semantic, target: &Semantic) -> (Vec<Header>, Vec<Header>) {
    let keys = target
        .header
        .iter()
        .filter(|h| h.role == HeaderRole::Property)
        .map(|h| header_key(h).to_owned())
        .collect();
    allocate(keys, &source.header)
}

/// Lean `mergeContent` (with `middle` and `needsLeftover`).
fn merge_content(rename: &Rename, format: Format, source: Semantic, target: Semantic) -> Semantic {
    let (joined, left) = allocation(&source, &target);
    let names = unique_names(
        rename,
        target
            .alias_names
            .iter()
            .chain(&source.alias_names)
            .cloned()
            .collect(),
    );
    let middle = (!left.is_empty()).then(|| Block {
        depth: 0,
        body: leftover_body(format, &left),
        under_header: false,
    });
    Semantic {
        header: target.header.into_iter().chain(joined).collect(),
        alias_declarations: usize::from(!names.is_empty()),
        alias_names: names,
        blocks: target
            .blocks
            .into_iter()
            .chain(middle)
            .chain(source.blocks.into_iter().map(|b| Block {
                under_header: false,
                ..b
            }))
            .collect(),
    }
}

/// Lean `expectedContent`: what a file the plan keeps must read as. `format`
/// is the file's (for a survivor, the survivor's).
pub fn expected(rename: &Rename, format: Format, input: Expect<'_>) -> Semantic {
    // Lean `transformed`, then `semantic`; source and survivor never skip.
    let t = |content: &Content, skip| semantic(&map_content(rename, skip, content));
    match input {
        Expect::File { content, skip } => t(content, skip),
        Expect::Survivor { source, target } => {
            merge_content(rename, format, t(source, false), t(target, false))
        }
    }
}

/// Lean `ContentMatches` over `semantic (O.content h)`: exact, except that a
/// merge survivor's header (`survivor`, Lean `headerFlexible`) may be any
/// permutation of the expected one.
pub fn matches(expected: &Semantic, observed: &Content, survivor: bool) -> bool {
    let observed = semantic(observed);
    let rest = observed.alias_names == expected.alias_names
        && observed.alias_declarations == expected.alias_declarations
        && observed.blocks == expected.blocks;
    rest && if survivor {
        is_permutation(&observed.header, &expected.header)
    } else {
        observed.header == expected.header
    }
}

/// Lean `List.Perm`, O(n²) over a page's header entries.
fn is_permutation(a: &[Header], b: &[Header]) -> bool {
    let mut left: Vec<&Header> = b.iter().collect();
    a.len() == b.len()
        && a.iter().all(|h| match left.iter().position(|x| *x == h) {
            Some(at) => {
                left.swap_remove(at);
                true
            }
            None => false,
        })
}

/// Lean `expectedPath`: where a file the plan keeps must be. `path` is
/// graph-relative with `/`; `page` is the name the file's page has (Lean
/// `P.name f`); `fixed` is Lean's "a journal or the merge survivor".
pub fn expected_path(rename: &Rename, path: &str, page: &str, fixed: bool) -> String {
    let n = rename.mapped_name(page);
    if fixed || n == page {
        return path.to_owned();
    }
    // Lean `movedPath`: parent ++ encode n ++ extension.
    let stem_start = path.rfind('/').map_or(0, |at| at + 1);
    let extension = path[stem_start..]
        .rfind('.')
        .map_or("", |at| &path[stem_start + at..]);
    format!(
        "{}{}{extension}",
        &path[..stem_start],
        crate::model::encode_page_name(&n, rename.names)
    )
}

/// Lean `skipFile` for a file that is neither the merge's source nor its
/// survivor: it holds VCS conflict markers and does not move, so its sites
/// stay as they are.
pub fn skip_file(text: &str, format: Format, path: &str, expected_path: &str) -> bool {
    !crate::concord_queue::vcs_conflict_markers(text, format).is_empty() && path == expected_path
}
