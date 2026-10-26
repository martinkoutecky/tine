//! One integration executable; module names retain each source suite in test filters.

#[path = "block_region_ratchet.rs"]
mod block_region_ratchet;

#[path = "block_regions.rs"]
mod block_regions;

#[path = "bounded_projection.rs"]
mod bounded_projection;

#[path = "linkable_page_property_refs.rs"]
mod linkable_page_property_refs;

#[path = "logseq_pdf_edn.rs"]
mod logseq_pdf_edn;

#[path = "lsdoc_parse_boundary.rs"]
mod lsdoc_parse_boundary;

#[path = "media_mime.rs"]
mod media_mime;

#[path = "mixed_code_edit.rs"]
mod mixed_code_edit;

#[path = "no_file_io.rs"]
mod no_file_io;

#[path = "og_b_door2_reference.rs"]
mod og_b_door2_reference;

#[path = "og_b_store_integrity.rs"]
mod og_b_store_integrity;

#[path = "og_b_store_preamble.rs"]
mod og_b_store_preamble;

#[path = "og_differential_pages.rs"]
mod og_differential_pages;

#[path = "og_dupd2.rs"]
mod og_dupd2;

#[path = "og_dupf05.rs"]
mod og_dupf05;

#[path = "og_gh644_property_blank_line.rs"]
mod og_gh644_property_blank_line;

#[path = "og_gh644_f2_quote_property_group.rs"]
mod og_gh644_f2_quote_property_group;

#[path = "og_k1_content.rs"]
mod og_k1_content;

#[path = "og_p10_journal_date_cost.rs"]
mod og_p10_journal_date_cost;

#[path = "og_p10_projection_cost.rs"]
mod og_p10_projection_cost;

#[path = "og_p12_parser.rs"]
mod og_p12_parser;

#[path = "og_r_facets_fixture.rs"]
mod og_r_facets_fixture;

#[path = "og_r_render_facets.rs"]
mod og_r_render_facets;

#[path = "org_headline_refs.rs"]
mod org_headline_refs;

#[path = "page_filename_identity.rs"]
mod page_filename_identity;

#[path = "page_header_panic.rs"]
mod page_header_panic;

#[path = "page_title_preamble.rs"]
mod page_title_preamble;

#[path = "query_edn_spans.rs"]
mod query_edn_spans;

#[path = "region_edit_regressions.rs"]
mod region_edit_regressions;

#[path = "roundtrip.rs"]
mod roundtrip;

#[path = "serialization_layout.rs"]
mod serialization_layout;

#[path = "sync_conflict_names.rs"]
mod sync_conflict_names;
