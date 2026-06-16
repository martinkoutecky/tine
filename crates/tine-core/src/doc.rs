//! Single-file document model: parse a Logseq `.md` file into a tree of blocks
//! and serialize it back in Logseq-compatible form.
//!
//! Round-trip contract: for well-formed Logseq input (TAB per nesting level,
//! continuation lines = `<tabs>` + two spaces), `serialize(parse(x)) == x`.
//! For differently-indented input we canonicalize to TABs (Logseq itself
//! reformats on save, so this is acceptable — see plan "File fidelity").
//!
//! `raw` holds the full block body (first line + continuation/property lines,
//! dedented). Keeping it authoritative is what makes round-tripping safe; the
//! structured views (`properties`, `marker`, `collapsed`) are computed on top.

use serde::{Deserialize, Serialize};

/// Recognized task markers (leading keyword of a block).
pub const MARKERS: &[&str] = &[
    "TODO", "DOING", "DONE", "NOW", "LATER", "WAITING", "WAIT", "CANCELED", "CANCELLED",
    "IN-PROGRESS",
];

/// A parsed `.md` document: an optional page-property pre-block plus a forest
/// of blocks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct Document {
    /// Raw text of the region before the first bullet (page properties / free
    /// text), with the trailing blank separator removed. `None` if the file
    /// starts with a bullet.
    pub pre_block: Option<String>,
    pub roots: Vec<DocBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocBlock {
    /// Dedented block body: first line + continuation lines joined with `\n`.
    pub raw: String,
    pub children: Vec<DocBlock>,
    /// Stable identity assigned once when the block enters the in-memory cache
    /// (the persisted `id::` if any, else a generated uuid). It is the handle
    /// every surface (main view, sidebar, query result, ref) uses to address the
    /// block, and round-trips through save so it stays stable across edits. It is
    /// NOT part of block *content*, so it is excluded from equality — otherwise
    /// the conflict guard (`parse(disk) == cached`) would always see a "change".
    #[serde(default)]
    pub uuid: String,
}

// Identity is metadata, not content: two blocks are equal iff their body and
// subtree match, regardless of uuid. Keeps the external-change conflict guard
// and the round-trip tests comparing on content alone.
impl PartialEq for DocBlock {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw && self.children == other.children
    }
}
impl Eq for DocBlock {}

impl DocBlock {
    pub fn new(raw: impl Into<String>) -> Self {
        DocBlock { raw: raw.into(), children: Vec::new(), uuid: String::new() }
    }

    /// `key:: value` properties found in the block body, in order.
    pub fn properties(&self) -> Vec<(String, String)> {
        self.raw.lines().filter_map(parse_property_line).collect()
    }

    pub fn property(&self, key: &str) -> Option<String> {
        self.raw
            .lines()
            .filter_map(parse_property_line)
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, v)| v)
    }

    pub fn collapsed(&self) -> bool {
        self.property("collapsed").as_deref() == Some("true")
    }

    /// The leading task marker, if any (`TODO`, `DOING`, ...).
    pub fn marker(&self) -> Option<&'static str> {
        let first = self.raw.trim_start();
        for m in MARKERS {
            if first == *m || first.starts_with(&format!("{m} ")) {
                return Some(m);
            }
        }
        None
    }

    /// Heading level if the block body is an ATX heading (`# ` .. `###### `).
    pub fn heading_level(&self) -> Option<u8> {
        let first = self.raw.lines().next().unwrap_or("");
        let hashes = first.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) && first[hashes..].starts_with(' ') {
            Some(hashes as u8)
        } else {
            None
        }
    }
}

pub(crate) fn parse_property_line(line: &str) -> Option<(String, String)> {
    // `key:: value` — key is letters/digits/_/-/. and at least one char.
    let idx = line.find("::")?;
    let key = line[..idx].trim();
    if key.is_empty()
        || !key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return None;
    }
    let value = line[idx + 2..].trim().to_string();
    Some((key.to_string(), value))
}

