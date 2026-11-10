//! D-20261110-TW: disabling empty doctest targets must not lose future examples.

use quote::ToTokens;
use std::path::{Path, PathBuf};
use syn::{parse::Parser, spanned::Spanned};

#[derive(Default)]
struct DocExamples {
    origin: PathBuf,
    current: String,
    current_inner: Option<bool>,
    source: String,
    origins: Vec<PathBuf>,
    rejected: Vec<String>,
}

impl DocExamples {
    fn flush(&mut self) {
        if !self.current.is_empty() {
            self.source.push_str(&self.current);
            self.source.push_str(&format!(
                "pub fn doctest_guard_doc_{}() {{}}\n",
                self.origins.len()
            ));
            self.origins.push(self.origin.clone());
            self.current.clear();
            self.current_inner = None;
        }
    }

    fn inspect(&mut self, path: &Path, source: &str) {
        self.flush();
        self.origin = path.to_path_buf();
        self.tokens(source.parse().expect("tokenize Rust documentation"));
        self.flush();
    }

    fn tokens(&mut self, tokens: proc_macro2::TokenStream) {
        use proc_macro2::{Delimiter, TokenTree};
        let mut tokens = tokens.into_iter().peekable();
        while let Some(token) = tokens.next() {
            if matches!(&token, TokenTree::Punct(p) if p.as_char() == '#') {
                let inner =
                    matches!(tokens.peek(), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if inner {
                    tokens.next();
                }
                if matches!(tokens.peek(), Some(TokenTree::Group(g)) if g.delimiter() == Delimiter::Bracket)
                {
                    if self.current_inner.is_some_and(|previous| previous != inner) {
                        self.flush();
                    }
                    self.current_inner = Some(inner);
                    let group = tokens.next().unwrap();
                    let TokenTree::Group(ref body) = group else {
                        unreachable!()
                    };
                    if !matches!(body.stream().into_iter().next(), Some(TokenTree::Ident(name)) if name == "doc" || name == "cfg_attr")
                    {
                        continue;
                    }
                    let attribute = [token, group].into_iter().collect();
                    match syn::Attribute::parse_outer.parse2(attribute) {
                        Ok(attrs) => {
                            for attr in attrs {
                                self.attribute(&attr);
                            }
                        }
                        Err(error) => self.rejected.push(format!(
                            "{}: uninspectable attribute: {error}",
                            self.origin.display()
                        )),
                    }
                    continue;
                }
            }
            self.flush();
            if let TokenTree::Group(group) = token {
                self.tokens(group.stream());
            }
        }
        self.flush();
    }

    fn examples(mut self) -> Vec<String> {
        self.flush();
        if self.source.is_empty() {
            return self.rejected;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("documentation.rs");
        std::fs::write(&path, self.source).unwrap();
        // List examples without compiling their bodies: imports in examples need no
        // dependencies, and rustdoc owns Markdown/fence/attribute interpretation.
        let output = std::process::Command::new(
            std::env::var_os("RUSTDOC").unwrap_or_else(|| "rustdoc".into()),
        )
        .args(["--test", "--edition=2021", "--test-args=--list"])
        .arg(&path)
        .output()
        .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            if let Some((_, suffix)) = line.split_once("doctest_guard_doc_") {
                let index: usize = suffix
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                self.rejected
                    .push(format!("{}: {line}", self.origins[index].display()));
            }
        }
        self.rejected
    }
    fn attribute(&mut self, attr: &syn::Attribute) {
        if attr.path().is_ident("cfg_attr")
            && attr
                .meta
                .to_token_stream()
                .to_string()
                .split(|c: char| !c.is_alphanumeric() && c != '_')
                .any(|word| word == "doc")
        {
            self.rejected.push(format!(
                "{}: conditional docs require doctests",
                self.origin.display()
            ));
        }
        if !attr.path().is_ident("doc") {
            return;
        }
        if let syn::Meta::NameValue(value) = &attr.meta {
            if !matches!(
                &value.value,
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(_),
                    ..
                })
            ) {
                self.rejected.push(format!(
                    "{}: included/generated docs require doctests",
                    self.origin.display()
                ));
                return;
            }
            // Preserve sugared block comments so rustdoc, rather than this guard,
            // strips their leading stars. Inner docs attach to a synthetic item.
            let spelling = attr.span().source_text().unwrap_or_default();
            if spelling.starts_with("///") || spelling.starts_with("/**") {
                self.current.push_str(&spelling);
            } else if spelling.starts_with("//!") || spelling.starts_with("/*!") {
                self.current.push_str(&spelling.replacen(
                    '!',
                    if spelling.starts_with("//") { "/" } else { "*" },
                    1,
                ));
            } else {
                let mut outer = attr.clone();
                outer.style = syn::AttrStyle::Outer;
                self.current.push_str(&outer.to_token_stream().to_string());
            }
            self.current.push('\n');
        }
    }
}

