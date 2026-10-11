//! Source guards' one classification of test-only code (REVIEW-3b-P2b-2 N2).
//! A `cfg` predicate is parsed, never matched as text: `cfg(not(test))` and
//! `cfg(feature = "test-faults")` are production. Shared by the lib census
//! tests (`crate::test_cfg`) and the integration-test guards
//! (`#[path = "../src/test_cfg.rs"]`); not every includer uses both helpers.
#![allow(dead_code)]

/// A `#[cfg(P)]` whose predicate implies `test`: `test`, `all(.., P, ..)`
/// with any child implying it, `any(..)` with every child implying it. `not`,
/// every other predicate and one that does not parse are production (fail
/// closed).
pub fn cfg_test(attrs: &[syn::Attribute]) -> bool {
    fn implies_test(predicate: &syn::Meta) -> bool {
        let args = |list: &syn::MetaList| {
            list.parse_args_with(
                syn::punctuated::Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated,
            )
            .map(|args| args.into_iter().collect::<Vec<_>>())
            .unwrap_or_default()
        };
        match predicate {
            syn::Meta::Path(path) => path.is_ident("test"),
            syn::Meta::List(list) if list.path.is_ident("all") => {
                args(list).iter().any(implies_test)
            }
            syn::Meta::List(list) if list.path.is_ident("any") => {
                let args = args(list);
                !args.is_empty() && args.iter().all(implies_test)
            }
            _ => false,
        }
    }
    attrs.iter().any(|attr| {
        attr.path().is_ident("cfg")
            && attr
                .parse_args::<syn::Meta>()
                .is_ok_and(|predicate| implies_test(&predicate))
    })
}

/// Test-only code: a test attribute (`#[test]`, `#[tokio::test]`) or
/// [`cfg_test`].
pub fn is_test(attrs: &[syn::Attribute]) -> bool {
    cfg_test(attrs)
        || attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|s| s.ident == "test")
        })
}
