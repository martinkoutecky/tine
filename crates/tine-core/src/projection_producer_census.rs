//! Executable guard for the projection-producer census.
//!
//! These tests deliberately inspect production source. They do not claim that
//! the grammar below can recognize every future filesystem API; they make the
//! currently audited grammar and architectural boundaries fail closed. A new
//! primitive, caller, native writer, process handoff, or user-selected writer
//! must update the census and this guard in the same change.

use regex::Regex;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use syn::spanned::Spanned;
use syn::visit::{self, Visit};

#[derive(Debug)]
pub(crate) struct ProductionFile {
    pub(crate) relative: String,
    /// The file exactly as it is on disk, comments and all. Use this to assert
    /// things about what the source *says*; use [`ProductionFile::code`] to
    /// assert things about what it *does*.
    pub(crate) raw: String,
    pub(crate) code: String,
    compact: String,
}

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("tine-core remains under <repo>/crates/tine-core")
        .to_path_buf()
}

fn visit_rs(directory: &Path, files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("source directory is readable") {
        let path = entry.expect("source entry is readable").path();
        if path.is_dir() {
            visit_rs(&path, files);
        } else if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
            files.push(path);
        }
    }
}

fn visit_source_extensions(directory: &Path, extensions: &[&str], files: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).expect("native source directory is readable") {
        let path = entry.expect("native source entry is readable").path();
        if path.is_dir() {
            // Tauri's Android codegen lands in a gitignored `generated/`
            // sibling of the hand-written sources after any Android build;
            // it is not shipped source and would make this guard depend on
            // whether the checkout has ever built for Android.
            if path.file_name().is_some_and(|name| name == "generated") {
                continue;
            }
            visit_source_extensions(&path, extensions, files);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extensions.contains(&extension))
        {
            files.push(path);
        }
    }
}

fn module_directory(source_path: &Path) -> PathBuf {
    match source_path.file_name().and_then(|name| name.to_str()) {
        Some("lib.rs" | "mod.rs") => source_path.parent().unwrap().to_path_buf(),
        _ => source_path
            .parent()
            .unwrap()
            .join(source_path.file_stem().unwrap()),
    }
}

fn test_only_external_modules(source_path: &Path, source: &str) -> Vec<PathBuf> {
    let module_directory = module_directory(source_path);
    let mut modules = Vec::new();
    let mut suffixes = source.split("#[cfg(test)]").skip(1).collect::<Vec<_>>();
    let mut search = source;
    while let Some(offset) = search.find("#[cfg(all(test,") {
        let tail = &search[offset..];
        let Some(end) = tail.find(']') else {
            break;
        };
        suffixes.push(&tail[end + 1..]);
        search = &tail[end + 1..];
    }
    for suffix in suffixes {
        let declaration = suffix
            .trim_start()
            .lines()
            .next()
            .unwrap_or_default()
            .trim();
        let declaration = declaration
            .strip_prefix("pub(crate) ")
            .or_else(|| declaration.strip_prefix("pub "))
            .unwrap_or(declaration);
        let Some(name) = declaration
            .strip_prefix("mod ")
            .and_then(|name| name.strip_suffix(';'))
        else {
            continue;
        };
        for candidate in [
            module_directory.join(format!("{name}.rs")),
            module_directory.join(name).join("mod.rs"),
        ] {
            if candidate.exists() {
                modules.push(candidate);
            }
        }
    }
    modules
}

/// Replace comments and string/character literal bytes with spaces while
/// retaining byte offsets. This is a small lexer, not a Rust parser; offsets
/// matter because the test-item remover applies ranges to the original source.
fn code_mask(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = bytes.to_vec();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index..].starts_with(b"//") {
            let start = index;
            index += 2;
            while index < bytes.len() && bytes[index] != b'\n' {
                index += 1;
            }
            out[start..index].fill(b' ');
            continue;
        }
        if bytes[index..].starts_with(b"/*") {
            let start = index;
            index += 2;
            let mut depth = 1_usize;
            while index < bytes.len() && depth > 0 {
                if bytes[index..].starts_with(b"/*") {
                    depth += 1;
                    index += 2;
                } else if bytes[index..].starts_with(b"*/") {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            for byte in &mut out[start..index] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            continue;
        }

        let raw_prefix = match bytes[index] {
            b'r' => Some(index + 1),
            b'b' if bytes.get(index + 1) == Some(&b'r') => Some(index + 2),
            _ => None,
        };
        if let Some(mut delimiter) = raw_prefix {
            let mut hashes = 0_usize;
            while bytes.get(delimiter) == Some(&b'#') {
                hashes += 1;
                delimiter += 1;
            }
            if bytes.get(delimiter) == Some(&b'"') {
                let start = index;
                index = delimiter + 1;
                while index < bytes.len() {
                    if bytes[index] == b'"'
                        && bytes
                            .get(index + 1..index + 1 + hashes)
                            .is_some_and(|suffix| suffix.iter().all(|byte| *byte == b'#'))
                    {
                        index += 1 + hashes;
                        break;
                    }
                    index += 1;
                }
                for byte in &mut out[start..index] {
                    if *byte != b'\n' {
                        *byte = b' ';
                    }
                }
                continue;
            }
        }

        let string_start = if bytes[index] == b'"' {
            Some(index)
        } else if bytes[index] == b'b' && bytes.get(index + 1) == Some(&b'"') {
            Some(index)
        } else {
            None
        };
        if let Some(start) = string_start {
            if bytes[index] == b'b' {
                index += 1;
            }
            index += 1;
            while index < bytes.len() {
                match bytes[index] {
                    b'\\' => index = (index + 2).min(bytes.len()),
                    b'"' => {
                        index += 1;
                        break;
                    }
                    _ => index += 1,
                }
            }
            for byte in &mut out[start..index] {
                if *byte != b'\n' {
                    *byte = b' ';
                }
            }
            continue;
        }

        let char_start = if bytes[index] == b'\'' {
            Some(index)
        } else if bytes[index] == b'b' && bytes.get(index + 1) == Some(&b'\'') {
            Some(index)
        } else {
            None
        };
        if let Some(start) = char_start {
            // The payload is NOT always one or two bytes. `'\\u{000d}'` and `'\\x41'`
            // are variable-length escapes, and `'\u{00e9}'` written literally is
            // multi-byte UTF-8. Assuming otherwise left the literal's own bytes --
            // quote, braces, digits -- unmasked in `code`, so an occurrence inside
            // an extracted function body would mis-count brace depth.
            let cursor = index + usize::from(bytes[index] == b'b') + 1;
            let payload_end = match bytes.get(cursor) {
                Some(b'\\') => match bytes.get(cursor + 1) {
                    // `\\u{XXXXXX}`: variable length, terminated by the brace.
                    Some(b'u') if bytes.get(cursor + 2) == Some(&b'{') => bytes[cursor + 3..]
                        .iter()
                        .position(|byte| *byte == b'}')
                        .map(|offset| cursor + 3 + offset + 1),
                    // `\\xNN`: always two hex digits.
                    Some(b'x') => Some(cursor + 4),
                    // Every other escape is one character after the backslash.
                    Some(_) => Some(cursor + 2),
                    None => None,
                },
                // A literal character, possibly multi-byte UTF-8.
                Some(_) => {
                    let mut end = cursor + 1;
                    while bytes
                        .get(end)
                        .is_some_and(|byte| byte & 0b1100_0000 == 0b1000_0000)
                    {
                        end += 1;
                    }
                    Some(end)
                }
                None => None,
            };
            if let Some(cursor) = payload_end {
                if bytes.get(cursor) == Some(&b'\'') {
                    index = cursor + 1;
                    out[start..index].fill(b' ');
                    continue;
                }
            }
        }
        index += 1;
    }
    String::from_utf8(out).expect("mask preserves UTF-8 bytes")
}

fn matching_brace(mask: &str, open: usize) -> Option<usize> {
    let bytes = mask.as_bytes();
    let mut depth = 0_usize;
    for (offset, byte) in bytes[open..].iter().enumerate() {
        match byte {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(open + offset + 1);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        if attribute.path().is_ident("test") {
            return true;
        }
        if attribute.path().segments.len() == 2
            && attribute.path().segments[0].ident == "tokio"
            && attribute.path().segments[1].ident == "test"
        {
            return true;
        }
        if !attribute.path().is_ident("cfg") {
            return false;
        }
        let syn::Meta::List(list) = &attribute.meta else {
            return false;
        };
        let predicate = list.tokens.to_string().replace(' ', "");
        predicate == "test" || predicate.starts_with("all(test,")
    })
}

#[derive(Default)]
struct TestOnlyRanges {
    ranges: Vec<(proc_macro2::LineColumn, proc_macro2::LineColumn)>,
}

impl TestOnlyRanges {
    fn omit<T: Spanned>(&mut self, node: &T, attributes: &[syn::Attribute]) -> bool {
        if !is_test_only(attributes) {
            return false;
        }
        let start = attributes
            .first()
            .map_or_else(|| node.span().start(), |attribute| attribute.span().start());
        self.ranges.push((start, node.span().end()));
        true
    }
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        syn::Item::Verbatim(_) => &[],
        _ => &[],
    }
}

