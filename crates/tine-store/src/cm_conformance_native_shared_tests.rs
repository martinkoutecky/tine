//! Native15c parser observations, shared by store and physical merge tests.
//! These read native bytes independently of merge placements and origins.
use super::*;
use tine_core::text_trace::{Placement, Role, TextTrace};

fn row(raw: &str, level: usize, block: &DocBlock, config: &Config) -> Value {
    let mut row = block_json(raw, level, block, config);
    // Native occurrence kind follows the value's semantic owner. A bare
    // alias becoming [[comma, name]] remains one linkable-value occurrence;
    // r13's concrete scanner-rule diagnostic remains available separately.
    let projection = block.projection();
    let source = projection.reference_source();
    let mut values =
        tine_core::reference_evidence::linkable_property_references(source, &projection.regions)
            .peekable();
    row["scannerOcc"] = row["occ"].clone();
    let mut output = Vec::new();
    let mut tag: Option<&tine_core::reference_evidence::ProjectedPageRef> = None;
    for reference in source.explicit {
        let value = values
            .peek()
            .is_some_and(|value| std::ptr::eq(*value, reference));
        if value {
            values.next();
        }
        // A link used solely to encode its enclosing tag names the same
        // semantic site. Keep separate declarations and distinct child names.
        // The concrete scanner nodes remain exported above as diagnostics.
        let encoding_child = matches!(reference.rule, "explicit_link" | "explicit_nested_link")
            && tag.is_some_and(|tag| {
                tag.range.start <= reference.range.start
                    && reference.range.end <= tag.range.end
                    && tag.name == reference.name
            });
        if reference.rule == "explicit_tag" {
            tag = Some(reference);
        }
        if !encoding_child && visit_reference_targets(reference, config, |_| true) {
            let kind = if value
                && matches!(
                    reference.rule,
                    "implicit_linkable_property"
                        | "explicit_link"
                        | "explicit_nested_link"
                        | "explicit_tag"
                ) {
                "linkableValue"
            } else {
                occ_kind(reference.rule)
            };
            output.push(json!([kind, reference.name]));
        }
    }
    row["occ"] = json!(output);
    row
}

#[test]
fn native_linkable_values_keep_their_kind_through_wrappers() {
    for (outside, value) in [
        ("#outside", "inside"),
        ("#outside", "[[inside]]"),
        ("#outside", "#[[inside]]"),
        ("#[[outside]]", "[[comma, name]]"),
        ("#outside", "inside, inside"),
    ] {
        let raw = format!("[[inside]] {outside}\nalias:: {value}");
        let block = DocBlock::new(raw.as_str());
        let config = Config {
            property_pages_enabled: false,
            ..Config::default()
        };
        let row = row(&raw, 1, &block, &config);
        let occurrences = row["occ"].as_array().unwrap();
        assert!(occurrences.contains(&json!(["link", "inside"])));
        assert!(occurrences.contains(&json!(["tag", "outside"])));
        let name = if value == "[[comma, name]]" {
            "comma, name"
        } else {
            "inside"
        };
        assert!(occurrences.contains(&json!(["linkableValue", name])));
        assert_eq!(
            occurrences.len(),
            if value == "inside, inside" { 4 } else { 3 },
            "{value}: {occurrences:?}"
        );
    }
}

