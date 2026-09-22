//! Lowering: parsed pages to the projection's physical rows -- blocks,
//! properties, reference postings, tasks and source revisions. Pure
//! functions of a page's text and parse; the worker decides when they run.

use super::*;

pub(super) struct LoweredDeltas {
    pub(super) applied: AppliedTurn,
    pub(super) change: PhysicalGraphProjectionChange,
    pub(super) revisions: Vec<PhysicalGraphProjectionSourceRevision>,
    pub(super) aliases: Vec<PhysicalAliasDeclaration>,
}

pub(super) fn lower_deltas(
    deltas: BTreeMap<String, (u64, PageDelta)>,
) -> Result<LoweredDeltas, String> {
    let mut turn = AppliedTurn::default();
    let mut replacements = Vec::new();
    let mut reference_postings = Vec::new();
    let mut aliases = Vec::new();
    let mut replacement_sources = Vec::new();
    let mut deletions = Vec::new();
    for (_, (_, delta)) in deltas {
        match delta {
            PageDelta::Replace {
                entry,
                document,
                revision,
                parse_config,
                page_position,
            } => {
                replacement_sources.push(PhysicalGraphProjectionSourceRevision {
                    path: entry.rel_path.clone(),
                    revision: projection_source_revision(&revision, parse_config.digest()),
                });
                let (mut page, mut postings, mut page_aliases) =
                    physical_page(&entry, &document, &parse_config)?;
                page.position = page_position;
                turn.pages.lowered.push(page.path.clone());
                replacements.push(page);
                reference_postings.append(&mut postings);
                aliases.append(&mut page_aliases);
            }
            PageDelta::Delete { entry } => {
                let id = entry.rel_path;
                turn.pages.deleted.push(id.clone());
                deletions.push(id);
            }
        }
    }
    Ok(LoweredDeltas {
        applied: turn,
        change: PhysicalGraphProjectionChange {
            replacements,
            deletions,
            reference_postings,
        },
        revisions: replacement_sources,
        aliases,
    })
}

/// The revision Direct Files compares to decide whether a page's rows are still
/// current. Folding the parse-config digest in is what makes a config edit a
/// full re-lowering (§5.8 J7): reconciliation compares only source revisions,
/// so without it an unchanged file would keep rows built under the old config.
pub(crate) fn projection_source_revision(
    content_revision: &str,
    parse_config_digest: tine_storage::ContentDigest,
) -> String {
    let digest = parse_config_digest
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    format!("direct-facts-v{DIRECT_PROJECTION_FACTS_VERSION}:{digest}:{content_revision}")
}

/// The page and block text a snapshot projects — the input to
/// `projection_budget::build_page_cache_budget`, because the build's cache
/// working set is a schema-fixed multiple of it, not of the page count.
pub(super) fn projected_text_bytes(pages: &[tine_storage::sqlite::PhysicalPage]) -> u64 {
    pages
        .iter()
        .map(|page| {
            let own = page.search_tokens.len() as u64
                + page.preamble.as_ref().map_or(0, |text| text.len() as u64);
            own + page
                .blocks
                .iter()
                .map(|block| (block.content.len() + block.search_tokens.len()) as u64)
                .sum::<u64>()
        })
        .sum()
}

/// The Direct Files producer, reachable from the cross-backend parity guard.
///
/// Named as a seam rather than widened: the guard has to compare the rows this
/// exact function emits against the walk's,
/// and a reimplementation in the test would prove only that the test agrees
/// with itself (§5.8 G1, I-19).
#[cfg(test)]
pub(crate) fn physical_page_for_test(
    entry: &PageEntry,
    document: &Document,
    parse_config: &ParseConfig,
) -> Result<PhysicalPage, String> {
    physical_page(entry, document, parse_config).map(|(page, _, _)| page)
}

pub(super) fn physical_page(
    entry: &PageEntry,
    document: &Document,
    parse_config: &ParseConfig,
) -> Result<
    (
        PhysicalPage,
        Vec<PhysicalReferencePosting>,
        Vec<PhysicalAliasDeclaration>,
    ),
    String,