/// Number of leading whitespace characters (tabs and spaces). Used as an
/// indent "column" for nesting. Tabs and spaces each count as one; within a
/// single file indentation is consistent (all tabs or all N-spaces), so column
/// comparison recovers nesting regardless of which a file uses. Output is
/// always canonicalized to TABs.
fn leading_ws(line: &str) -> usize {
    line.bytes().take_while(|b| *b == b'\t' || *b == b' ').count()
}

/// Classify a line as a bullet at a given indent column, returning its content
/// (text after the `- ` marker). Returns `None` for non-bullet lines.
fn bullet(line: &str) -> Option<(usize, &str)> {
    let col = leading_ws(line);
    let rest = &line[col..];
    if rest == "-" {
        Some((col, ""))
    } else if let Some(content) = rest.strip_prefix("- ") {
        Some((col, content))
    } else {
        None
    }
}

/// Parse a file's contents into a [`Document`].
pub fn parse(content: &str) -> Document {
    let body = content.strip_suffix('\n').unwrap_or(content);
    let lines: Vec<&str> = if body.is_empty() { Vec::new() } else { body.split('\n').collect() };

    // Find the first bullet to split pre-block from block region.
    let first_bullet = lines.iter().position(|l| bullet(l).is_some());

    let (pre_lines, block_lines) = match first_bullet {
        Some(i) => (&lines[..i], &lines[i..]),
        None => (&lines[..], &[][..]),
    };

    // Pre-block: drop trailing blank lines (the separator is re-added on write).
    let mut pre_end = pre_lines.len();
    while pre_end > 0 && pre_lines[pre_end - 1].trim().is_empty() {
        pre_end -= 1;
    }
    let pre_block = if pre_end == 0 { None } else { Some(pre_lines[..pre_end].join("\n")) };

    // Build the block forest with a stack of frames keyed by indent column.
    struct Frame {
        col: usize,
        /// Column where the block's text starts (`col` + 2 for the `- `).
        content_start: usize,
        raw: String,
        children: Vec<DocBlock>,
    }
    let mut stack: Vec<Frame> = Vec::new();
    let mut roots: Vec<DocBlock> = Vec::new();

    // Collapse frames at indent column >= `keep_above` into their parents.
    fn fold_to(stack: &mut Vec<Frame>, roots: &mut Vec<DocBlock>, keep_above: usize) {
        while let Some(top) = stack.last() {
            if top.col >= keep_above {
                let f = stack.pop().unwrap();
                let block = DocBlock { raw: f.raw, children: f.children, uuid: String::new() };
                match stack.last_mut() {
                    Some(parent) => parent.children.push(block),
                    None => roots.push(block),
                }
            } else {
                break;
            }
        }
    }

    for line in block_lines {
        if let Some((col, content)) = bullet(line) {
            // New block: fold every block at this column or deeper, so the
            // remaining stack top (shallower column) becomes the parent.
            fold_to(&mut stack, &mut roots, col);
            stack.push(Frame {
                col,
                content_start: col + 2,
                raw: content.to_string(),
                children: Vec::new(),
            });
        } else if let Some(top) = stack.last_mut() {
            // Continuation line: strip the block's content-start indentation.
            let stripped = strip_n_ws(line, top.content_start);
            top.raw.push('\n');
            top.raw.push_str(stripped);
        }
        // (A continuation before any bullet can't happen: it'd be pre-block.)
    }
    fold_to(&mut stack, &mut roots, 0);

    Document { pre_block, roots }
}

/// Remove up to `n` leading whitespace characters (tabs or spaces).
fn strip_n_ws(line: &str, n: usize) -> &str {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < n && i < bytes.len() && (bytes[i] == b'\t' || bytes[i] == b' ') {
        i += 1;
    }
    &line[i..]
}