pub fn decorate(file: &mut Value, config: &Config) {
    let path = file["path"].as_str().unwrap();
    let text = file["text"].as_str().unwrap();
    let org = file_format(path) == Format::Org;
    let parsed = parse_file(path, text);
    let outline = tine_core::refs::physical_outline(text, org);
    let first = outline.first().map_or(text.len(), |b| b.0.start);
    let pre = parsed.pre_block.as_deref();
    let header =
        tine_core::page_header::page_header_text(pre, parsed.roots.first().map(DocBlock::raw));
    let fields: Vec<_> = header.into_iter().flat_map(|raw| {
        // Only map layout here: the parser supplies the field and its line.
        // A mixed-ending header cannot borrow another field's terminator.
        let physical_end = if pre.is_some() { first } else { outline.first().map_or(0, |b| b.0.end) };
        let mut at = 0;
        let endings: BTreeMap<_, _> = raw.split_inclusive('\n').zip(tine_core::doc::physical_lines(&text[..physical_end])).map(|(logical, physical)| {
            let start = at;
            at += logical.len();
            (start, tine_core::doc::line_terminator(physical))
        }).collect();
        let regions = tine_core::block_regions::parse_document(raw, org);
        regions.page_properties().filter(|p| tine_core::page_header::is_alias_key(&p.key)).map(|p| {
            json!({"names": tine_core::property_refs::linkable_names(&p.value, org), "ending": endings[&p.line.0]})
        }).collect::<Vec<_>>()
    }).collect();
    let mut rows = Vec::new();
    if let Some(pre) = pre {
        let projections = tine_core::page_properties::pre_block_projections(pre, org);
        let split =
            tine_core::page_properties::preamble_content_range(pre, org).map_or(first, |r| {
                // The reader normalizes Markdown CRLF. Convert its parser offset
                // to physical bytes; Org preambles retain their original endings.
                if org {
                    r.start
                } else {
                    let mut lf = 0;
                    for (at, c) in text[..first].char_indices() {
                        if lf == r.start {
                            return at;
                        }
                        // A CRLF's CR is dropped; a lone CR reads as LF.
                        if !text[at..].starts_with("\r\n") {
                            lf += c.len_utf8();
                        }
                    }
                    first
                }
            });
        let count = projections.len();
        for (i, (kind, block)) in projections.into_iter().enumerate() {
            let mut row = row(block.raw(), 1, &block, config);
            row["kind"] = json!(match kind {
                tine_core::page_properties::PreBlockKind::Properties => "header",
                tine_core::page_properties::PreBlockKind::Preamble => "preamble",
            });
            row["startByte"] = json!(if i == 0 { 0 } else { split });
            row["endByte"] = json!(if count == 1 || i > 0 { first } else { split });
            rows.push(row);
        }
    }
    fn walk_rows(blocks: &[DocBlock], level: usize, out: &mut Vec<Value>, config: &Config) {
        for b in blocks {
            let mut row = row(b.raw(), level, b, config);
            row["kind"] = json!("body");
            out.push(row);
            walk_rows(&b.children, level + 1, out, config);
        }
    }
    let offset = rows.len();
    walk_rows(&parsed.roots, 1, &mut rows, config);
    assert_eq!(
        rows.len() - offset,
        outline.len(),
        "parser outline ownership {path}"
    );
    for (i, (range, _, _, _)) in outline.iter().enumerate() {
        rows[offset + i]["startByte"] = json!(if offset == 0 && i == 0 {
            0
        } else {
            range.start
        });
        rows[offset + i]["endByte"] = json!(range.end);
    }
    let first_root_header = pre.is_none() && header.is_some();
    if first_root_header {
        rows[0]["kind"] = json!("header");
    }
    for row in &mut rows {
        row["parts"] = json!([]);
        row["image"] = json!([]);
    }
    let nonblank = !text.trim().is_empty();
    file["nativeFields"] = json!(fields);
    file["preBlock"] = json!(rows.first().is_some_and(|row| row["kind"] == "header"));
    file["nonblank"] = json!(nonblank);
    file["pieces"] = json!([]);
    file["blocks"] = json!(rows);
}

pub fn pieces(
    trace: &TextTrace,
    file: usize,
    cuts: &[usize],
    next: &mut usize,
) -> (Vec<Value>, BTreeMap<usize, Vec<usize>>) {
    let mut ids = BTreeMap::new();
    let mut pieces = Vec::new();
    for (id, p) in trace
        .pieces
        .iter()
        .enumerate()
        .filter(|(_, p)| p.file == file)
    {
        let start = cuts.partition_point(|at| *at <= p.range.start);
        let stop = cuts.partition_point(|at| *at < p.range.end);
        assert!(
            !p.editable || start == stop,
            "a semantic name cannot cross native rows"
        );
        let mut at = p.range.start;
        let mut refined = Vec::new();
        for end in cuts[start..stop].iter().copied().chain([p.range.end]) {
            let global = *next;
            *next += 1;
            refined.push(global);
            pieces.push(json!({"id": global, "text": &trace.inputs[file][at..end], "editable": p.editable,
                "role": match p.role { Role::Opaque => "opaque", Role::AliasName => "name", Role::AliasKey => "key", Role::AliasDelimiter => "delimiter" }}));
            at = end;
        }
        ids.insert(id, refined);
    }
    (pieces, ids)
}