fn impl_item_attributes(item: &syn::ImplItem) -> &[syn::Attribute] {
    match item {
        syn::ImplItem::Const(item) => &item.attrs,
        syn::ImplItem::Fn(item) => &item.attrs,
        syn::ImplItem::Type(item) => &item.attrs,
        syn::ImplItem::Macro(item) => &item.attrs,
        syn::ImplItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn trait_item_attributes(item: &syn::TraitItem) -> &[syn::Attribute] {
    match item {
        syn::TraitItem::Const(item) => &item.attrs,
        syn::TraitItem::Fn(item) => &item.attrs,
        syn::TraitItem::Type(item) => &item.attrs,
        syn::TraitItem::Macro(item) => &item.attrs,
        syn::TraitItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn foreign_item_attributes(item: &syn::ForeignItem) -> &[syn::Attribute] {
    match item {
        syn::ForeignItem::Fn(item) => &item.attrs,
        syn::ForeignItem::Static(item) => &item.attrs,
        syn::ForeignItem::Type(item) => &item.attrs,
        syn::ForeignItem::Macro(item) => &item.attrs,
        syn::ForeignItem::Verbatim(_) => &[],
        _ => &[],
    }
}

fn expr_attributes(expr: &syn::Expr) -> &[syn::Attribute] {
    match expr {
        syn::Expr::Array(expr) => &expr.attrs,
        syn::Expr::Assign(expr) => &expr.attrs,
        syn::Expr::Async(expr) => &expr.attrs,
        syn::Expr::Await(expr) => &expr.attrs,
        syn::Expr::Binary(expr) => &expr.attrs,
        syn::Expr::Block(expr) => &expr.attrs,
        syn::Expr::Break(expr) => &expr.attrs,
        syn::Expr::Call(expr) => &expr.attrs,
        syn::Expr::Cast(expr) => &expr.attrs,
        syn::Expr::Closure(expr) => &expr.attrs,
        syn::Expr::Const(expr) => &expr.attrs,
        syn::Expr::Continue(expr) => &expr.attrs,
        syn::Expr::Field(expr) => &expr.attrs,
        syn::Expr::ForLoop(expr) => &expr.attrs,
        syn::Expr::Group(expr) => &expr.attrs,
        syn::Expr::If(expr) => &expr.attrs,
        syn::Expr::Index(expr) => &expr.attrs,
        syn::Expr::Infer(expr) => &expr.attrs,
        syn::Expr::Let(expr) => &expr.attrs,
        syn::Expr::Lit(expr) => &expr.attrs,
        syn::Expr::Loop(expr) => &expr.attrs,
        syn::Expr::Macro(expr) => &expr.attrs,
        syn::Expr::Match(expr) => &expr.attrs,
        syn::Expr::MethodCall(expr) => &expr.attrs,
        syn::Expr::Paren(expr) => &expr.attrs,
        syn::Expr::Path(expr) => &expr.attrs,
        syn::Expr::Range(expr) => &expr.attrs,
        syn::Expr::RawAddr(expr) => &expr.attrs,
        syn::Expr::Reference(expr) => &expr.attrs,
        syn::Expr::Repeat(expr) => &expr.attrs,
        syn::Expr::Return(expr) => &expr.attrs,
        syn::Expr::Struct(expr) => &expr.attrs,
        syn::Expr::Try(expr) => &expr.attrs,
        syn::Expr::TryBlock(expr) => &expr.attrs,
        syn::Expr::Tuple(expr) => &expr.attrs,
        syn::Expr::Unary(expr) => &expr.attrs,
        syn::Expr::Unsafe(expr) => &expr.attrs,
        syn::Expr::Verbatim(_) => &[],
        syn::Expr::While(expr) => &expr.attrs,
        syn::Expr::Yield(expr) => &expr.attrs,
        _ => &[],
    }
}

impl<'ast> Visit<'ast> for TestOnlyRanges {
    fn visit_item(&mut self, node: &'ast syn::Item) {
        if !self.omit(node, item_attributes(node)) {
            visit::visit_item(self, node);
        }
    }

    fn visit_impl_item(&mut self, node: &'ast syn::ImplItem) {
        if !self.omit(node, impl_item_attributes(node)) {
            visit::visit_impl_item(self, node);
        }
    }

    fn visit_trait_item(&mut self, node: &'ast syn::TraitItem) {
        if !self.omit(node, trait_item_attributes(node)) {
            visit::visit_trait_item(self, node);
        }
    }

    fn visit_foreign_item(&mut self, node: &'ast syn::ForeignItem) {
        if !self.omit(node, foreign_item_attributes(node)) {
            visit::visit_foreign_item(self, node);
        }
    }

    fn visit_field(&mut self, node: &'ast syn::Field) {
        if !self.omit(node, &node.attrs) {
            visit::visit_field(self, node);
        }
    }

    fn visit_variant(&mut self, node: &'ast syn::Variant) {
        if !self.omit(node, &node.attrs) {
            visit::visit_variant(self, node);
        }
    }

    fn visit_local(&mut self, node: &'ast syn::Local) {
        if !self.omit(node, &node.attrs) {
            visit::visit_local(self, node);
        }
    }

    fn visit_stmt_macro(&mut self, node: &'ast syn::StmtMacro) {
        if !self.omit(node, &node.attrs) {
            visit::visit_stmt_macro(self, node);
        }
    }

    fn visit_arm(&mut self, node: &'ast syn::Arm) {
        if !self.omit(node, &node.attrs) {
            visit::visit_arm(self, node);
        }
    }

    fn visit_expr(&mut self, node: &'ast syn::Expr) {
        if !self.omit(node, expr_attributes(node)) {
            visit::visit_expr(self, node);
        }
    }
}

fn byte_offset(line_starts: &[usize], location: proc_macro2::LineColumn) -> usize {
    line_starts[location.line - 1] + location.column
}

/// Blank syntax nodes disabled in tests while preserving byte offsets and lines.
/// A syntax-aware walk matters: cfg(test) is legal on fields, match arms, local
/// declarations, and expressions as well as whole items.
fn without_test_items(source: &str) -> String {
    let parsed = syn::parse_file(source).expect("production Rust source parses");
    let mut omitted = TestOnlyRanges::default();
    omitted.visit_file(&parsed);
    let mut line_starts = vec![0];
    for (index, byte) in source.bytes().enumerate() {
        if byte == b'\n' {
            line_starts.push(index + 1);
        }
    }
    let mut bytes = source.as_bytes().to_vec();
    for (start, end) in omitted.ranges {
        let start = byte_offset(&line_starts, start);
        let end = byte_offset(&line_starts, end);
        for byte in &mut bytes[start..end] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    }
    String::from_utf8(bytes).expect("blanking preserves UTF-8")
}

/// Every production Rust source in `tine-core` and `src-tauri`, with test items
/// and comment/literal bytes masked out.
///
/// Shared with the rest of the crate so a "no production caller" / "only caller"
/// claim anywhere can be asserted against the same definition of "production"
/// this census uses, instead of each site inventing its own file scan.
///
/// The scan reads and syntax-parses the whole tree, so it is computed once per
/// test process rather than once per assertion.
pub(crate) fn production_rust() -> &'static [ProductionFile] {
    static SCANNED: std::sync::OnceLock<Vec<ProductionFile>> = std::sync::OnceLock::new();
    SCANNED.get_or_init(scan_production_rust)
}

/// Every Rust source in the two crates the census walks, tests included, as
/// `(repository-relative path, raw source)`.
///
/// [`production_rust`] deliberately drops test-only files; a guard that has to
/// ask "does the thing this comment names exist anywhere?" needs them back.
pub(crate) fn repository_rust_sources() -> &'static [(String, String)] {
    static SCANNED: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    SCANNED.get_or_init(|| {
        let repo = repository_root();
        let mut paths = Vec::new();
        for root in [
            repo.join("crates/tine-core/src"),
            repo.join("crates/tine-core/tests"),
            repo.join("src-tauri/src"),
        ] {
            if root.is_dir() {
                visit_rs(&root, &mut paths);
            }
        }
        paths.sort();
        paths
            .into_iter()
            .map(|path| {
                let relative = path
                    .strip_prefix(&repo)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                let source = fs::read_to_string(&path).expect("Rust source is readable");
                (relative, source)
            })
            .collect()
    })
}

fn scan_production_rust() -> Vec<ProductionFile> {
    let repo = repository_root();
    let roots = [
        repo.join("crates/tine-core/src"),
        repo.join("src-tauri/src"),
    ];
    let mut paths = Vec::new();
    for root in &roots {
        visit_rs(root, &mut paths);
    }
    paths.sort();

    let test_only = paths
        .iter()
        .flat_map(|path| {
            let source = fs::read_to_string(path).expect("Rust source is readable");
            test_only_external_modules(path, &source)
        })
        .collect::<BTreeSet<_>>();

    paths
        .into_iter()
        .filter(|path| !test_only.contains(path))
        .filter(|path| {
            let relative = path.strip_prefix(&repo).unwrap().to_string_lossy();
            !relative.contains("/tests/")
                && !relative.contains("/benches/")
                && !relative.ends_with("_tests.rs")
        })
        .map(|path| {
            let relative = path
                .strip_prefix(&repo)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let source = fs::read_to_string(&path).expect("Rust source is readable");
            let code = code_mask(&without_test_items(&source));
            let compact = code
                .chars()
                .filter(|character| !character.is_whitespace())
                .collect();
            ProductionFile {
                relative,
                raw: source,
                code,
                compact,
            }
        })
        .collect()
}

fn token_inventory(
    files: &[ProductionFile],
    tokens: &[(&'static str, &'static str)],
) -> Vec<(String, String, usize)> {
    let mut inventory = Vec::new();
    for file in files {
        for (name, token) in tokens {
            let count = file.compact.matches(token).count();
            if count != 0 {
                inventory.push((file.relative.clone(), (*name).to_owned(), count));
            }
        }
    }
    inventory.sort();
    inventory
}

fn tine_storage_surface_inventory(files: &[ProductionFile]) -> Vec<(String, String, usize)> {
    let direct_call =
        Regex::new(r"tine_storage(?:::[A-Za-z_][A-Za-z0-9_]*)+\(").expect("static regex");
    let mut inventory = Vec::new();
    for file in files {
        for matched in direct_call.find_iter(&file.compact) {
            let token = matched.as_str().to_owned();
            if let Some((_, _, count)) = inventory
                .iter_mut()
                .find(|(path, existing, _)| path == &file.relative && existing == &token)
            {
                *count += 1;
            } else {
                inventory.push((file.relative.clone(), token, 1));
            }
        }
        let mut offset = 0;
        while let Some(relative) = file.compact[offset..].find("usetine_storage") {
            let start = offset + relative;
            let end = start
                + file.compact[start..]
                    .find(';')
                    .expect("use declaration ends with semicolon")
                + 1;
            inventory.push((
                file.relative.clone(),
                file.compact[start..end].to_owned(),
                1,
            ));
            offset = end;
        }
    }
    inventory.sort();
    inventory
}

fn tine_storage_imported_call_inventory(files: &[ProductionFile]) -> Vec<(String, String, usize)> {
    let identifier = Regex::new(r"[A-Za-z_][A-Za-z0-9_]*").expect("static regex");
    let imports_of = |file: &ProductionFile| {
        tine_storage_surface_inventory(std::slice::from_ref(file))
            .into_iter()
            .filter_map(|(_, token, _)| token.starts_with("usetine_storage").then_some(token))
            .collect::<Vec<_>>()
    };
    let mut inventory = Vec::new();
    for file in files {
        let mut imports = imports_of(file);
        // A module that opens with `use super::*;` calls its parent's imports
        // too. K3 split model.rs into such seam modules; without this, a
        // `DurableDirectoryPublication::open(` moved into model/atomic_copy.rs
        // silently left the surface instead of moving on it.
        if file.compact.contains("usesuper::*;") {
            let parent = file
                .relative
                .rsplit_once('/')
                .map(|(directory, _)| format!("{directory}.rs"));
            if let Some(parent) = files
                .iter()
                .find(|candidate| Some(&candidate.relative) == parent.as_ref())
            {
                imports.extend(imports_of(parent));
            }
        }
        if imports.is_empty() {
            continue;
        }
        let imported = imports
            .iter()
            .flat_map(|declaration| identifier.find_iter(declaration))
            .map(|matched| matched.as_str().to_owned())
            .filter(|name| !matches!(name.as_str(), "use" | "tine_storage" | "as" | "self"))
            .collect::<BTreeSet<_>>();

        for name in &imported {
            let direct = identifier_occurrences(&file.code, &format!("{name}("));
            if direct != 0 {
                inventory.push((file.relative.clone(), format!("import-call:{name}"), direct));
            }
            let associated = Regex::new(&format!(
                r"{}::[A-Za-z_][A-Za-z0-9_]*\(",
                regex::escape(name)
            ))
            .unwrap();
            for matched in associated.find_iter(&file.compact) {
                inventory.push((
                    file.relative.clone(),
                    format!("import-associated:{}", matched.as_str()),
                    1,
                ));
            }
        }

        let write_capable_types = BTreeSet::from([
            "DurableDirectoryPublication",
            "ExactImmutablePublicationBatch",
            "LocalJournalSegment",
            "LocalJournalSegmentV2",
            "PatriciaIndexConstruction",
            "PatriciaIndexStore",
            "PhysicalGraphProjectionDatabase",
            "ScratchRun",
            "SqliteFileSet",
            "StagedExactImmutablePublication",
        ]);
        let qualified_type =
            Regex::new(r"tine_storage::([A-Z][A-Za-z0-9_]*)").expect("static regex");
        let mut candidate_types = imported.clone();
        candidate_types.extend(
            qualified_type
                .captures_iter(&file.compact)
                .map(|captures| captures[1].to_owned()),
        );
        let storage_types = candidate_types
            .iter()
            .filter(|name| write_capable_types.contains(name.as_str()))
            .collect::<Vec<_>>();
        let mut receivers = BTreeSet::new();
        for storage_type in storage_types {
            let typed = Regex::new(&format!(
                r"([a-z_][A-Za-z0-9_]*):(?:&mut|&)?(?:[A-Za-z_][A-Za-z0-9_]*<)*(?:tine_storage::)?{}(?:<[^;{{}}()]*?>)?(?:[>,])",
                regex::escape(storage_type)
            ))
            .unwrap();
            for captures in typed.captures_iter(&file.compact) {
                receivers.insert(captures[1].to_owned());
            }
            let constructed = Regex::new(&format!(
                r"let(?:mut)?([a-z_][A-Za-z0-9_]*)=(?:tine_storage::)?{}::[A-Za-z_][A-Za-z0-9_]*\(",
                regex::escape(storage_type)
            ))
            .unwrap();
            for captures in constructed.captures_iter(&file.compact) {
                receivers.insert(captures[1].to_owned());
            }
        }
        for receiver in receivers {
            let methods = Regex::new(&format!(
                r"(?:self\.)?{}\.([A-Za-z_][A-Za-z0-9_]*)\(",
                regex::escape(&receiver)
            ))
            .unwrap();
            for captures in methods.captures_iter(&file.compact) {
                inventory.push((
                    file.relative.clone(),
                    format!("storage-receiver:{receiver}.{}", &captures[1]),
                    1,
                ));
            }
        }
    }
    inventory.sort();
    inventory
}

fn inventory_digest(inventory: &[(String, String, usize)]) -> String {
    let mut hasher = Sha256::new();
    for (path, token, count) in inventory {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update(token.as_bytes());
        hasher.update([0]);
        hasher.update(count.to_le_bytes());
        hasher.update([b'\n']);
    }
    hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// How many times production code calls `name`, not counting its definition.
pub(crate) fn call_count(files: &[ProductionFile], name: &str) -> usize {
    let call = format!("{name}(");
    let definition = format!("fn {name}(");
    files
        .iter()
        .map(|file| {
            identifier_occurrences(&file.code, &call) - file.code.matches(&definition).count()
        })
        .sum()
}

fn identifier_occurrences(source: &str, needle: &str) -> usize {
    let mut count = 0;
    let mut offset = 0;
    while let Some(relative) = source[offset..].find(needle) {
        let start = offset + relative;
        let boundary = start == 0
            || !source.as_bytes()[start - 1].is_ascii_alphanumeric()
                && source.as_bytes()[start - 1] != b'_';
        count += usize::from(boundary);
        offset = start + needle.len();
    }
    count
}

fn function_process_handoffs(files: &[ProductionFile], name: &str) -> usize {
    let definition = format!("fn{name}(");
    let mut total = 0;
    for file in files {
        let mut offset = 0;
        while let Some(relative) = file.compact[offset..].find(&definition) {
            let start = offset + relative;
            let open = start
                + file.compact[start..]
                    .find('{')
                    .expect("function definition has a body");
            let end = matching_brace(&file.compact, open).expect("function body is balanced");
            let body = &file.compact[open..end];
            total += body.matches(".spawn(").count() + body.matches(".status(").count();
            offset = end;
        }
    }
    total
}

fn function_bodies<'a>(files: &'a [ProductionFile], name: &str) -> Vec<&'a str> {
    let definition = format!("fn{name}(");
    let mut bodies = Vec::new();
    for file in files {
        let mut offset = 0;
        while let Some(relative) = file.compact[offset..].find(&definition) {
            let start = offset + relative;
            let open = start
                + file.compact[start..]
                    .find('{')
                    .expect("function definition has a body");
            let end = matching_brace(&file.compact, open).expect("function body is balanced");
            bodies.push(&file.compact[open..end]);
            offset = end;
        }
    }
    bodies
}

#[test]
fn g_a_mutation_primitive_counts_are_pinned_per_file() {
    let actual = token_inventory(
        production_rust(),
        &[
            ("cap.rename", ".rename("),
            ("cap.remove_file", ".remove_file("),
            ("cap.create_dir", ".create_dir("),
            ("cap.create_dir_all", ".create_dir_all("),
            ("cap.hard_link", ".hard_link("),
            ("cap.remove_dir", ".remove_dir("),
            ("cap.remove_dir_all", ".remove_dir_all("),
            ("fs.rename", "fs::rename("),
            ("fs.remove_file", "fs::remove_file("),
            ("fs.create_dir", "fs::create_dir("),
            ("fs.create_dir_all", "fs::create_dir_all("),
            ("fs.dir_builder", "DirBuilder::new("),
            ("fs.hard_link", "fs::hard_link("),
            ("fs.remove_dir", "fs::remove_dir("),
            ("fs.remove_dir_all", "fs::remove_dir_all("),
            ("fs.write", "fs::write("),
            ("fs.copy", "fs::copy("),
            ("libc.renameat", "libc::renameat("),
            ("libc.unlinkat", "libc::unlinkat("),
            ("libc.mkdirat", "libc::mkdirat("),
            ("libc.linkat", "libc::linkat("),
            ("libc.openat.create", "libc::O_CREAT"),
            ("libc.renameat2", "libc::SYS_renameat2"),
            ("open.create", ".create(true)"),
            ("open.create_new", ".create_new(true)"),
            ("open.truncate", ".truncate(true)"),
            ("file.create", "File::create("),
            ("file.set_len", ".set_len("),
            ("windows.MoveFileW", "MoveFileW("),
            ("windows.CreateDirectoryW", "CreateDirectoryW("),
            ("windows.NtSetInformationFile", "NtSetInformationFile("),
            (
                "windows.SetFileInformationByHandle",
                "SetFileInformationByHandle(",
            ),
        ],
    );
    let expected = [
        (
            "crates/tine-core/src/bin/export-block-raws.rs",
            "fs.write",
            1,
        ),
        (
            "crates/tine-core/src/concord_ledger.rs",
            "fs.create_dir_all",
            1,
        ),
        // 5 since 8eb922d8 ("prune reclaims ledger entries whose blob is
        // gone"): prune now retires a dangling entry and a corrupt entry as
        // two separate named cases beside the three prior removals.
        (
            "crates/tine-core/src/concord_ledger.rs",
            "fs.remove_file",
            5,
        ),
        ("crates/tine-core/src/concord_ledger.rs", "fs.rename", 1),
        ("crates/tine-core/src/concord_ledger.rs", "fs.write", 1),
        // The Direct cross-page move recovery store (packet B2). Its GRAPH
        // writes are not here because they are not raw primitives: every page
        // byte it publishes goes through `model::atomic_write`, which is why
        // this file appears in the `atomic_write` caller count below. What is
        // left is the app-private store's own housekeeping — creating its three
        // subdirectories, retiring a record, dropping an unreferenced blob,
        // quarantining a malformed record — plus the one primitive an atomic
        // write cannot express: removing a page file when rolling a move back
        // to "this page did not exist". See
        // `docs/contracts/direct-move-recovery.md`.
        (
            "crates/tine-core/src/direct_move_recovery.rs",
            "fs.create_dir_all",
            5,
        ),
        (
            "crates/tine-core/src/direct_move_recovery.rs",
            "fs.remove_file",
            4,
        ),
        (
            "crates/tine-core/src/direct_move_recovery.rs",
            "fs.rename",
            1,
        ),
        // 2026-09-20: P4 removes exact owned unpublished stage files and their
        // known SQLite sidecars through the directory capability. Prefix-only
        // matches are preserved, and storage owns destination publication.
        (
            "crates/tine-core/src/direct_projection.rs",
            "cap.remove_file",
            3,
        ),
        (
            "crates/tine-core/src/direct_projection.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/direct_projection.rs",
            "open.create",
            1,
        ),
        // K5 (2026-09-15) moved the shared atomic filesystem machinery out of
        // model; only its production path changed, not its primitive totals.
        (
            "crates/tine-core/src/filesystem_durability.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            // 9 and 3 since GH #538: the conditional replace removes its
            // staged probe temp on refusal, and `restore_vacated_name` falls
            // back to a plain rename of a vacated name when the storage
            // refuses the no-replace flag.
            "crates/tine-core/src/filesystem_durability/atomic_fs.rs",
            "fs.remove_file",
            9,
        ),
        (
            "crates/tine-core/src/filesystem_durability/atomic_fs.rs",
            "fs.rename",
            3,
        ),
        (
            "crates/tine-core/src/filesystem_durability/atomic_fs.rs",
            "libc.renameat2",
            1,
        ),
        (
            "crates/tine-core/src/filesystem_durability/atomic_fs.rs",
            "open.create_new",
            2,
        ),
        (
            "crates/tine-core/src/filesystem_durability/atomic_fs.rs",
            "windows.MoveFileW",
            1,
        ),
        // K3 (2026-09-15) moved these out of model.rs verbatim; the module's
        // totals did not change.
        (
            "crates/tine-core/src/model/assets.rs",
            "fs.create_dir_all",
            3,
        ),
        (
            "crates/tine-core/src/model/assets.rs",
            "fs.remove_dir_all",
            1,
        ),
        ("crates/tine-core/src/model/assets.rs", "fs.remove_file", 2),
        (
            "crates/tine-core/src/model/atomic_copy.rs",
            "cap.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/model/atomic_copy.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/model/atomic_copy.rs",
            "fs.remove_file",
            3,
        ),
        ("crates/tine-core/src/model/atomic_copy.rs", "fs.rename", 1),
        (
            "crates/tine-core/src/model/atomic_copy.rs",
            "open.create_new",
            3,
        ),
        (
            "crates/tine-core/src/model/conflicts.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/model/conflicts.rs",
            "fs.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/model/graph_text_writes.rs",
            "cap.remove_file",
            11,
        ),
        (
            "crates/tine-core/src/model/open_graph.rs",
            "cap.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/model/pages_merge.rs",
            "fs.create_dir_all",
            1,
        ),
        ("crates/tine-core/src/model/pdf.rs", "fs.create_dir_all", 6),
        (
            "crates/tine-core/src/model/projection_rename.rs",
            "cap.create_dir",
            1,
        ),
        (
            "crates/tine-core/src/model/projection_rename.rs",
            "cap.remove_file",
            2,
        ),
        (
            "crates/tine-core/src/model/projection_rename.rs",
            "libc.renameat2",
            2,
        ),
        (
            "crates/tine-core/src/model/projection_rename.rs",
            "open.create_new",
            2,
        ),
        (
            "crates/tine-core/src/model/projection_rename.rs",
            "windows.NtSetInformationFile",
            1,
        ),
        (
            "crates/tine-core/src/model/retired_files.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/model/trash.rs",
            "fs.create_dir_all",
            1,
        ),
        ("crates/tine-core/src/onboarding.rs", "fs.create_dir_all", 4),
        ("crates/tine-core/src/publish.rs", "cap.create_dir", 2),
        // Stage-side parents: the recovery slot, a query export's parent
        // (`published-queries/`) and copied-asset subdirectories, all inside
        // bound handles.
        ("crates/tine-core/src/publish.rs", "cap.create_dir_all", 3),
        // Retire a previous occupant into recovery, plus the create-only
        // path's own-stage retirement when another export appeared.
        ("crates/tine-core/src/publish.rs", "cap.rename", 3),
        // Owner-private temporary query storage: Unix creates through a mode
        // 0700 builder; Windows creates with an explicit protected DACL.
        ("crates/tine-core/src/publish.rs", "fs.dir_builder", 1),
        ("crates/tine-core/src/publish.rs", "fs.remove_dir_all", 1),
        // Page files and copied assets are each written create-new into the
        // stage handle.
        ("crates/tine-core/src/publish.rs", "open.create_new", 2),
        (
            "crates/tine-core/src/publish/private_directory.rs",
            "fs.remove_dir",
            1,
        ),
        (
            "crates/tine-core/src/publish/private_directory.rs",
            "windows.CreateDirectoryW",
            1,
        ),
        ("src-tauri/src/backup.rs", "cap.create_dir", 2),
        // Packet R1 gives Windows the same no-clobber hard-link publication
        // shape as Unix and removes the replacement-style rename fallback.
        ("src-tauri/src/backup.rs", "cap.hard_link", 2),
        ("src-tauri/src/backup.rs", "cap.remove_file", 3),
        ("src-tauri/src/backup.rs", "fs.copy", 3),
        ("src-tauri/src/backup.rs", "fs.create_dir", 1),
        ("src-tauri/src/backup.rs", "fs.create_dir_all", 5),
        ("src-tauri/src/backup.rs", "fs.remove_dir_all", 3),
        ("src-tauri/src/backup.rs", "fs.remove_file", 1),
        ("src-tauri/src/backup.rs", "fs.rename", 2),
        ("src-tauri/src/backup.rs", "libc.renameat2", 1),
        ("src-tauri/src/backup.rs", "open.create_new", 3),
        ("src-tauri/src/commands.rs", "cap.remove_file", 1),
        // Packet B3: the app-private live-save conflict envelope. Its
        // directory is created before the audited atomic replacement, torn
        // temporaries and the retired final file are removed, and an
        // unreadable envelope is renamed aside rather than deleted.
        ("src-tauri/src/conflict_capsule.rs", "fs.create_dir_all", 1),
        ("src-tauri/src/conflict_capsule.rs", "fs.remove_file", 2),
        ("src-tauri/src/conflict_capsule.rs", "fs.rename", 1),
        ("src-tauri/src/data_home.rs", "fs.create_dir_all", 1),
        ("src-tauri/src/data_home.rs", "fs.remove_file", 1),
        ("src-tauri/src/data_home.rs", "fs.write", 1),
        ("src-tauri/src/debug.rs", "fs.create_dir_all", 1),
        ("src-tauri/src/debug.rs", "fs.remove_file", 5),
        ("src-tauri/src/debug.rs", "fs.rename", 3),
        ("src-tauri/src/debug.rs", "open.create", 1),
        ("src-tauri/src/graph.rs", "fs.create_dir", 1),
        ("src-tauri/src/graph.rs", "fs.create_dir_all", 1),
        (
            "src-tauri/src/linux_window_identity.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "src-tauri/src/linux_window_identity.rs",
            "fs.remove_file",
            1,
        ),
        ("src-tauri/src/linux_window_identity.rs", "fs.rename", 1),
        (
            "src-tauri/src/linux_window_identity.rs",
            "open.create_new",
            1,
        ),
        ("src-tauri/src/migrate_identifier.rs", "fs.copy", 1),
        (
            "src-tauri/src/migrate_identifier.rs",
            "fs.create_dir_all",
            2,
        ),
        (
            "src-tauri/src/migrate_identifier.rs",
            "fs.remove_dir_all",
            4,
        ),
        ("src-tauri/src/migrate_identifier.rs", "fs.rename", 4),
        // Packet B5p moved every plugin-package mutation behind tine-storage's
        // package protocol (see g_d); `plugins.rs` has no raw primitive left.
        // +1 from packet P2 (query engine): `save_notices_at` creates the
        // `sessions/` directory before publishing the device-local notice
        // dismissals, exactly as the session and workspace publishers do.
        ("src-tauri/src/settings.rs", "fs.create_dir_all", 4),
        // Packet B5s collapses the settings, workspace, and session publishers
        // onto the shared atomic writer. Only the audited legacy-session move
        // still needs a raw rename in this file.
        ("src-tauri/src/settings.rs", "fs.rename", 1),
    ]
    .into_iter()
    .map(|(path, primitive, count)| (path.to_owned(), primitive.to_owned(), count))
    .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "update the census before accepting a primitive delta"
    );
}

