use tine_core::{page_properties::property_projection_with_config, query::atom::ParseConfig};

#[test]
fn configured_property_atoms_share_reference_names_and_source_spans() {
    for (is_org, raw) in [
        (false, "related:: Alpha，[[Beta]], `Code`, *Emphasis*, Gamma\nalias:: Editor\nquoted:: \"Literal, Value\"\nordinary:: Lone, Value"),
        (true, ":PROPERTIES:\n:related: Alpha，[[Beta]], ~Code~, /Emphasis/, Gamma\n:alias: Editor\n:quoted: \"Literal, Value\"\n:ordinary: Lone, Value\n:END:"),
    ] {
        let config = ParseConfig { separated_by_commas: vec!["related".into(), "quoted".into()], ..Default::default() };
        let mut block = property_projection_with_config(raw, is_org, &config);
        for name in ["Alpha", "Beta", "Gamma", "Editor"] {
            assert!(block.projection().refs_contains(name), "{name}: {:?}", block.projection().refs_page());
            let reference = block.projection().reference_source().explicit.iter().find(|r| r.name == name).unwrap();
            assert!(raw[reference.range.clone()].contains(name));
        }
        for name in ["Code", "Emphasis", "Literal", "Value", "Lone"] {
            assert!(!block.projection().refs_contains(name), "literal {name}");
        }
        assert!(block.clone().projection().refs_contains("Gamma"));
        block.set_raw(if is_org { ":PROPERTIES:\n:related: Delta\n:END:" } else { "related:: Delta" });
        assert!(block.projection().refs_contains("Delta"));
        assert!(!block.projection().refs_contains("Gamma"));
        let config = ParseConfig { ignored_page_references_keywords: vec!["related".into()], ..config };
        block.set_property_ref_config(&config);
        assert!(!block.projection().refs_contains("Delta"));
    }
}