fn has_example(source: &str) -> bool {
    let mut examples = DocExamples::default();
    examples.inspect(Path::new("fixture.rs"), source);
    !examples.examples().is_empty()
}

fn check_sources(dir: &Path, examples: &mut DocExamples) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let kind = entry.file_type().unwrap();
        let path = entry.path();
        if kind.is_dir() {
            check_sources(&path, examples);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            examples.inspect(&path, &std::fs::read_to_string(&path).unwrap());
        }
    }
}

#[test]
fn disabled_doctest_targets_have_no_examples() {
    let _lock = super::integration_lock();
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let output = std::process::Command::new(env!("CARGO"))
        .args([
            "metadata",
            "--format-version=1",
            "--no-deps",
            "--offline",
            "--locked",
        ])
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let mut examples = DocExamples::default();
    let mut checked = 0;
    for package in metadata["packages"].as_array().unwrap() {
        for target in package["targets"].as_array().unwrap() {
            let library =
                target["kind"].as_array().unwrap().iter().any(|kind| {
                    matches!(kind.as_str(), Some("lib" | "rlib" | "staticlib" | "cdylib"))
                });
            if library && target["doctest"] == false {
                checked += 1;
                check_sources(
                    Path::new(target["src_path"].as_str().unwrap())
                        .parent()
                        .unwrap(),
                    &mut examples,
                );
            }
        }
    }
    assert!(
        checked > 0,
        "the guard must inspect disabled library targets"
    );
    let offenders = examples.examples();
    assert!(
        offenders.is_empty(),
        "Enable doctests or move examples into unit tests: {offenders:?}"
    );
}

#[test]
fn detects_examples_in_rust_doc_syntax() {
    let _lock = super::integration_lock();
    for source in [
        "/// ```\n/// assert!(true);\n/// ```\nfn example() {}",
        "//! ~~~rust,no_run\n//! example();\n//! ~~~\n",
        "//! ```text\n/// ```rust\n/// example();\n/// ```\nfn example() {}",
        "/** ```compile_fail\nexample();\n``` */ fn example() {}",
        "/**\n * ```rust\n * example();\n * ```\n */ fn example() {}",
        "/**\n *     example();\n */ fn example() {}",
        "#[doc = \"```rust\\nexample();\\n```\"] fn example() {}",
        "/// Example:\n///\n///     example();\nfn example() {}",
        "/// # Examples\n///     example();\nfn example() {}",
        "#[doc = include_str!(\"example.md\")] fn example() {}",
        "#[cfg_attr(feature = \"example\", doc = \"```rust\\nexample();\\n```\")] fn example() {}",
        "/// ```ignore\n/// example();\n/// ```\nfn example() {}",
        "macro_rules! example { () => { /// ```rust\n/// example();\n/// ```\nfn example() {} }; }",
    ] {
        assert!(has_example(source), "missed: {source}");
    }
    for source in [
        "/// Plain prose.\nfn example() {}",
        "///     prose after rustdoc removes common indentation\nfn example() {}",
        "/// A paragraph\n///     continued on the next line.\nfn example() {}",
        "/// ```text\n/// example();\n/// ```\nfn example() {}",
        "fn example() { let _ = \"/// ```rust\"; }",
        "macro_rules! example { () => { let _ = \"/// ```rust\"; }; }",
        "macro_rules! example { () => { /// Plain prose.\nfn example() {} }; }",
    ] {
        assert!(!has_example(source), "false positive: {source}");
    }
}