#[test]
fn g_b_choke_helper_caller_counts_are_pinned() {
    let files = production_rust();
    let roots = [
        "graph_text_atomic_create_with_proof",
        "graph_text_atomic_write_validated",
        "graph_text_atomic_replace_bound",
        "rename_projection_noreplace_platform",
        "rename_graph_text_noreplace",
        "atomic_publish",
        "atomic_write",
        "atomic_write_new",
        "atomic_replace_expected_with_hooks",
        "atomic_copy",
        "atomic_copy_new",
        "atomic_copy_file_new",
        "move_file_noreplace",
        "move_to_trash",
        "create_projection_chain_component",
        "empty_asset_trash",
        "reserve_publish_stage",
        "reserve_publish_recovery",
        "commit_publish_stage",
        "write_publish_stage_file",
        "stage",
        "write_config",
        "atomic_update",
        "create_graph",
        "create_demo_graph",
        "reserve_restore_recovery",
        "open_or_create_real_parent",
        "rename_noreplace_between",
        "publish_temp_noreplace",
        "atomic_copy_new_into_live",
        "move_live_to_recovery",
    ];
    let actual = roots
        .into_iter()
        .map(|name| (name, call_count(&files, name)))
        .collect::<Vec<_>>();
    let expected = vec![
        ("graph_text_atomic_create_with_proof", 2),
        ("graph_text_atomic_write_validated", 2),
        ("graph_text_atomic_replace_bound", 2),
        ("rename_projection_noreplace_platform", 1),
        ("rename_graph_text_noreplace", 2),
        ("atomic_publish", 2),
        // +4 from `direct_move_recovery.rs` (packet B2): the record, each
        // content-addressed image blob, a quarantined record, and every page a
        // recovery completes or rolls back. Recovery writes graph bytes through
        // the SAME named audited protocol an ordinary save uses.
        // +2 from `settings.rs` (packet B5s): the workspace registry and scoped
        // session saves now use that same named audited protocol.
        // +1 from `src-tauri/src/conflict_capsule.rs` (packet B3): the
        // app-private live-save conflict envelope is replaced whole through
        // the same named audited protocol.
        // +1 from `settings.rs` (packet P2): the device-local notice
        // dismissals are published through that same named audited protocol.
        ("atomic_write", 14),
        ("atomic_write_new", 10),
        ("atomic_replace_expected_with_hooks", 1),
        ("atomic_copy", 0),
        ("atomic_copy_new", 1),
        ("atomic_copy_file_new", 1),
        // 11 since GH #538: the conditional replace and the retired-file
        // restore take the mover as a parameter (a test seam for storage that
        // refuses the flag); production binds it to `move_file_noreplace`.
        ("move_file_noreplace", 11),
        ("move_to_trash", 3),
        ("create_projection_chain_component", 1),
        ("empty_asset_trash", 1),
        ("reserve_publish_stage", 1),
        ("reserve_publish_recovery", 3),
        ("commit_publish_stage", 1),
        // 9: a query export's app bake adds the stage-root redirect script
        // that sends an HTTP visitor from the static front door to `app/`.
        ("write_publish_stage_file", 9),
        ("stage", 2),
        ("write_config", 9),
        ("atomic_update", 3),
        ("create_graph", 0),
        ("create_demo_graph", 1),
        ("reserve_restore_recovery", 2),
        ("open_or_create_real_parent", 8),
        ("rename_noreplace_between", 2),
        ("publish_temp_noreplace", 1),
        ("atomic_copy_new_into_live", 4),
        ("move_live_to_recovery", 7),
    ];
    assert_eq!(
        actual, expected,
        "update the producer-family census with every caller delta"
    );
}

