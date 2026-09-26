use syn::{Attribute, ImplItem, Item, Meta};

const RULE: &str = "production indexes are not test-only; one composition";
const TEST_HOOKS: &[&str] = &["cache_publish_pause"];

fn is_cache_name(name: &str) -> bool {
    !TEST_HOOKS.contains(&name)
        && (name.ends_with("_index") || name.ends_with("_cache") || name.starts_with("memo"))
}

fn test_gated(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        matches!(
            attr.path().get_ident().map(|id| id.to_string()).as_deref(),
            Some("cfg" | "cfg_attr")
        ) && match &attr.meta {
            Meta::List(list) => list
                .tokens
                .to_string()
                .split(|c: char| !c.is_alphanumeric())
                .any(|token| token == "test"),
            _ => false,
        }
    })
}

fn violations(source: &str) -> Vec<String> {
    let parsed = syn::parse_file(source).expect("model.rs must parse");
    let mut found = Vec::new();
    for item in parsed.items {
        match item {
            Item::Struct(item)
                if matches!(item.ident.to_string().as_str(), "Graph" | "ReadSnapshot") =>
            {
                let owner = item.ident.to_string();
                for field in item.fields {
                    if let Some(name) = field.ident {
                        let name = name.to_string();
                        if is_cache_name(&name) && test_gated(&field.attrs) {
                            found.push(format!("{owner}.{name}"));
                        }
                    }
                }
            }
            Item::Impl(item) => {
                let impl_gated = test_gated(&item.attrs);
                let syn::Type::Path(path) = *item.self_ty else {
                    continue;
                };
                let Some(owner) = path
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.ident.to_string())
                else {
                    continue;
                };
                if !matches!(owner.as_str(), "Graph" | "ReadSnapshot") {
                    continue;
                }
                for member in item.items {
                    if let ImplItem::Fn(method) = member {
                        let name = method.sig.ident.to_string();
                        if is_cache_name(&name) && (impl_gated || test_gated(&method.attrs)) {
                            found.push(format!("{owner}.{name}"));
                        }
                    }
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn production_indexes_are_not_test_only() {
    let found = violations(include_str!("model.rs"));
    assert!(found.is_empty(), "{RULE}: {}", found.join(", "));
}

#[test]
fn guard_detects_field_method_and_impl_gates() {
    let planted = r#"
        struct Graph {
            #[cfg(test)]
            planted_index: (),
        }
        struct ReadSnapshot {
            #[cfg(test)]
            planted_cache: (),
        }
        #[cfg(test)]
        impl Graph {
            fn memo_planted(&self) {}
        }
    "#;
    assert_eq!(
        violations(planted),
        [
            "Graph.planted_index",
            "ReadSnapshot.planted_cache",
            "Graph.memo_planted"
        ]
    );
}