pub fn row_cuts(file: &Value) -> Vec<usize> {
    file["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["endByte"].as_u64().unwrap() as usize)
        .collect()
}

pub fn placements(trace: &TextTrace, ids: &BTreeMap<usize, Vec<usize>>) -> Vec<Value> {
    trace
        .output
        .iter()
        .flat_map(|p| match p {
            Placement::Original(id) => ids[id].iter().map(|id| json!({"original": id})).collect(),
            Placement::Insert(text) => vec![json!({"insert": text})],
        })
        .collect()
}

/// BEFORE row partitions come from the legal span finder on the old raw row,
/// before consulting any final merge output. The independent Lean map checks
/// their actual renderings; it never trusts Rust's replacement strings.
pub fn row_images(
    before: &mut Value,
    shadow: &mut Value,
    old: &str,
    chosen: &str,
    spec: &GraphSpec,
    next: &mut usize,
) {
    let org = before["format"] == "org";
    let links = std::collections::HashMap::from([(page_key(old), chosen.to_owned())]);
    let a = before["blocks"].as_array_mut().unwrap();
    let b = shadow["blocks"].as_array_mut().unwrap();
    assert_eq!(a.len(), b.len(), "a shadow cannot change the outline");
    for (row, image) in a.iter_mut().zip(b) {
        let raw = row["text"].as_str().unwrap();
        let mut proposal =
            tine_core::refs::rename_proposal(raw, org, &links, &links, spec.name_format()).unwrap();
        if row["kind"] == "header" {
            if let Some(title) =
                page_title_from_preamble(raw, if org { Format::Org } else { Format::Md })
            {
                if let Some(mapped) = tine_core::refs::renamed_page_name(&title, &links) {
                    proposal.rebind_title(org, &mapped).unwrap();
                }
            }
        }
        proposal.trace.check_shadow(&proposal.text).unwrap();
        let (parts, ids) = pieces(&proposal.trace, 0, &[], next);
        let by_id: BTreeMap<_, _> = parts
            .iter()
            .map(|p| (p["id"].as_u64().unwrap() as usize, p.clone()))
            .collect();
        let places: Vec<_> = placements(&proposal.trace, &ids)
            .into_iter()
            .map(|mut p| {
                if let Some(id) = p["original"].as_u64() {
                    p["original"] = by_id[&(id as usize)].clone();
                }
                p
            })
            .collect();
        row["parts"] = json!(parts);
        image["image"] = json!(places);
    }
}

/// Include all descendants: native document equality cannot be replaced by a
/// root-only or outline-count comparison.
pub fn document(doc: &Document) -> Value {
    fn descendants(blocks: &[DocBlock], depth: usize, out: &mut Vec<Value>) {
        for b in blocks {
            out.push(json!([b.raw(), depth]));
            descendants(&b.children, depth + 1, out);
        }
    }
    json!({"preamble": doc.pre_block, "roots": doc.roots.iter().map(|b| {
        let mut children = Vec::new();
        descendants(&b.children, 1, &mut children);
        json!({"raw": b.raw(), "descendants": children})
    }).collect::<Vec<_>>()})
}

pub fn block_query_rows(view: &WholeGraph, q: &Value) -> Vec<BlockId> {
    let (text, dialect) = tine_query_text("block", &q["q"]).expect("native query has a printer");
    let (query, presentation) = parse_query_text(
        &text,
        if dialect == "tql" {
            QueryDialect::Tql
        } else {
            QueryDialect::Og
        },
        JournalDate::from_days(0),
    );
    assert!(!query.is_invalid());
    let Some(IrAnswer::Result(result)) = view
        .query_ir(
            IrRequest::Run {
                query: &query,
                view: &presentation,
                context: &ExecutionContext::none(),
            },
            &Cancel::default(),
        )
        .unwrap()
    else {
        panic!("native query did not produce a result")
    };
    let QueryRows::Block { groups } = result.rows else {
        panic!("block query returned pages")
    };
    let ids = tine_block_ids(view);
    let mut rows: Vec<_> = groups
        .iter()
        .flat_map(|g| g.blocks.iter().map(|b| ids[&b.id].clone()))
        .collect();
    rows.sort();
    rows
}