#[test]
fn g_c_producer_classes_keep_representative_entrypoints_and_negative_gates() {
    let repo = repository_root();
    let files = production_rust();
    let representatives = [
        (
            "PC-1",
            "crates/tine-core/src/model/save_path.rs",
            "fnsave_page(",
        ),
        (
            "PC-7",
            "src-tauri/src/watcher.rs",
            "fnobserve_graph_text_event(",
        ),
        (
            "PC-8",
            "crates/tine-core/src/model/queries.rs",
            "fnpublish_html(",
        ),
        (
            "PC-9",
            "src-tauri/src/commands.rs",
            "fnapply_journal_filename_migrations(",
        ),
        (
            "PC-11",
            "src-tauri/src/commands.rs",
            "fnset_preferred_workflow(",
        ),
        ("PC-12", "src-tauri/src/graph.rs", "fncreate_graph("),
        ("PC-13", "src-tauri/src/backup.rs", "fnrestore_backup("),
        (
            "PC-18",
            "src-tauri/src/commands.rs",
            "fnedit_asset_external(",
        ),
        (
            "PC-19",
            "src-tauri/src/debug.rs",
            "fnsave_diagnostic_report(",
        ),
        (
            "PC-20",
            "crates/tine-core/src/bin/export-block-raws.rs",
            "fnmain(",
        ),
    ];
    for (class, path, needle) in representatives {
        let source = files
            .iter()
            .find(|file| file.relative == path)
            .unwrap_or_else(|| panic!("{class} lost production source {path}"));
        assert!(
            source.compact.contains(needle),
            "{class} lost representative {path}:{needle}"
        );
    }
    assert!(fs::read_to_string(
        repo.join("src-tauri/ios-folder-picker-native/ios/Sources/GraphFolderPickerPlugin.swift")
    )
    .unwrap()
    .contains(".tine-container"));
    let restore = function_bodies(&files, "restore_backup");
    assert_eq!(restore.len(), 1, "PC-13 restore entry remains unique");
    assert!(
        restore[0].contains("slot.graph("),
        "PC-13 restore reads its source from the bound slot\'s graph"
    );
}