/// Formatting knobs detected from a file so re-saving preserves its existing
/// style (avoids gratuitous diffs / Syncthing churn). Logseq, for instance,
/// writes files with NO trailing newline; imposing one would rewrite every file.
#[derive(Debug, Clone)]
pub struct SerializeOpts {
    /// Number of trailing `\n` characters to end the file with.
    pub trailing_newlines: usize,
    /// Emit a blank line between the page-property pre-block and the first block.
    pub blank_after_props: bool,
    /// Whitespace for one level of indentation (e.g. `"\t"` or `"  "`).
    pub indent: String,
}

impl Default for SerializeOpts {
    fn default() -> Self {
        SerializeOpts { trailing_newlines: 1, blank_after_props: true, indent: "\t".into() }
    }
}

impl SerializeOpts {
    /// Infer the formatting of an existing on-disk file so a save reproduces it.
    /// `None` (new file) falls back to the default.
    pub fn detect(existing: Option<&str>) -> SerializeOpts {
        match existing {
            None => SerializeOpts::default(),
            Some(s) => SerializeOpts {
                trailing_newlines: s.bytes().rev().take_while(|b| *b == b'\n').count(),
                blank_after_props: blank_after_props(s),
                indent: detect_indent(s),
            },
        }
    }
}

/// Does the file put a blank line between its pre-block and the first bullet?
fn blank_after_props(s: &str) -> bool {
    let lines: Vec<&str> = s.split('\n').collect();
    match lines.iter().position(|l| bullet(l).is_some()) {
        Some(i) if i > 0 => lines[i - 1].trim().is_empty(),
        _ => true,
    }
}

fn gcd(a: usize, b: usize) -> usize {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// Infer the per-level indentation unit from a file's indented bullet lines:
/// a tab if any are tab-indented, else N spaces (the GCD of space widths).
fn detect_indent(s: &str) -> String {
    let mut space_widths: Vec<usize> = Vec::new();
    for line in s.split('\n') {
        let lead_len = line.len() - line.trim_start_matches([' ', '\t']).len();
        if lead_len == 0 {
            continue;
        }
        let rest = &line[lead_len..];
        if !(rest == "-" || rest.starts_with("- ")) {
            continue; // only indented bullet lines reveal the level unit
        }
        if line[..lead_len].contains('\t') {
            return "\t".into();
        }
        space_widths.push(lead_len);
    }
    let w = space_widths.into_iter().fold(0usize, gcd);
    if w >= 2 { " ".repeat(w) } else { "\t".into() }
}

/// Serialize a [`Document`] back to Logseq-compatible markdown (default style).
pub fn serialize(doc: &Document) -> String {
    serialize_with(doc, &SerializeOpts::default())
}

/// Serialize, reproducing a file's detected formatting (see [`SerializeOpts`]).
pub fn serialize_with(doc: &Document, opts: &SerializeOpts) -> String {
    let mut out: Vec<String> = Vec::new();
    if let Some(pre) = &doc.pre_block {
        for line in pre.split('\n') {
            out.push(line.to_string());
        }
        // Blank separator before blocks — only when blocks follow and the file
        // used one.
        if !doc.roots.is_empty() && opts.blank_after_props {
            out.push(String::new());
        }
    }
    for block in &doc.roots {
        emit_block(block, 0, &opts.indent, &mut out);
    }
    let mut s = out.join("\n");
    s.push_str(&"\n".repeat(opts.trailing_newlines));
    s
}

fn emit_block(block: &DocBlock, level: usize, unit: &str, out: &mut Vec<String>) {
    let ind = unit.repeat(level);
    let mut lines = block.raw.split('\n');
    let first = lines.next().unwrap_or("");
    if first.is_empty() {
        out.push(format!("{ind}-"));
    } else {
        out.push(format!("{ind}- {first}"));
    }
    for line in lines {
        if line.is_empty() {
            out.push(String::new());
        } else {
            out.push(format!("{ind}  {line}"));
        }
    }
    for child in &block.children {
        emit_block(child, level + 1, unit, out);
    }
}
