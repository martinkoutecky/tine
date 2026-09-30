//! Read-only corpus gate. Historical projection below is a test oracle, never a writer.
use std::path::Path;
use tine_core::{
    block_regions::{parse, Edit},
    doc::{self, DocBlock},
};
fn check_block(
    b: &DocBlock,
    file: usize,
    index: &mut usize,
    total: &mut usize,
    visible: &mut usize,
    copy: &mut usize,
) {
    *index += 1;
    *total += 1;
    let regions = parse(b.raw(), b.is_org());
    let blocks = tine_core::render::parse_block(b.raw(), b.is_org());
    let old = visible_minus_properties(b.raw(), &blocks);
    let next = regions.apply(b.raw(), b.is_org(), Edit::Visible).unwrap();
    if next != old {
        *visible += 1;
        println!(
            "visible-difference file={file} block={index} literal_regions={}",
            regions.literals.len()
        );
    }
    let old_copy = b
        .raw()
        .lines()
        .filter(|line| {
            let t = line.trim().to_ascii_lowercase();
            !(t.starts_with("id::")
                || t.starts_with(":id:")
                || t.starts_with("template::")
                || t.starts_with("template-including-parent::"))
        })
        .collect::<Vec<_>>()
        .join("\n");
    let next_copy = regions
        .apply(b.raw(), b.is_org(), Edit::StripCopy { template: true })
        .unwrap();
    if next_copy != old_copy {
        *copy += 1;
        let reason = if next_copy.strip_suffix('\n') == Some(old_copy.as_str()) {
            "terminal-newline-preservation"
        } else if b.is_org() {
            "org-metadata-ownership"
        } else if b.raw().contains("\r\n") {
            "line-ending-preservation"
        } else if !regions.literals.is_empty() {
            "literal-metadata-preservation"
        } else {
            "parser-property-ownership"
        };
        let old_removed = b
            .raw()
            .lines()
            .filter(|line| {
                let t = line.trim().to_ascii_lowercase();
                t.starts_with("id::")
                    || t.starts_with(":id:")
                    || t.starts_with("template::")
                    || t.starts_with("template-including-parent::")
            })
            .count();
        let accepted_removed = regions
            .properties
            .iter()
            .filter(|p| {
                matches!(
                    p.key.to_ascii_lowercase().as_str(),
                    "id" | "template" | "template-including-parent"
                )
            })
            .count();
        println!("copy-difference file={file} block={index} reason={reason} old_removed={old_removed} accepted_removed={accepted_removed}");
    }
    for child in &b.children {
        check_block(child, file, index, total, visible, copy);
    }
}
fn walk(dir: &Path, files: &mut usize, total: &mut usize, visible: &mut usize, copy: &mut usize) {
    let mut entries = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect::<Vec<_>>();
    entries.sort();
    for path in entries {
        if path.is_dir() {
            walk(&path, files, total, visible, copy);
        } else {
            let org = path.extension().and_then(|e| e.to_str()) == Some("org");
            if !org && path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let raw = std::fs::read_to_string(&path).unwrap();
            let d = if org {
                tine_core::org::parse_org(&raw)
            } else {
                doc::parse(&raw)
            };
            *files += 1;
            let mut index = 0;
            for b in &d.roots {
                check_block(b, *files, &mut index, total, visible, copy);
            }
        }
    }
}
fn main() {
    let dir = std::env::args()
        .nth(1)
        .expect("usage: region-corpus <read-only graph copy>");
    let (mut files, mut total, mut visible, mut copy) = (0, 0, 0, 0);
    walk(
        Path::new(&dir),
        &mut files,
        &mut total,
        &mut visible,
        &mut copy,
    );
    println!("files={files} blocks={total} panics=0 visible_differences={visible} copy_differences={copy}");
}
fn visible_minus_properties(raw: &str, blocks: &[lsdoc::ast::Block]) -> String {
    use lsdoc::ast::Block;
    let lead = raw.len() - raw.trim_start().len();
    let bytes = raw.as_bytes();
    let mut cuts: Vec<(usize, usize)> = Vec::new();
    for b in blocks {
        if let Block::Properties { span: Some(sp), .. } = b {
            let mut rs = (sp.0.saturating_sub(2) + lead).min(raw.len());
            let mut re = (sp.1.saturating_sub(2) + lead).min(raw.len());
            if rs >= re {
                continue;
            }
            // Extend to whole lines (newlines are char boundaries → slices stay UTF-8 valid).
            while rs > 0 && bytes[rs - 1] != b'\n' {
                rs -= 1;
            }
            while re < raw.len() && bytes[re - 1] != b'\n' {
                re += 1;
            }
            cuts.push((rs, re));
        }
    }
    if cuts.is_empty() {
        return raw.to_string();
    }
    cuts.sort_by_key(|c| c.0);
    let mut out = String::with_capacity(raw.len());
    let mut pos = 0usize;
    for (s, e) in cuts {
        if s < pos {
            pos = pos.max(e); // overlapping/adjacent property ranges
            continue;
        }
        out.push_str(&raw[pos..s]);
        pos = e;
    }
    out.push_str(&raw[pos..]);
    out.trim_end_matches('\n').to_string()
}