#[test]
fn g_d_tine_storage_write_boundaries_are_pinned() {
    let files = production_rust();
    let actual = token_inventory(
        &files,
        &[
            (
                "immutable.single_writer",
                "publish_immutable_exact_single_writer(",
            ),
            ("immutable.batch", "ExactImmutablePublicationBatch::new("),
            (
                "durable_directory.open",
                "DurableDirectoryPublication::open(",
            ),
            ("journal.v1.open", "LocalJournalSegment::open("),
            ("journal.v2.prepare", "::prepare_single_writer("),
            ("journal.v2.open", "LocalJournalSegmentV2::open_selected("),
            ("journal.fast_append", "self.segment.append("),
            ("journal.payload_append", ".append(payload_kind,payload)"),
            ("journal.turn_append", "self.journal.append("),
            ("package.publish", "publish_package_noclobber("),
            ("package.recover", "recover_package_store("),
            ("package.retire", "retire_package("),
        ],
    );
    let expected = [
        (
            "crates/tine-core/src/direct_projection.rs",
            "durable_directory.open",
            1,
        ),
        // GH #466: the three Direct Files graph-text sites (create, validated
        // write, bounded replace) left this boundary — its Android arm is a
        // hard link that shared storage refuses — for the graph tree's own
        // no-clobber rename (`move_graph_text_exact_no_replace`). The two
        // remaining opens are the app-private durable authorities.
        (
            "crates/tine-core/src/model/atomic_copy.rs",
            "durable_directory.open",
            2,
        ),
        ("src-tauri/src/plugins.rs", "package.publish", 1),
        ("src-tauri/src/plugins.rs", "package.recover", 1),
        ("src-tauri/src/plugins.rs", "package.retire", 1),
    ]
    .into_iter()
    .map(|(path, boundary, count)| (path.to_owned(), boundary.to_owned(), count))
    .collect::<Vec<_>>();
    assert_eq!(
        actual, expected,
        "a new tine-storage write crossing needs a census row"
    );
    let dependency_surface = tine_storage_surface_inventory(&files);
    let mut dependency_surface = dependency_surface;
    dependency_surface.extend(tine_storage_imported_call_inventory(&files));
    dependency_surface.sort();
    assert!(fs::read_to_string(repository_root().join("crates/tine-core/Cargo.toml"))
        .unwrap()
        .contains("tine-storage = { git = \"https://github.com/martinkoutecky/tine-storage\", tag = \"v0.27.1\""));
    // The digest covers every tine-storage import and direct call in
    // production Rust, so any change to that surface fails here. Re-pin it with
    // one dated line below naming the packet and what moved; this file's git
    // history holds the earlier re-pins.
    //
    // 2026-09-15: the Managed Storage removal (ADR 0066) and the K0 cleanup
    // moved the digest by deletion only.
    // 2026-09-15: K2 made the query walk and Direct `property_owner_rows`
    // test-only; their storage calls left the production surface, by deletion
    // only.
    // 2026-09-15: K4 made the SQL lowering decide on the operator first, so
    // `=` and `<>` (and the day comparisons) share one bind each: query/sql.rs
    // has one fewer `PhysicalQueryValue::Integer(` (12 → 11) and one fewer
    // `PhysicalQueryValue::Text(` (25 → 24) call site, and nothing else moved.
    // 2026-09-15: K3 moved model.rs's storage calls verbatim into the model/
    // seam modules (atomic_copy, projection_rename, trash), which reach
    // model.rs's imports through `use super::*`; the inventory now follows
    // that inheritance. With model/*.rs read as model.rs, the surface hashes
    // to the K4 digest above: paths moved, nothing else.
    // 2026-09-15: K7 split watcher.rs and commands.rs after K0 had removed
    // their tine-storage surface; zero inventory rows moved, so the digest held.
    // 2026-09-17: GH #543 pinned v0.20.1 (writer cache budget, deferred
    // indexes) and then v0.20.2 (navigation readers page on index-served
    // keysets); the pin string above moved twice, the v0.20.1 bump added
    // the `set_page_cache_budget`/`shrink_page_cache_budget` calls in
    // direct_projection.rs, and v0.20.2 changed no call site.
    // 2026-09-17: GH #543 partial admission pinned v0.20.3 and added the
    // relaxed-WAL build calls that P4 later retired with partial serving.
    // 2026-09-17: the bounded-reads packet drove BOTH Friendly block reads from
    // the trigram index instead of filtering a full scan with it, which binds a
    // candidate literal and a candidate cap per read: query/friendly.rs gains
    // `PhysicalQueryValue::Text(` and `PhysicalQueryValue::Integer(` sites.
    // Read-side binds only. The write crossings asserted above are unchanged,
    // no call site was added to a storage writer, and the pin is still v0.20.3.
    // 2026-09-17: GH #543 pinned v0.24.0, which keys the navigation
    // reference-name read to `reference_postings_navigation_names_idx` and
    // narrows its statement to the DISTINCT it wants. The pin string above
    // moved v0.20.3 → v0.24.0. `direct_projection.rs`'s cursor closure changed
    // SHAPE — `navigation_reference_names_after` takes `(normalized_name,
    // raw_name)` where it took a 4-tuple — but that is an argument change at an
    // existing call site, not a new or removed one, so no inventory row moved
    // and the digest below held (the same way the v0.20.2 bump did).
    // 2026-09-18: GH #543 — the torn-header recreate path added storage open,
    // schema initialization and validation calls that P4 later replaced.
    // 2026-09-19: compact-projection P0b routed every projection SQL statement
    // through `query/projection_sql.rs`, which imports
    // `PhysicalProjectionQuerySnapshot`, `PhysicalQueryValue` and
    // `MaterializationError` from tine-storage: one new import row for the
    // door file. The 17 call sites keep their snapshot imports and call the
    // door instead of the storage methods (method calls are not inventory
    // rows), so no other row moved and no write crossing changed.
    // 2026-09-19: compact-projection P1 pinned v0.25.0, which deletes the
    // Managed Storage spine from tine-storage (durable batches, journals v1/v2,
    // sealed digests, oplog frontier, managed layout: 32,640 → 11,863 source
    // lines) and leaves the Direct Files durability and SQLite projection
    // surface. tine-core imported nothing from the deleted half, so the pin
    // string above moved v0.24.0 → v0.25.0 and the digest is unchanged.
    // 2026-09-19: compact-projection P1 pinned v0.26.0, which trims the
    // crate's public surface to the methods Tine's Direct Files path calls
    // (35 public methods on exported types deleted). Nothing Tine imports or
    // calls moved, so the pin string above moved v0.25.0 → v0.26.0 and the
    // digest is unchanged.
    // 2026-09-20: compact-projection P2 removed probe_fts_ready and its one
    // PhysicalQueryValue::Integer pattern in query/results.rs. Restoring that
    // single inventory row reproduces the previous digest exactly; no write
    // crossing or storage dependency pin changed. P3 native folding adds none.
    // 2026-09-20: native linked-reference filtering adds one PhysicalQueryValue
    // import in model/direct_query.rs, twelve Text and one Integer patterns or
    // bindings for scoped read-only queries. No other inventory row changed.
    // 2026-09-20: compact-projection P3a adapted projection DTO imports and
    // calls from physical hash coordinates to public paths/result IDs and
    // integer private coordinates, moved SQL bind sites from blobs to
    // integers, and removed the retired blob decoder. The read/write call
    // ownership and the write crossings asserted above are unchanged.
    // 2026-09-20: P3b correction A classifies malformed streamed candidates
    // directly as ResultReadError::Corrupt instead of constructing three
    // storage InvalidQuery errors. Correction B binds source exclusions and
    // decodes the page/block entity discriminator in the shared unlinked
    // candidate read. These alter read-only value/error inventory rows; no
    // storage write crossing or public storage dependency pin changed.
    // 2026-09-20: one-pass page ranking removes one read-only Integer bind;
    // the byte-length/text callback frame adds two InvalidQuery error sites.
    // No write crossing or dependency pin changes.
    // 2026-09-20: compact-projection P4 replaces partial relaxed-WAL builds
    // with a fresh unpublished OFF image and a storage-owned directory
    // publication. direct_projection.rs gains one DurableDirectoryPublication
    // write crossing plus the fresh/open/validation calls, and removes the
    // retired build-durability calls.
    // 2026-09-20: the P4 final reader-drain correction routes every pooled
    // projection read, including alias ownership, through one mutex-held
    // admission helper. This changes only the counted receiver-call shape;
    // it adds no storage import or authority crossing.
    // 2026-09-20: the P4 streaming-build correction replaces the per-chunk
    // `apply_with_source_revisions_and_aliases` call plus final page-order,
    // optimize and staged-replacement calls with append, finish and finalized-
    // token publication. Schema initialization and one pre-publication schema
    // validation moved inside storage; the ordinary apply call remains. No
    // authority crossing or dependency pin changed.
    // 2026-09-20: final compact-projection delivery pins tine-storage v0.27.0.
    // The production import and direct-call inventory is unchanged, so the
    // digest below holds.
    // 2026-09-21: GH #550 settles pre-seed launch deltas against the reopened
    // image with two read-only `database.source_delta` calls in
    // `settle_unseeded_deltas`. No write crossing or dependency pin changed.
    // 2026-09-21: GH #543 pins tine-storage v0.27.1 (fresh-build speedup:
    // cached statements and coordinate-resolved postings inside storage). The
    // production import and direct-call inventory is unchanged.
    // 2026-09-21: BL1 adds read-only derived SQL snapshots and typed row
    // validation. No storage write crossing, schema or dependency pin changes.
    // 2026-09-22: GH #543 bounded warm repair applies the pages a warm named
    // changed through `apply_warm_repair`: one more
    // `apply_with_source_revisions_aliases_and_page_order` call (the existing
    // order-turn write, now also used for the repair's single transaction) and
    // one read-only `source_delta` call. No new storage import, write kind,
    // schema or dependency pin.
    assert_eq!(
        inventory_digest(&dependency_surface),
        "7226f9c812cc3f908edd05601122fe340bba7ed7877122cd145e236a8c3806cb",
        "the complete tine-storage import/direct-call surface changed: {dependency_surface:#?}"
    );
}