> {
    #[cfg(test)]
    {
        let mut receipt = PHYSICAL_PAGE_LOWERINGS.lock().unwrap();
        if receipt
            .0
            .as_ref()
            .is_some_and(|root| entry.path.starts_with(root))
        {
            receipt.1 += 1;
        }
    }
    let path = entry.rel_path.as_str();
    let format = Format::from_path(Path::new(&entry.rel_path));
    let is_org = format == Format::Org;
    // `Format::from_path` and never `reference_source_is_org`: the latter is a
    // case-sensitive `ends_with(".org")` and would type an `Outline.ORG` page
    // Markdown here while Direct Files types it Org (§5.8 E4).
    let atom_format = crate::query::atom::AtomFormat::from(format);
    let (preamble_visible, properties, tags) = document
        .pre_block
        .as_deref()
        .map(|raw| facets(raw, is_org))
        .unwrap_or_default();
    let visible_search_text = if preamble_visible.is_empty() {
        entry.name.clone()
    } else {
        format!("{} {preamble_visible}", entry.name)
    };
    let mut blocks = Vec::new();
    let mut reference_postings = Vec::new();
    let aliases = crate::query::document_alias_spellings(document)
        .into_iter()
        .enumerate()
        .map(|(ordinal, (raw_alias, normalized_alias))| {
            Ok(PhysicalAliasDeclaration {
                source_page_path: path.to_owned(),
                source_entity: PhysicalEntityId::Page(path.to_owned()),
                source_locator: b"page-alias".to_vec(),
                ordinal: u32::try_from(ordinal)
                    .map_err(|_| "one page exceeds u32::MAX aliases".to_string())?,
                raw_alias,
                normalized_alias,
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    if let Some(preamble) = document.pre_block.as_deref() {
        append_reference_postings(
            &mut reference_postings,
            path,
            PhysicalEntityId::Page(path.to_owned()),
            b"preamble",
            std::iter::empty(),
            crate::doc::property_reference_page_names(preamble).into_iter(),
        )?;
    }
    let mut block_refs_norm: Vec<Vec<String>> = Vec::new();
    lower_blocks(
        &document.roots,
        path,
        None,
        &mut Vec::new(),
        &mut blocks,
        &mut reference_postings,
        &mut block_refs_norm,
        parse_config,
        atom_format,
    )?;
    // The two derived tables come from the ONE tine-core computation (§5.8):
    // this side only hands it the block's own `refs_norm` and its parent.
    let block_indices = blocks
        .iter()
        .enumerate()
        .map(|(index, block)| (block.result_id.as_str(), index))
        .collect::<HashMap<_, _>>();
    let flat = blocks
        .iter()
        .enumerate()
        .zip(block_refs_norm.iter())
        .map(
            |((index, block), refs)| crate::query::path_refs::PathRefBlock {
                id: index,
                parent: block
                    .parent
                    .as_deref()
                    .and_then(|parent| block_indices.get(parent).copied()),
                refs: refs.as_slice(),
            },
        )
        .collect::<Vec<_>>();
    let mut path_refs = crate::query::derived::path_ref_rows(&entry.name, &flat);
    for (index, block) in blocks.iter_mut().enumerate() {
        block.path_refs = path_refs
            .remove(&index)
            .unwrap_or_default()
            .into_iter()
            .map(|key| PhysicalName {
                raw: key.clone(),
                key,
            })
            .collect();
    }
    let journal_days = crate::query::derived::JournalDays::new(parse_config);
    let page_property_atoms = crate::query::derived::property_atom_rows(
        &properties
            .iter()
            .map(|property| (property.name.clone(), property.value.clone()))
            .collect::<Vec<_>>(),
        atom_format,
        parse_config,
    );
    Ok((
        PhysicalPage {
            position: None,
            name: entry.name.clone(),
            name_key: crate::refs::page_key(&entry.name),
            path: entry.rel_path.clone(),
            text_kind: page_kind_to_sql(entry.kind),
            journal_day: journal_days.day(&entry.rel_path, entry.kind == PageKind::Journal),
            preamble: document.pre_block.clone(),
            search_tokens: crate::search_query::canonical_fold(&visible_search_text),
            properties,
            tags: crate::query::derived::tag_rows(&tags),
            property_atoms: page_property_atoms,
            blocks,
        },
        reference_postings,
        aliases,
    ))
}

#[allow(clippy::too_many_arguments)]
pub(super) fn lower_blocks(
    source: &[DocBlock],
    page_path: &str,
    parent: Option<&str>,
    structural_path: &mut Vec<u32>,
    out: &mut Vec<PhysicalBlock>,
    reference_postings: &mut Vec<PhysicalReferencePosting>,
    refs_norm: &mut Vec<Vec<String>>,
    parse_config: &ParseConfig,
    atom_format: crate::query::atom::AtomFormat,
) -> Result<(), String> {
    for (position, block) in source.iter().enumerate() {
        let position = u32::try_from(position)
            .map_err(|_| "page has more than u32::MAX sibling blocks".to_string())?;
        structural_path.push(position);
        let projection = block.projection();
        let order = structural_path
            .iter()
            .map(|part| format!("{part:08x}"))
            .collect::<Vec<_>>()
            .join("/");
        append_reference_postings(
            reference_postings,
            page_path,
            PhysicalEntityId::Block(block.uuid.clone()),
            order.as_bytes(),
            projection.refs_page.iter().cloned(),
            crate::doc::property_reference_page_names(&block.raw).into_iter(),
        )?;
        for raw_claim in &projection.block_refs {
            let Ok(raw_claim) = Uuid::parse_str(raw_claim) else {
                continue;
            };
            reference_postings.push(PhysicalReferencePosting {
                source_page_path: page_path.to_owned(),
                source_entity: PhysicalEntityId::Block(block.uuid.clone()),
                source_locator: order.as_bytes().to_vec(),
                ordinal: u32::try_from(reference_postings.len())
                    .map_err(|_| "one page exceeds u32::MAX reference postings".to_string())?,
                kind: 6,
                target: PhysicalReferenceTarget::ExternalUuid {
                    raw_claim: raw_claim.into_bytes(),
                },
            });
        }
        let properties = projection
            .properties
            .iter()
            .map(|(name, value)| PhysicalProperty {
                name: name.clone(),
                normalized_name: property_key_norm(name),
                value: value.clone(),
            })
            .collect();
        let property_atoms = crate::query::derived::property_atom_rows(
            &projection.properties,
            atom_format,
            parse_config,
        );
        refs_norm.push(projection.refs_norm.clone());
        let logseq_uuid = block
            .property("id")
            .and_then(|value| Uuid::parse_str(value.trim()).ok())
            .map(Uuid::into_bytes);
        out.push(PhysicalBlock {
            result_id: block.uuid.clone(),
            own_refs: projection
                .refs_norm
                .iter()
                .map(|key| PhysicalName {
                    raw: key.clone(),
                    key: key.clone(),
                })
                .collect(),
            parent: parent.map(str::to_owned),
            order,
            content: block.raw.clone(),
            search_tokens: projection.visible_lower.clone(),
            heading_level: projection.heading_level,
            collapsed: block.collapsed(),
            logseq_uuid,
            logseq_identity_origin: logseq_uuid.map(|_| 0),
            properties,
            tags: crate::query::derived::tag_rows(&projection.tags),
            task: projection.marker.as_ref().map(|marker| PhysicalTask {
                marker: marker.to_ascii_uppercase(),
                priority: projection.priority.clone(),
                scheduled: projection.scheduled.clone(),
                deadline: projection.deadline.clone(),
            }),
            // Written from the three projection fields alone, so a markerless
            // block gets a row exactly as a marked one does (§3.2 M2).
            planning: crate::query::derived::planning_row(
                projection.priority.as_deref(),
                projection.scheduled.as_deref(),
                projection.deadline.as_deref(),
            ),
            // Filled once per page, after the whole flat block list exists.
            path_refs: Vec::new(),
            property_atoms,
        });
        lower_blocks(
            &block.children,
            page_path,
            Some(block.uuid.as_str()),
            structural_path,
            out,
            reference_postings,
            refs_norm,
            parse_config,
            atom_format,
        )?;
        structural_path.pop();
    }
    Ok(())
}

pub(super) fn append_reference_postings(
    out: &mut Vec<PhysicalReferencePosting>,
    page_path: &str,
    source: PhysicalEntityId,
    source_locator: &[u8],
    inline_names: impl IntoIterator<Item = String>,
    property_names: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    let mut ordinal = 0_u32;
    for (kind, names) in [
        (0_i64, inline_names.into_iter().collect::<Vec<_>>()),
        (3_i64, property_names.into_iter().collect::<Vec<_>>()),
    ] {
        for raw_name in names {
            out.push(PhysicalReferencePosting {
                source_page_path: page_path.to_owned(),
                source_entity: source.clone(),
                source_locator: source_locator.to_vec(),
                ordinal,
                kind,
                target: PhysicalReferenceTarget::PageName {
                    normalized_name: crate::refs::page_key(&raw_name),
                    raw_name,
                },
            });
            ordinal = ordinal
                .checked_add(1)
                .ok_or_else(|| "one reference source exceeds u32::MAX postings".to_string())?;
        }
    }
    Ok(())
}

pub(super) fn facets(raw: &str, is_org: bool) -> (String, Vec<PhysicalProperty>, Vec<String>) {
    let block = DocBlock::preamble(raw, is_org);
    let searchable = block.visible_text().to_owned();
    let properties = block
        .projection()
        .properties
        .iter()
        .map(|(name, value)| PhysicalProperty {
            name: name.clone(),
            normalized_name: property_key_norm(name),
            value: value.clone(),
        })
        .collect();
    (searchable, properties, block.projection().tags.clone())
}

pub(super) fn page_kind_to_sql(kind: PageKind) -> i64 {
    match kind {
        PageKind::Page => 0,
        PageKind::Journal => 1,
    }
}

/// `pages.text_kind` back to the parser's `PageKind`. A value outside the two
/// the producer writes is projection damage, not a third kind, so every reader
/// treats `None` as a failed read (D-3).
pub(crate) fn page_kind_from_sql(kind: i64) -> Option<PageKind> {
    match kind {
        0 => Some(PageKind::Page),
        1 => Some(PageKind::Journal),
        _ => None,
    }
}