/// Lane evidence never enters the tracked tree at the repository root.
///
/// Wave-2 lanes committed `RECEIPT*.md`, `baseline-*.txt`, `necessity-*.txt`
/// and a fail-before log at the root; every lane wrote the same `RECEIPT.md`
/// name, so the merges destroyed four receipts, and the surviving files leaked
/// private worktree paths and dossier names to both public remotes. Lanes still
/// write their receipt and baselines to the worktree root (a workspace-write
/// lane cannot reach outside it), but those files stay UNTRACKED — `.gitignore`
/// carries the patterns — and the manager archives them under
/// `tine-agents/evidence/` before integration. This guard checks the tracked
/// set, so an in-flight lane's untracked receipt does not trip it.
///
/// The log rule is deliberately `/*.log` and not an enumeration of the suffixes
/// lanes have used so far. The enumeration was tried: it reached nine patterns
/// and still missed `*-pass-after.log`, `*-debug.log`, `*-gate.log` and
/// `*-compile.log`, which is most of what a lane actually writes — 128 such
/// files were sitting untracked at the P3 worktree root, one `git add -A` from
/// repeating the wave-2 leak. No root-level `.log` has ever been tracked, so the
/// wide rule costs nothing and a lane can no longer invent a name that escapes
/// it.
#[test]
fn g_h_repository_root_tracks_no_lane_evidence() {
    let repo = repository_root();
    let ignore = fs::read_to_string(repo.join(".gitignore")).unwrap();
    for pattern in [
        "/RECEIPT*.md",
        "/baseline-*.txt",
        "/necessity-*.txt",
        "/*.log",
    ] {
        assert!(
            ignore.lines().any(|line| line.trim() == pattern),
            ".gitignore lost the lane-evidence pattern {pattern}"
        );
    }
    let Ok(output) = std::process::Command::new("git")
        .arg("-C")
        .arg(&repo)
        .args([
            "ls-files",
            "--",
            "RECEIPT*.md",
            "baseline-*.txt",
            "necessity-*.txt",
            // `:(glob)` keeps `*` from crossing a `/`, so this is the repository
            // root only: a genuine log fixture nested under tests/ is untouched.
            ":(glob)*.log",
        ])
        .output()
    else {
        return; // no git on this machine: the .gitignore half still holds
    };
    let tracked = String::from_utf8_lossy(&output.stdout);
    assert!(
        tracked.trim().is_empty(),
        "lane evidence is tracked at the repository root; move it under \
         tine-agents/evidence/ and `git rm` it:\n{tracked}"
    );
}

#[test]
fn g_e_shipped_native_targets_and_writers_are_pinned() {
    let repo = repository_root();
    let ios_root = repo.join("src-tauri/ios-folder-picker-native/ios/Sources");
    let mut ios = Vec::new();
    visit_source_extensions(&ios_root, &["swift"], &mut ios);
    ios.sort();
    let ios_relative = ios
        .iter()
        .map(|path| path.strip_prefix(&ios_root).unwrap().to_string_lossy())
        .collect::<Vec<_>>();
    assert_eq!(ios_relative, ["GraphFolderPickerPlugin.swift"]);
    let swift = fs::read_to_string(&ios[0]).unwrap();
    let swift_compact = code_mask(&swift)
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let swift_mutations = [
        ("FileManager.default.createDirectory(", 2),
        ("Data().write(", 2),
        (".removeItem(", 0),
        (".moveItem(", 0),
        (".copyItem(", 0),
        (".replaceItemAt(", 0),
    ];
    for (token, expected) in swift_mutations {
        assert_eq!(
            swift_compact.matches(token).count(),
            expected,
            "iOS native mutation surface changed at {token}"
        );
    }
    assert_eq!(
        swift_compact
            .matches("Data().write(to:marker,options:.atomic)")
            .count(),
        2
    );

    let android_root = repo.join("src-tauri/gen/android/app/src/main/java/page/tine/app");
    let mut android = Vec::new();
    visit_source_extensions(&android_root, &["kt", "java"], &mut android);
    android.sort();
    let android_relative = android
        .iter()
        .map(|path| path.strip_prefix(&android_root).unwrap().to_string_lossy())
        .collect::<Vec<_>>();
    assert_eq!(
        android_relative,
        [
            "GraphFolderPickerPlugin.kt",
            "MainActivity.kt",
            "MediaCapturePlugin.kt",
            "SafeBackPlugin.kt",
            "SystemBarsPlugin.kt",
        ]
    );
    let picker = fs::read_to_string(android_root.join("GraphFolderPickerPlugin.kt")).unwrap();
    assert_eq!(
        picker.matches("Intent.ACTION_OPEN_DOCUMENT_TREE").count(),
        1
    );
    for mutation in [
        "FileOutputStream",
        "createTempFile",
        "writeBytes",
        "outputStream(",
    ] {
        assert!(
            !picker.contains(mutation),
            "Android picker became a graph-tree writer: {mutation}"
        );
    }
    let media = fs::read_to_string(android_root.join("MediaCapturePlugin.kt")).unwrap();
    let media_compact = media
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    let native_mutations = [
        ("File.createTempFile(", 2),
        ("FileOutputStream(", 1),
        (".write(", 1),
        (".delete(", 13),
        (".mkdir(", 0),
        (".mkdirs(", 0),
        (".outputStream(", 0),
        ("setOutputFile(", 1),
    ];
    for (token, expected) in native_mutations {
        let actual = android
            .iter()
            .map(|path| {
                code_mask(&fs::read_to_string(path).unwrap())
                    .matches(token)
                    .count()
            })
            .sum::<usize>();
        assert_eq!(
            actual, expected,
            "Android native mutation surface changed at {token}"
        );
    }
    assert_eq!(
        media_compact
            .matches("File.createTempFile(\"tine_photo_\",\".jpg\",activity.cacheDir)")
            .count(),
        1
    );
    assert_eq!(
        media_compact
            .matches("File.createTempFile(\"tine_memo_\",\".m4a\",activity.cacheDir)")
            .count(),
        1
    );
    assert!(media_compact.contains("FileOutputStream(out,false)"));
    assert!(media_compact.contains("copyPickedPhoto(uri,photo)"));
    assert!(media_compact.contains("rec.setOutputFile(out.absolutePath)"));
}

#[test]
fn g_f_graph_path_process_handoffs_are_pinned() {
    let files = production_rust();
    let launch_roots = token_inventory(
        &files,
        &[
            ("process.command.std", "std::process::Command::new("),
            ("process.command.imported", "Command::new("),
            ("process.opener", "opener_command("),
            ("tauri.opener", ".opener("),
            ("tauri.open_url", ".open_url("),
        ],
    );
    let expected_launch_roots = [
        ("src-tauri/src/commands.rs", "process.opener", 3),
        ("src-tauri/src/lib.rs", "process.command.imported", 1),
        ("src-tauri/src/lib.rs", "process.command.std", 1),
        ("src-tauri/src/platform.rs", "process.command.imported", 2),
        ("src-tauri/src/platform.rs", "process.command.std", 1),
        ("src-tauri/src/platform.rs", "process.opener", 9),
        ("src-tauri/src/platform.rs", "tauri.open_url", 2),
        ("src-tauri/src/platform.rs", "tauri.opener", 2),
        ("src-tauri/src/spellcheck.rs", "process.command.imported", 1),
        ("src-tauri/src/spellcheck.rs", "process.command.std", 1),
    ]
    .into_iter()
    .map(|(path, root, count)| (path.to_owned(), root.to_owned(), count))
    .collect::<Vec<_>>();
    assert_eq!(
        launch_roots, expected_launch_roots,
        "a new production process/opener construction site must be classified as graph-derived or non-graph-derived"
    );
    assert_eq!(
        files
            .iter()
            .map(|file| file.compact.matches(".open_path(").count())
            .sum::<usize>(),
        0,
        "a Tauri opener path handoff must be classified before it is added"
    );
    let expected = [
        ("edit_asset_external", 2),
        ("open_asset", 1),
        ("open_page_source", 1),
        ("reveal_page_source", 4),
    ];
    let actual = expected.map(|(name, _)| (name, function_process_handoffs(&files, name)));
    assert_eq!(
        actual, expected,
        "new graph-path process handoffs need a PC-18 census row"
    );
}

#[test]
fn g_g_user_selected_report_writes_stay_on_the_atomic_family() {
    let repo = repository_root();
    let save_dialogs = token_inventory(
        production_rust(),
        &[("dialog.blocking_save_file", ".blocking_save_file(")],
    );
    assert_eq!(
        save_dialogs,
        [
            (
                "src-tauri/src/debug.rs".to_owned(),
                "dialog.blocking_save_file".to_owned(),
                1,
            ),
            (
                "src-tauri/src/graph_verification.rs".to_owned(),
                "dialog.blocking_save_file".to_owned(),
                1,
            ),
        ],
        "a new user-selected destination must be classified and use the atomic family"
    );
    for relative in [
        "src-tauri/src/debug.rs",
        "src-tauri/src/graph_verification.rs",
    ] {
        let source = code_mask(&without_test_items(
            &fs::read_to_string(repo.join(relative)).unwrap(),
        ));
        assert_eq!(
            source.matches("tine_core::model::atomic_write(").count(),
            1,
            "{relative}"
        );
        assert_eq!(source.matches("std::fs::write(").count(), 0, "{relative}");
        assert_eq!(source.matches("fs::write(").count(), 0, "{relative}");
    }
}

#[test]
fn ms14b_retired_patricia_and_detached_bootstrap_routes_are_absent() {
    let files = production_rust();

    // The production Patricia-opening constructor is retired. Tests that need
    // an archive exercise `attach_clean_archive_store` through a cfg(test)
    // helper, so neither spelling may become a production entry point.
    assert_eq!(call_count(&files, "with_archive_store"), 0);
    assert_eq!(call_count(&files, "with_clean_archive_store_for_test"), 0);

    // The detached/inactive-bootstrap entry roots are physically absent from
    // production. Any new caller or definition is an architectural decision,
    // not an incidental resurrection of the retired bootstrap route.
    for root in [
        "prepare_bootstrap_transaction",
        "publish_install_verify_inactive_bootstrap",
        "prepare_inactive_bootstrap_import",
        "prepare_inactive_bootstrap_import_with_progress",
        "reopen_inactive_bootstrap_accepted_authority",
        "retain_inactive_bootstrap_accepted_authority",
    ] {
        assert_eq!(call_count(&files, root), 0, "unexpected caller of {root}");
    }

    for opener in [
        "open_logseq_claim_index",
        "open_portable_path_index",
        "open_page_name_ownership_index",
    ] {
        assert_eq!(call_count(&files, opener), 0, "retired opener {opener}");
    }
    assert!(files
        .iter()
        .all(|file| file.relative != "oplog/content_patricia.rs"));
    assert_eq!(
        files
            .iter()
            .map(|file| identifier_occurrences(&file.code, "PatriciaIndexStore"))
            .sum::<usize>(),
        0
    );
    assert_eq!(call_count(&files, "bootstrap_authoring_capability"), 0);
}

#[test]
fn code_mask_masks_variable_length_character_literals() {
    // `'\u{0009}'..='\u{000d}'` appears verbatim in production sources. The
    // char-literal branch used to assume a one-or-two-byte payload, so NONE of
    // these bytes were masked -- the braces and digits reached `code`, and a
    // brace inside an extracted function body mis-counts depth.
    let source = "const R: RangeInclusive<char> = '\\u{0009}'..='\\u{000d}';\nlet b = '\\x41';\nlet e = '\u{00e9}';\nlet n = '\\n';\n";
    let masked = code_mask(source);

    assert_eq!(masked.len(), source.len(), "the mask must preserve offsets");
    assert!(
        !masked.contains('{'),
        "unmasked char-literal brace: {masked}"
    );
    assert!(
        !masked.contains('}'),
        "unmasked char-literal brace: {masked}"
    );
    assert!(!masked.contains("0009"));
    assert!(!masked.contains("000d"));
    assert!(!masked.contains("x41"));
    assert!(
        !masked.contains('\u{00e9}'),
        "unmasked multi-byte char literal"
    );
    // Code around the literals survives.
    assert!(masked.contains("const R: RangeInclusive<char> ="));
    assert!(masked.contains("..="));
}

#[test]
fn syntax_aware_test_mask_handles_items_fields_locals_and_expressions() {
    let source = r#"
        #[cfg(test)] fn omitted_item() { fs::write("x", b"x"); }
        #[cfg(all(test, unix))] mod omitted_module { fn nested() {} }
        struct Example {
            kept: u8,
            #[cfg(test)] omitted_field: u8,
        }
        fn kept() {
            #[cfg(test)] let omitted_local = fs::write("x", b"x");
            #[cfg(test)] { fs::write("x", b"x"); }
            fs::write("kept", b"kept");
        }
    "#;
    let production = code_mask(&without_test_items(source));
    assert!(!production.contains("omitted_item"));
    assert!(!production.contains("omitted_module"));
    assert!(!production.contains("omitted_field"));
    assert!(!production.contains("omitted_local"));
    assert_eq!(production.matches("fs::write(").count(), 1);
    assert!(production.contains("fn kept()"));
}

#[test]
fn census_guard_itself_names_every_required_guard() {
    let source = include_str!("projection_producer_census.rs");
    let tests = source
        .lines()
        .filter_map(|line| line.trim().strip_prefix("fn g_"))
        .filter_map(|line| line.split_once('(').map(|(name, _)| name))
        .collect::<BTreeSet<_>>();
    assert_eq!(tests.len(), 8);
    let prefixes = tests
        .iter()
        .map(|name| name.split('_').next().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        prefixes,
        BTreeSet::from(["a", "b", "c", "d", "e", "f", "g", "h"])
    );
}

/// I-11 guard: a comment may not point at another file by line number.
///
/// Line numbers in one file are invalidated by an edit in another, silently and
/// without a compiler or test noticing. The 2026-09 sweep found this exact rot:
/// `object_store.rs` and `sqlite.rs` both cited `hot_engine.rs:13120-13127` as
/// the batch-acceptance gate. That range holds unrelated projection-manifest
/// encoding today, and the function the same sentence named,
/// `accept_batch_at_history`, no longer exists anywhere in the crate. Cite the
/// type, function or module by name instead — a name that disappears is at
/// least greppable, and often a compile error.
#[test]
fn production_comments_cite_names_not_line_numbers() {
    /// Immutable published third-party sources are addressable by line because
    /// the exact version is pinned in the same citation.
    const PINNED_EXTERNAL_CITATIONS: &[(&str, &str)] =
        &[("src-tauri/src/ios_folder_picker.rs", "tauri-2.11.2")];

    let mut offenders = Vec::new();
    for file in production_rust() {
        for (number, line) in file.raw.lines().enumerate() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("//") || trimmed.starts_with("/*")) {
                continue;
            }
            if PINNED_EXTERNAL_CITATIONS
                .iter()
                .any(|(path, marker)| file.relative == *path && line.contains(marker))
            {
                continue;
            }
            let bytes = line.as_bytes();
            let cites_a_line = line.match_indices(".rs:").any(|(index, _)| {
                bytes
                    .get(index + 4)
                    .is_some_and(|byte| byte.is_ascii_digit())
                    && bytes[..index]
                        .last()
                        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            });
            if cites_a_line {
                offenders.push(format!("{}:{}: {}", file.relative, number + 1, trimmed));
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these comments cite another file by line number, which rots the moment that \
         file is edited and no gate notices (invariant I-11: code does not lie about \
         itself). Cite the type/function/module by name instead. Offenders:\n{}",
        offenders.join("\n")
    );
}

/// I-11 guard: a comment may not name a test that does not exist.
///
/// A comment saying that some named guard "is the architectural fact that says
/// so" is a promise that the guard is running. The 2026-09 sweep found two comments pointing at
/// `no_production_path_appends_a_turn` and
/// `no_production_path_opens_or_appends_a_projection_turn` — neither had ever
/// existed, and both were asserting that nothing in production opened the
/// projection-turn journal while `sync_runtime.rs` was opening and draining it.
/// A named guard is only worth citing if citing it is checked.
#[test]
fn comments_that_cite_a_test_name_a_test_that_exists() {
    let sources = repository_rust_sources();
    let mut offenders = Vec::new();
    for (relative, source) in sources {
        for (number, line) in source.lines().enumerate() {
            let trimmed = line.trim_start();
            if !(trimmed.starts_with("//") || trimmed.starts_with("*")) {
                continue;
            }
            for (index, _) in line.match_indices("tests::") {
                let cited = line[index + "tests::".len()..]
                    .chars()
                    .take_while(|character| character.is_alphanumeric() || *character == '_')
                    .collect::<String>();
                if cited.is_empty() {
                    continue;
                }
                let defined = sources.iter().any(|(_, other)| {
                    other.contains(&format!("fn {cited}("))
                        || other.contains(&format!("mod {cited} "))
                        || other.contains(&format!("mod {cited};"))
                });
                if !defined {
                    offenders.push(format!("{relative}:{}: tests::{cited}", number + 1));
                }
            }
        }
    }

    assert!(
        offenders.is_empty(),
        "these comments cite a test or test module that does not exist anywhere in the \
         crate, so the architectural fact they promise is not actually being asserted \
         (invariant I-11: code does not lie about itself). Write the guard, or cite the \
         one that really covers the claim. Offenders:\n{}",
        offenders.join("\n")
    );
}
