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
    let mut inventory = Vec::new();
    for file in files {
        let imports = tine_storage_surface_inventory(std::slice::from_ref(file))
            .into_iter()
            .filter_map(|(_, token, _)| token.starts_with("usetine_storage").then_some(token))
            .collect::<Vec<_>>();
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
        (
            "crates/tine-core/src/direct_projection.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/direct_projection.rs",
            "fs.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/direct_projection.rs",
            "open.create",
            1,
        ),
        (
            "crates/tine-core/src/fast_commit.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/graph_name_folding.rs",
            "fs.create_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/graph_name_folding.rs",
            "fs.remove_dir_all",
            1,
        ),
        (
            "crates/tine-core/src/graph_name_folding.rs",
            "fs.remove_file",
            2,
        ),
        ("crates/tine-core/src/graph_name_folding.rs", "fs.write", 2),
        (
            "crates/tine-core/src/managed_storage_journey.rs",
            "file.create",
            2,
        ),
        (
            "crates/tine-core/src/managed_storage_journey.rs",
            "fs.create_dir_all",
            5,
        ),
        (
            "crates/tine-core/src/managed_storage_journey.rs",
            "fs.remove_dir_all",
            2,
        ),
        ("crates/tine-core/src/model.rs", "cap.create_dir", 1),
        ("crates/tine-core/src/model.rs", "cap.remove_file", 26),
        ("crates/tine-core/src/model.rs", "cap.rename", 1),
        ("crates/tine-core/src/model.rs", "fs.create_dir", 8),
        ("crates/tine-core/src/model.rs", "fs.create_dir_all", 15),
        ("crates/tine-core/src/model.rs", "fs.remove_dir_all", 2),
        ("crates/tine-core/src/model.rs", "fs.remove_file", 16),
        ("crates/tine-core/src/model.rs", "fs.rename", 3),
        ("crates/tine-core/src/model.rs", "libc.renameat2", 3),
        ("crates/tine-core/src/model.rs", "open.create_new", 16),
        ("crates/tine-core/src/model.rs", "windows.MoveFileW", 1),
        (
            "crates/tine-core/src/model.rs",
            "windows.NtSetInformationFile",
            1,
        ),
        ("crates/tine-core/src/onboarding.rs", "fs.create_dir_all", 4),
        // Current-action roots reclaim only covered cursor marks and obsolete
        // derived roots; original receipt and sweep records remain retained.
        (
            "crates/tine-core/src/oplog/absence_sweep.rs",
            "cap.remove_file",
            2,
        ),
        // Packet 3 v2 checkpoint cleanup removes only digest-named image/map
        // objects absent from both retained generations and all active-reader
        // pins. The graph-derived scan budget bounds each cleanup pass.
        (
            "crates/tine-core/src/oplog/checkpoint_generation.rs",
            "cap.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/oplog/current_action_roots.rs",
            "cap.remove_file",
            3,
        ),
        ("crates/tine-core/src/oplog/import.rs", "fs.create_dir", 1),
        (
            "crates/tine-core/src/oplog/import.rs",
            "fs.create_dir_all",
            1,
        ),
        ("crates/tine-core/src/oplog/import.rs", "fs.remove_file", 4),
        ("crates/tine-core/src/oplog/import.rs", "open.create_new", 1),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "fs.create_dir",
            1,
        ),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "fs.create_dir_all",
            2,
        ),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "fs.remove_dir_all",
            2,
        ),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "fs.remove_file",
            2,
        ),
        // Join marker replacement now crosses the shared durable boundary.
        ("crates/tine-core/src/oplog/lazy_genesis.rs", "fs.rename", 3),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "open.create_new",
            5,
        ),
        (
            "crates/tine-core/src/oplog/local_completion_index.rs",
            "cap.remove_file",
            1,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "cap.create_dir",
            2,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "cap.hard_link",
            1,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "cap.remove_file",
            4,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "cap.rename",
            1,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "open.create_new",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "cap.remove_file",
            4,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "cap.rename",
            2,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "file.set_len",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "fs.create_dir",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "libc.mkdirat",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "libc.openat.create",
            2,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "libc.renameat",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "libc.renameat2",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "libc.unlinkat",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "open.create",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_store.rs",
            "open.create_new",
            1,
        ),
        (
            "crates/tine-core/src/oplog/receiver_absence_summary.rs",
            "cap.remove_file",
            3,
        ),
        // Packet 3 v2 §4 selects a fresh empty recovery-input segment only
        // after checkpoint reinstall, then best-effort unlinks the superseded
        // segment and frontier. The anchor replacement remains the authority.
        (
            "crates/tine-core/src/oplog/recovery_input_journal.rs",
            "cap.remove_file",
            2,
        ),
        ("crates/tine-core/src/oplog/sqlite.rs", "cap.create_dir", 1),
        ("crates/tine-core/src/oplog/sqlite.rs", "fs.create_dir", 1),
        (
            "crates/tine-core/src/oplog/sqlite.rs",
            "fs.create_dir_all",
            2,
        ),
        ("crates/tine-core/src/oplog/sqlite.rs", "fs.remove_file", 1),
        (
            "crates/tine-core/src/oplog/sqlite.rs",
            "libc.openat.create",
            1,
        ),
        ("crates/tine-core/src/oplog/sqlite.rs", "open.create", 1),
        ("crates/tine-core/src/oplog/sqlite.rs", "open.create_new", 1),
        ("crates/tine-core/src/oplog/wire.rs", "cap.create_dir", 1),
        // 10 since ca9bd718 (W5-smalls): `retire_provider_residue_entry` is
        // the single validated front door for retiring one `removed/` entry.
        ("crates/tine-core/src/oplog/wire.rs", "cap.remove_file", 10),
        ("crates/tine-core/src/oplog/wire.rs", "cap.rename", 8),
        ("crates/tine-core/src/oplog/wire.rs", "file.set_len", 1),
        ("crates/tine-core/src/oplog/wire.rs", "fs.create_dir_all", 2),
        ("crates/tine-core/src/oplog/wire.rs", "libc.renameat2", 2),
        ("crates/tine-core/src/oplog/wire.rs", "open.create_new", 3),
        (
            "crates/tine-core/src/oplog/wire.rs",
            "windows.SetFileInformationByHandle",
            1,
        ),
        ("crates/tine-core/src/publish.rs", "cap.create_dir", 2),
        ("crates/tine-core/src/publish.rs", "cap.create_dir_all", 1),
        ("crates/tine-core/src/publish.rs", "cap.rename", 2),
        // Owner-private temporary query storage: Unix creates through a mode
        // 0700 builder; Windows creates with an explicit protected DACL.
        ("crates/tine-core/src/publish.rs", "fs.dir_builder", 1),
        ("crates/tine-core/src/publish.rs", "fs.remove_dir_all", 1),
        ("crates/tine-core/src/publish.rs", "open.create_new", 1),
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
        ("crates/tine-core/src/sync_runtime.rs", "cap.remove_file", 7),
        (
            // +1 (5 -> 6): the clean open/activation path ensures the
            // device-private application runtime root exists before qualifying
            // this endpoint's persistent CRDT writer lanes (P1).
            "crates/tine-core/src/sync_runtime.rs",
            "fs.create_dir_all",
            6,
        ),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "fs.remove_dir_all",
            5,
        ),
        // Packet R1 replaces the rollback rename lattice with two staged
        // generation publications; the marker replacement is the sole commit.
        ("crates/tine-core/src/sync_runtime.rs", "fs.rename", 3),
        ("crates/tine-core/src/sync_runtime.rs", "open.create_new", 1),
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
        ("src-tauri/src/sync_runtime.rs", "fs.create_dir", 1),
        ("src-tauri/src/sync_runtime.rs", "fs.create_dir_all", 3),
        ("src-tauri/src/sync_runtime.rs", "fs.remove_dir_all", 3),
        ("src-tauri/src/sync_runtime.rs", "fs.remove_file", 2),
        ("src-tauri/src/sync_runtime.rs", "fs.rename", 4),
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
        "managed_atomic_create_with_proof",
        "managed_atomic_write_validated",
        "managed_atomic_replace_bound",
        "rename_projection_noreplace_platform",
        "rename_managed_noreplace",
        "atomic_publish",
        "atomic_write",
        "atomic_write_new",
        "atomic_replace_expected_with_hooks",
        "atomic_copy",
        "atomic_copy_new",
        "atomic_copy_file_new",
        "move_file_noreplace",
        "move_to_trash",
        "write_page_projection_with_attempts",
        "preserve_and_restore_projection_recovery",
        "retire_stable_projection_quarantine",
        "reserve_and_rename",
        "create_projection_chain_component",
        "empty_asset_trash",
        "reserve_publish_stage",
        "reserve_publish_recovery",
        "commit_publish_stage",
        "write_publish_stage_file",
        "pending_projection_cleanup_bounded",
        "validate_pending_cleanup_round_root",
        "remove_mutation_authority_if_exact",
        "replace_mutation_authority_if_exact_inner",
        "move_pending_cleanup_marker_noreplace",
        "acquire_mutation_lease",
        "publish_immutable_exact_with_durability",
        "publish_android_private_immutable",
        "publish_pending_cleanup_marker",
        "flip_pending_cleanup_round",
        "stage_object_bytes",
        "stage_manifest_bytes",
        "stage",
        "commit",
        "publish_immutable",
        "install_staged_artifact",
        "replace_head",
        "ensure_shared_provider_directory",
        "put_complete",
        "provider_retire_original_into_placeholder",
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
        "graph_name_folding",
        "probe_graph_name_folding",
    ];
    let actual = roots
        .into_iter()
        .map(|name| (name, call_count(&files, name)))
        .collect::<Vec<_>>();
    let expected = vec![
        ("managed_atomic_create_with_proof", 2),
        ("managed_atomic_write_validated", 2),
        ("managed_atomic_replace_bound", 2),
        ("rename_projection_noreplace_platform", 1),
        ("rename_managed_noreplace", 3),
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
        ("atomic_write_new", 11),
        ("atomic_replace_expected_with_hooks", 1),
        ("atomic_copy", 0),
        ("atomic_copy_new", 1),
        ("atomic_copy_file_new", 1),
        ("move_file_noreplace", 22),
        ("move_to_trash", 3),
        ("write_page_projection_with_attempts", 2),
        ("preserve_and_restore_projection_recovery", 2),
        ("retire_stable_projection_quarantine", 0),
        ("reserve_and_rename", 2),
        ("create_projection_chain_component", 2),
        ("empty_asset_trash", 1),
        ("reserve_publish_stage", 1),
        ("reserve_publish_recovery", 2),
        ("commit_publish_stage", 1),
        ("write_publish_stage_file", 8),
        ("pending_projection_cleanup_bounded", 2),
        ("validate_pending_cleanup_round_root", 2),
        ("remove_mutation_authority_if_exact", 3),
        ("replace_mutation_authority_if_exact_inner", 1),
        ("move_pending_cleanup_marker_noreplace", 1),
        ("acquire_mutation_lease", 4),
        ("publish_immutable_exact_with_durability", 4),
        ("publish_android_private_immutable", 1),
        ("publish_pending_cleanup_marker", 2),
        ("flip_pending_cleanup_round", 1),
        ("stage_object_bytes", 1),
        ("stage_manifest_bytes", 1),
        ("stage", 6),
        // 7 since W5-census: the run-local page-name overlay commits its own
        // in-memory point transition (`local_overlay.page_names.commit`)
        // alongside `ephemeral_page_names.commit`. Name-shared with the
        // durable choke helper; no new durable write path.
        // 8 since rebaselining v2 P1: the lazy-genesis builder again commits
        // each page shard's in-memory Loro transaction (`document.commit`), as
        // the control's page-shard builder did. Also name-shared; no durable
        // write path.
        ("commit", 8),
        ("publish_immutable", 6),
        ("install_staged_artifact", 1),
        ("replace_head", 0),
        ("ensure_shared_provider_directory", 4),
        ("put_complete", 1),
        ("provider_retire_original_into_placeholder", 1),
        ("write_config", 9),
        ("atomic_update", 4),
        ("create_graph", 0),
        ("create_demo_graph", 1),
        ("reserve_restore_recovery", 2),
        ("open_or_create_real_parent", 8),
        ("rename_noreplace_between", 2),
        ("publish_temp_noreplace", 1),
        ("atomic_copy_new_into_live", 4),
        ("move_live_to_recovery", 7),
        ("graph_name_folding", 2),
        ("probe_graph_name_folding", 2),
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
        ("PC-1", "crates/tine-core/src/model.rs", "fnsave_page("),
        (
            "PC-2",
            "crates/tine-core/src/sync_runtime.rs",
            "fnexecute_provider(",
        ),
        (
            "PC-3",
            "crates/tine-core/src/oplog/operational_coordinator.rs",
            "fnexecute_clean_local(",
        ),
        (
            "PC-4",
            "crates/tine-core/src/oplog/operational_coordinator.rs",
            "fnexecute_clean_external(",
        ),
        (
            "PC-5",
            "src-tauri/src/sync_runtime.rs",
            "fnopen_record_with_progress(",
        ),
        (
            "PC-6",
            "src-tauri/src/sync_runtime.rs",
            "fnshutdown_for_direct_files_escape(",
        ),
        (
            "PC-7",
            "src-tauri/src/watcher.rs",
            "fnobserve_legacy_graph_text_event(",
        ),
        ("PC-8", "crates/tine-core/src/model.rs", "fnpublish_html("),
        (
            "PC-9",
            "src-tauri/src/commands.rs",
            "fnapply_journal_filename_migrations(",
        ),
        (
            "PC-10",
            "crates/tine-core/src/sync_runtime.rs",
            "fnprepare_shared_clean(",
        ),
        (
            "PC-11",
            "src-tauri/src/commands.rs",
            "fnset_preferred_workflow(",
        ),
        ("PC-12", "src-tauri/src/graph.rs", "fncreate_graph("),
        ("PC-13", "src-tauri/src/backup.rs", "fnrestore_backup("),
        (
            "PC-14",
            "crates/tine-core/src/graph_name_folding.rs",
            "fnprobe_graph_name_folding(",
        ),
        (
            "PC-15",
            "src-tauri/src/sync_runtime.rs",
            "fnarchive_graph_provider_namespace(",
        ),
        (
            "PC-16",
            "src-tauri/src/android_managed_storage_smoke.rs",
            "fnJava_page_tine_app_ManagedStorageSmoke_runManagedActivationSmoke(",
        ),
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
        restore[0].contains("slot.legacy_graph_cloned("),
        "PC-13 restore must remain gated to a Direct-Files graph"
    );
    let tauri_lib = fs::read_to_string(repo.join("src-tauri/src/lib.rs")).unwrap();
    let tauri_lib_compact = tauri_lib
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect::<String>();
    assert!(tauri_lib_compact.contains(
        "#[cfg(all(target_os=\"android\",debug_assertions))]modandroid_managed_storage_smoke;"
    ));
    let folding_callers = files
        .iter()
        .filter_map(|file| {
            let count = identifier_occurrences(&file.code, "graph_name_folding(")
                - file.code.matches("fn graph_name_folding(").count();
            (count != 0).then_some((file.relative.clone(), count))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        folding_callers,
        [(
            "crates/tine-core/src/managed_storage_journey.rs".to_owned(),
            2
        )],
        "PC-14 must remain confined to the Android managed journey"
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
            ("journal.managed_append", ".append(payload_kind,payload)"),
            ("journal.turn_append", "self.journal.append("),
            ("package.publish", "publish_package_noclobber("),
            ("package.recover", "recover_package_store("),
            ("package.retire", "retire_package("),
        ],
    );
    let expected = [
        (
            "crates/tine-core/src/fast_commit.rs",
            "journal.fast_append",
            1,
        ),
        ("crates/tine-core/src/fast_commit.rs", "journal.v1.open", 1),
        // GH #466: the three Direct Files graph-text sites (create, validated
        // write, bounded replace) left this boundary — its Android arm is a
        // hard link that shared storage refuses — for the graph tree's own
        // no-clobber rename (`move_graph_text_exact_no_replace`). The two
        // remaining opens are the app-private durable authorities.
        ("crates/tine-core/src/model.rs", "durable_directory.open", 2),
        // Packet A5: the disposable clean-open checkpoint publishes its two
        // slots and commit pointer through one durable directory.
        (
            "crates/tine-core/src/oplog/checkpoint_generation.rs",
            "durable_directory.open",
            2,
        ),
        (
            "crates/tine-core/src/oplog/checkpoint_generation.rs",
            "immutable.batch",
            1,
        ),
        // The cold resolver publishes immutable packs and its guarded roots
        // through existing durable-directory handles; no raw writer is added.
        (
            "crates/tine-core/src/oplog/cold_object_store.rs",
            "durable_directory.open",
            2,
        ),
        (
            "crates/tine-core/src/oplog/hot_engine.rs",
            "journal.managed_append",
            1,
        ),
        // The join marker uses the shared exact atomic replacement primitive.
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "durable_directory.open",
            1,
        ),
        (
            "crates/tine-core/src/oplog/local_journal_v2_anchor.rs",
            "journal.fast_append",
            1,
        ),
        (
            "crates/tine-core/src/oplog/local_journal_v2_anchor.rs",
            "journal.managed_append",
            1,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "durable_directory.open",
            1,
        ),
        (
            "crates/tine-core/src/oplog/object_store.rs",
            "immutable.single_writer",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_turn_journal.rs",
            "durable_directory.open",
            2,
        ),
        (
            "crates/tine-core/src/oplog/projection_turn_journal.rs",
            "journal.turn_append",
            1,
        ),
        (
            "crates/tine-core/src/oplog/projection_turn_journal.rs",
            "journal.v2.open",
            2,
        ),
        (
            "crates/tine-core/src/oplog/projection_turn_journal.rs",
            "journal.v2.prepare",
            1,
        ),
        // Packet 3's below-floor custody is a separate sequence domain, but it
        // deliberately crosses the same audited v2 WAL and durable-directory
        // publication boundaries as the existing journals. §4 adds one
        // prepared empty successor, its confirmation open, and the exact
        // anchor replacement that commits successful reinstall.
        (
            "crates/tine-core/src/oplog/recovery_input_journal.rs",
            "durable_directory.open",
            2,
        ),
        (
            "crates/tine-core/src/oplog/recovery_input_journal.rs",
            "journal.v2.open",
            5,
        ),
        (
            "crates/tine-core/src/oplog/recovery_input_journal.rs",
            "journal.v2.prepare",
            2,
        ),
        (
            // New row: the device-private CRDT writer-lane record reaches the
            // audited durable publication family through the same shared
            // primitive as every other authority (D-7), and never through a
            // bespoke temp+rename (P1).
            "crates/tine-core/src/oplog/writer_lane.rs",
            "durable_directory.open",
            1,
        ),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "durable_directory.open",
            4,
        ),
        ("crates/tine-core/src/sync_runtime.rs", "journal.v2.open", 2),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "journal.v2.prepare",
            1,
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
        .contains("tine-storage = { git = \"https://github.com/martinkoutecky/tine-storage\", tag = \"v0.21.0\""));
    // Re-pinned 2026-09-12 (rebaselining v2, P4c): v0.21.0 adds the anchored
    // apply path -- applying a tail batch over covered history that has left
    // SQLite -- which is the capability 4c was blocked on. The bump is not a
    // pure pin change: `FrontierError` gained `CoveredBatchRedelivery`, and the
    // exhaustive `From<FrontierError> for ProjectionError` in `oplog/sqlite.rs`
    // had to gain an arm, so the tree did not compile until it did. That is the
    // failure mode a reference grep cannot see, and the reason this literal pin
    // is worth its maintenance: it makes a dependency bump a deliberate act.
    // The write-crossing table above is unchanged -- no new write boundary.
    // Re-pinned 2026-09-02 (wave-3 packet B4): B4 added read-only
    // `open_read_only`, `property_facet_rows_after`, and `PhysicalEntityId`
    // callers without updating this census, so checkpoint 15abd615 was red here.
    // The write-crossing table above remains unchanged.
    // Re-pinned 2026-09-02 (GH #466): the five Direct Files graph-text
    // `publication.move_exact_no_replace` receiver calls in `model.rs` left the
    // tine-storage boundary for `move_graph_text_exact_no_replace` (see the
    // `durable_directory.open` row for `model.rs`, 5 → 2, and the guard
    // `direct_files_graph_text_publication_uses_the_graph_tree_noreplace_rename`).
    // Re-pinned 2026-09-02 (wave-3 packet S): the certified dependency moved
    // from v0.12.0 to v0.12.2; the audited call surface remains unchanged.
    // Re-pinned 2026-09-03 (wave-4 packet B4b): collapsing the ten hand-written
    // cursor drains in `direct_projection.rs` onto the shared `drain_after`
    // added exactly ONE token to this surface —
    // `direct_projection.rs`'s `tine_storage::sqlite::MaterializationError::Corrupt(`
    // went 1 -> 2, because `block_ref_counts`'s `usize::try_from` conversion
    // must now return the read's typed error where the hand-written loop used
    // `.ok()?`. Derived, not assumed: the direct-call token multiset, the
    // `use tine_storage::…` declarations, and the imported-name call and
    // associated-function surfaces were diffed for every production file this
    // packet touched (`direct_projection.rs`, `oplog/query_lowering.rs`,
    // `model.rs`, `query.rs`) against `d1f98c61`, and that single count is the
    // only difference. No new write crossing: the write-boundary table above is
    // byte-identical, and the change is an error mapping, not a publication.
    // Re-pinned 2026-09-05 (query engine P0-rust wave B): `ParseConfig::digest()`
    // in `config.rs` adds exactly ONE read-only token to this surface,
    // `tine_storage::ContentDigest::of(`. The digest is the parse-config stamp
    // of SPEC §5.8 H6; it reuses the existing content-digest type rather than
    // introducing a second SHA-256 (D-14) and crosses no write boundary — the
    // write-crossing table above is byte-identical.
    // Re-pinned 2026-09-05 (query engine P0-rust wave D, item D3): SPEC §6.2's
    // third registry row source — `DirectProjection::property_owner_rows`, the
    // ready projection's raw stream, which CLOSURE §4 refused to leave deferred
    // — adds exactly FOUR read-only tokens to this surface, all inside that one
    // function and all derived by diffing the token multiset of every changed
    // production file against `f8149fbc`:
    //   `PhysicalGraphProjectionDatabase::open_read_only(`   13 -> 14
    //   `import-associated:PhysicalEntityId::Page(`           2 ->  3
    //   `import-associated:PhysicalEntityId::Block(`          2 ->  3
    //   `tine_storage::sqlite::MaterializationError::Corrupt(` 2 ->  3
    // The first is this file's own reader-open idiom (fourteen sites now), the
    // next two are the owner ids of a property row, and the last is the header
    // validator rejecting an unknown Direct Files text kind. No new write
    // crossing: the write-boundary table above is byte-identical, and every one
    // of the four is a read.
    // Re-pinned 2026-09-05 (query engine P1-a): the two derived tables' single
    // producer, `query/derived.rs`, adds exactly ONE token to this surface —
    // its `use tine_storage::sqlite::PhysicalPropertyAtom;`. Derived, not
    // assumed: the surface was dumped and every entry for each production file
    // this packet touched (`config.rs`, `direct_projection.rs`, `model.rs`,
    // `oplog/import.rs`, `oplog/sqlite.rs`, `oplog/sqlite_materialization.rs`,
    // `query.rs`, `query/eval.rs`, `query/registry.rs`, `sync_runtime.rs`) was
    // read back; only the new file contributes. The atom struct is a row shape
    // the physical layer already owns, so the producer names it rather than
    // introducing a parallel core type (D-14), and it crosses no write
    // boundary: the write-crossing table above is byte-identical.
    // The certified dependency moves v0.12.2 -> v0.13.0 in the same commit: the
    // schema goes 22 -> 23 and five physical types drop their `Eq` derive
    // (`atom_num` is an optional `f64`), which is a minor bump, not a patch.
    // `direct_projection.rs`'s new `tine_storage::ContentDigest` type annotation
    // is deliberately absent from this surface — the inventory records calls
    // (`tine_storage::X(`) and `use` declarations, and a bare type position is
    // neither. Checked, because it looked like a second contributor.
    // Re-pinned 2026-09-05 (query engine P1-a2): the four remaining §5.8
    // projection objects change exactly ONE entry in this surface —
    // `query/derived.rs`'s single `use` declaration widens from
    // `PhysicalPropertyAtom` to `{PhysicalPlanning, PhysicalPropertyAtom,
    // PhysicalTag}`, because the shared producer now also emits the
    // `block_planning` row and the `tags` rows. Derived, not assumed: the
    // import and direct-call inventories were recomputed over the HEAD and
    // working versions of every production file this packet touches
    // (`direct_projection.rs`, `oplog/import.rs`, `oplog/sqlite.rs`,
    // `oplog/sqlite_materialization.rs`, `oplog/mod.rs`, `query/derived.rs`)
    // and that one line is the only difference — zero new calls, zero new
    // `use` sites, and the write-crossing table above is byte-identical. Both
    // added names are row shapes the physical layer already owns, so the
    // producer names them rather than growing parallel core types (D-14).
    // The certified dependency moves v0.13.0 -> v0.14.0 in the same commit: the
    // projection schema goes 23 -> 24 for those four objects, and `PhysicalPage`
    // and `PhysicalBlock` change shape (`tags` becomes `Vec<PhysicalTag>`,
    // `PhysicalBlock` gains `planning`), which under cargo's 0.x rules is a
    // minor bump. v0.14.0 also adds `PhysicalProjectionQueryReader`, the
    // read-only statement seam (D-15) — it is deliberately absent from THIS
    // surface because no tine-core file calls it yet; the lowering that will is
    // P1-b's, and this census is an inventory of actual call sites, not of what
    // the dependency offers.
    // Re-pinned 2026-09-05 (query engine P1-b): the IR -> SQL lowering,
    // `query/sql.rs`, is the ONLY new contributor, and it contributes 35 read
    // entries and no write crossing:
    //   `usetine_storage::sqlite::PhysicalQueryValue;`         1 (its one import)
    //   `import-associated:PhysicalQueryValue::Text(`         21
    //   `import-associated:PhysicalQueryValue::Integer(`       7
    //   `import-associated:PhysicalQueryValue::Real(`          5
    //   `import-associated:PhysicalQueryValue::Blob(`          1
    // Every one is a bound-parameter constructor: SPEC §5.5 requires values to
    // be BOUND, so the compiler's only contact with the physical layer is the
    // value enum the seam's signature takes (I-22 — an interpolated statement is
    // not expressible through it). `PhysicalQueryValue` is not in
    // `write_capable_types`, and the write-crossing table above is byte-identical.
    // Derived, not assumed: the surface was dumped at the working head and every
    // entry was read back; the only other production files this packet touches
    // are `query.rs` (a module declaration and its comment) and `query/eval.rs`
    // (`format_number` widened to `pub(crate)` so the compiler reuses the walk's
    // number formatting rather than growing a twin, D-14), and neither names
    // `tine_storage` at all.
    // The gates that DO call `PhysicalProjectionQueryReader` live in
    // `query/sql_gates_tests.rs`, included by `sql.rs` under `#[cfg(test)]`, so
    // the shared production scanner blanks them and the read-only statement seam
    // is still absent from this surface — as the P1-a2 note above predicted it
    // would be until §5.9's dispatch is wired.
    // Re-pinned 2026-09-06 (query engine P1-c): §5.10's `content match` lowering
    // changes exactly ONE entry in this surface —
    // `query/sql.rs`'s `import-associated:PhysicalQueryValue::Text(` 21 -> 23.
    // The two new sites are both bound-parameter constructors, and they are the
    // two values a content leaf binds: the parsed term's own canonically folded
    // needle for the exact `instr` predicate, and the FTS5 phrase literal for
    // the trigram candidate bound. SPEC §5.5 requires values to be BOUND, so a
    // content predicate's only contact with the physical layer is still the
    // value enum the seam's signature takes (I-22 — an interpolated statement
    // is not expressible through it). Derived, not assumed: the production
    // halves of `query/sql.rs` at `5d99a33a` and at the working head were
    // diffed token by token, and `PhysicalQueryValue::{Blob,Integer,Real}` and
    // the single `use tine_storage::sqlite::PhysicalQueryValue;` are unchanged
    // at 1/7/5/1. The packet's other production edits name `tine_storage` not
    // at all: `query/eval.rs` widens `CompiledLeaves::regex` to `pub(crate)` so
    // the compiler reads the walk's ONE parse instead of growing a twin
    // (D-14/I-12), and `oplog/sqlite_materialization.rs` changes only its
    // `#[cfg(test)]` module, which `without_test_items` blanks. No new write
    // crossing: the write-crossing table above is byte-identical, and the
    // certified dependency stays at v0.14.0.
    // Re-pinned 2026-09-06 (query engine P1-d): §5.9's dispatch is the FIRST
    // tine-core caller of the D-15 read-only statement seam, which the P1-a2
    // note above predicted would appear here exactly now. It contributes five
    // new read entries and changes one existing one; there is no new write
    // crossing, and the write-crossing table above is byte-identical.
    //   `direct_projection.rs`
    //     `usetine_storage::sqlite::{…};`   widened by `PhysicalProjectionQueryReader`
    //                                        and `PhysicalQueryValue` (1 changed entry)
    //     `import-associated:PhysicalProjectionQueryReader::open(`   1 (new)
    //     `import-associated:PhysicalQueryValue::Integer(`           1 (new)
    //   `model.rs`
    //     `usetine_storage::sqlite::PhysicalQueryValue;`             1 (new)
    //     `import-associated:PhysicalQueryValue::Blob(`              1 (new)
    //     `import-associated:PhysicalQueryValue::Text(`              1 (new)
    // `PhysicalProjectionQueryReader` is the read-only handle: it exposes
    // `open`, `run_projection_query` and `explain_query_plan` and cannot reach a
    // writable connection, which is D-15's whole enforcement (the handle, not a
    // statement validator). It is not in `write_capable_types`, and neither is
    // `PhysicalQueryValue`, which is the BOUND-parameter enum §5.5 requires
    // (I-22 — an interpolated statement is not expressible through it). The
    // model-side entries are the two columns §5.3's hydration decodes from the
    // statement's select list, the block id and the page path.
    // Derived, not assumed, and two naming choices in the packet exist to keep
    // this inventory truthful rather than to satisfy it:
    //   * the seam's field is `statement_seam` and its local is `seam`, because
    //     `direct_projection.rs` already holds the WRITE-CAPABLE
    //     `PhysicalGraphProjectionDatabase` under the name `reader` and the
    //     `storage-receiver:` scan attributes by receiver NAME (by substring, so
    //     a `query_reader` field would have filed `run_projection_query` under
    //     the writable handle). With the rename, `reader.as_ref`, `reader.lock`
    //     and `reader.is_none` stay at 12 each, exactly as at `514e8495`.
    //   * `model.rs` imports `PhysicalQueryValue` unaliased, because an `as`
    //     rename mangles the identifier this scan extracts from the `use`
    //     declaration and would have hidden both decode sites from the
    //     inventory.
    // The packet's other production edits name `tine_storage` not at all:
    // `query.rs` splits the shared walk driver and adds the IR-keyed retention
    // rule, `query/sql.rs` changes only §5.3's result-set spelling, and neither
    // adds or removes a `PhysicalQueryValue` construction. The certified
    // dependency stays at v0.14.0 — the seam has been on it since P1-a2 and
    // nothing new is required to call it.
    // DB2/P1-d recovery review: persistent query-table damage must invalidate
    // unchanged source stamps. Three Direct projection entries are added:
    // database.reset (1), reader.take (1), reader.lock (12 -> 13). The reset
    // is the existing transactional disposable-cache API, called only on the
    // leased worker before applying a complete parser snapshot. Cached readers
    // are dropped before the existing reopen helper may replace the file.
    // No authority write boundary or new SQL write API is introduced. The
    // writer_slot Option is not a storage handle; the reset receiver remains
    // named database so this inventory sees the actual storage call.
    // R1: two pure shared helper calls replace local byte estimation and tree
    // traversal. Direct full reconciliation changes one of the two existing
    // database.apply_with_source_revisions_and_aliases calls to the ordered
    // inventory variant; live deltas retain the original call. Both remain
    // within the existing disposable page transaction. No authority crossing
    // was added. The dependency assertion above also catches up to DB2's pin.
    // R2 (regex lowering + nested `refs`): the surface moves by exactly THREE
    // entries, all in `query/sql.rs`, and the delta was DERIVED by dumping this
    // inventory at the base commit and after the packet and diffing the two —
    // not by copying the digest the failure printed:
    //     `usetine_storage::sqlite::PhysicalQueryValue;`
    //         -> `usetine_storage::sqlite::{MaterializationError,
    //             PhysicalQueryValue};`                              (rewritten)
    //     `import-associated:MaterializationError::InvalidQuery(`         1 (new)
    //     `import-associated:PhysicalQueryValue::Integer(`            7 -> 8
    // Nothing else in the whole surface moves: no new file appears, no
    // `storage-receiver:` entry is added or removed, and `direct_projection.rs`
    // is byte-identical here even though the packet edits it — installing the
    // regex predicate goes through the seam handle this census already counts.
    // The two additions are the SAME two facts, seen from the two sides:
    //   * `PhysicalQueryValue::Integer` is how a compiled-regex ID reaches the
    //     statement. It is a BOUND parameter, which is the point (I-22, D-15):
    //     this compiler binds the ID and retains the pattern in its program
    //     table rather than interpolating the pattern into SQL.
    //   * `MaterializationError::InvalidQuery` is how an ID the installed table
    //     does not name FAILS the read instead of quietly matching nothing.
    //     `MaterializationError` is the seam's own error type, already imported
    //     by `direct_projection.rs`; it is not write-capable and adding it does
    //     not widen what this crate can do to a projection.
    // Manager review reconciled these entries against the certified R1 base;
    // the additions neither expose a write operation nor widen the seam.
    // Re-pinned 2026-09-06 (query engine R3, Direct results from the
    // database). Derived, not assumed: the surface was dumped at the certified
    // R2 base `b768eff4` (325 entries) and at the working head (339 entries)
    // and the two multisets diffed. No `storage-receiver:` entry is added or
    // removed. Removed (4): `model.rs` loses its `PhysicalQueryValue` import
    // and its one `::Blob(` and one `::Text(` constructor — the hydration path
    // that bound them is deleted, not moved — and `direct_projection.rs`'s
    // import line is replaced by the one below. Added (18):
    //   `query/results.rs`  1 import line + `PhysicalQueryValue::Blob(` 4,
    //                       `::Integer(` 3, `::Real(` 1, `::Text(` 3 — the
    //                       descriptor read and payload batches bind their
    //                       parameters (I-22); every value is BOUND.
    //   `query/sql.rs`      `MaterializationError::InvalidQuery(` 1 -> 3: the
    //                       descriptor statement refuses an order it cannot
    //                       express instead of matching nothing.
    //   `query_jobs.rs`     1 import, `PhysicalProjectionQueryCancellation` —
    //                       the handle a drain cancels; it cannot write.
    //   `direct_projection.rs`  its `use` line widens by
    //                       `PhysicalProjectionQuerySnapshot`; ONE
    //                       `PhysicalProjectionQuerySnapshot::open_direct(` —
    //                       the job's own read snapshot, pinned at the
    //                       generation the walk answers for; and ONE
    //                       `MaterializationError::Incomplete(` constructor,
    //                       the value the open's validation closure returns
    //                       when the generation moved while the snapshot was
    //                       being acquired (the open fails and the job is
    //                       `NotReady`). All three are read-side.
    // `PhysicalProjectionQuerySnapshot` is not in `write_capable_types`; the
    // write-crossing table above is byte-identical.
    // Re-pinned 2026-09-06 (query engine R4a, the Managed off-actor executor).
    // Derived by diffing the dump at `fac4c938` (339 entries) against the
    // working head (342): added (3), all in `managed_query.rs` and nothing
    // removed — its `use` line (`MaterializationError`,
    // `PhysicalProjectionQuerySnapshot`, `PhysicalQueryValue`), ONE
    // `PhysicalProjectionQuerySnapshot::open_managed(` (the accepted-frontier
    // read snapshot, stamp validated inside its transaction) and ONE
    // `PhysicalQueryValue::Integer(` (the FTS readiness probe's row). No other
    // file gained or lost a `tine_storage` reference; no `storage-receiver:`
    // entry moved. All read-side.
    // S1 Managed-main retirement (2026-09-08): the disposable pending query
    // projection and its registry-patch reader are removed. This deletes their
    // writable database open, disposable-file cleanup, second query snapshot,
    // mask-value decoding, and patch-only SQL binds. Managed live reads now
    // retain one read-only PhysicalProjectionQuerySnapshot::open_managed site;
    // committed registry construction reuses that validated snapshot through
    // query::registry_cache. The authoritative editor/navigation pending
    // journal and its application overlay remain outside this retired surface.
    // Re-pinned 2026-09-07 (query engine R6, warm validation from bytes).
    // Derived by diffing the dump at `2ad7911d` (352) against the working
    // head (369) and subtracting the R5a/R5c deltas above (11): R6 adds 7
    // entries and changes 1, ALL in `direct_projection.rs`, nothing removed
    // elsewhere — `page_inventory` (the warm-session `list_pages` source: one
    // more `open_read_only(` and the `reader.lock/is_none/as_ref` receiver
    // triple every other read in this file uses), `validate_warm` (ONE
    // `database.source_delta` — the read that names replacements from the
    // caller's byte revisions — and ONE `database.apply_with_source_revisions_and_aliases`,
    // applying the warm's DELETIONS through the same writer `apply_pending`
    // already uses; this is the one write-side addition, and it crosses no new
    // boundary), and `MaterializationError::Corrupt(` 3 → 4 (the inventory
    // row decode). No new `tine_storage` symbol is imported.
    // Re-pinned 2026-09-07 (query engine RET1, the public IR commands read the
    // database). Derived by dumping the surface at the working head (370) and
    // diffing it against the R6 pin (369): ONE entry added, NOTHING removed or
    // changed —
    //   `query/sql.rs`  `MaterializationError::InvalidQuery(`  3 -> 4
    // — the `page_statement` wrapper refusing a lowered statement that is not
    // page-anchored, exactly as `descriptor_statement` refuses one that is not
    // block-anchored. Read-side and a refusal, so the write-crossing table
    // above is byte-identical. The packet's other production edits
    // (`query/results.rs`' page read, `model.rs`' §5.9 page/probe dispatch,
    // `managed_query.rs`' request-shaped executor, `sync_runtime.rs`' captured
    // IR route) add no `tine_storage` token at all: they decode through this
    // module's existing row helpers and reuse the snapshots their callers
    // already own.
    // RET2 (2026-09-08): DirectQueryJob::read_registry adds a scoped
    // MaterializationError import and Corrupt decoding/classification sites.
    // It streams narrow registry metadata on the existing read-only snapshot;
    // no writable storage entrypoint or write-crossing table entry changed.
    // This is a new reviewed surface change, not an inherited test failure.
    // RET2 assembly: the former digest did not describe the Direct checkpoint
    // 1350514c. Reconstructing that checkpoint's source with this same scanner
    // yields exactly the current inventory (8821a0de..., no tuple delta).
    // The write-crossing table above is unchanged. This corrects the checkpoint
    // expectation; it does not add a new storage writer to the accepted set.
    // Candidate retirement removes exactly two query_lowering.rs entries:
    // the sqlite import (PhysicalReadError, PhysicalEntityId,
    // SqliteGraphProjectionRead) and PhysicalEntityId::Page, each count one.
    // Restoring those tuples reproduces the accepted 8821a0de digest exactly;
    // all surviving entries and the write-crossing table remain unchanged.
    // The adapters now compile only for the independent test oracle.
    // Re-pinned 2026-09-07 (rebaselining R1b/1). Reviewed the production
    // checkpoint_generation.rs diff: the existing sealed row writer is shared
    // with the inert cutoff builder; empty roots and point membership readers
    // qualify each delta. Imports move with that extraction. No physical write
    // boundary changes: the independently pinned table above still matches.
    // Join marker fix adds exactly one fully qualified shared publication open
    // in lazy_genesis.rs. Its replace_exact method replaces the removed private
    // two-rename protocol; g_a pins the removed raw mutation sites separately.
    // R1b/2 adds the shared CausalTipRecordV2 constructor and one qualified
    // SealedAcceptedIndexWriter::new call for the per-peer map. Both are in
    // checkpoint_generation.rs; no physical write boundary changes.
    // R1b/3 reviewed staging diff: one shared bounded regular point read,
    // one immutable batch constructor and one retained private-directory open.
    // The latter uses the existing single-writer publication method outside
    // Linux. No new raw writes; the physical boundary table above adds those
    // two constructors. Remaining changes are shared store/error type paths.
    // R1b/4: reviewed the document-roster delta. Two more shared bounded reads
    // resolve descriptor and checkpoint blobs; shared map reader/writer imports
    // and the empty root serve the roster. Staging error conversion moves into
    // the shared named-bytes helper. No additional physical write constructor
    // or raw mutation: capsules reuse the same bounded publication handle.
    // R1b/5: the full document-key oracle imports the existing shared
    // authenticated_map_root and point reader, deriving the root from exactly
    // the accepted keys. Descriptor reading moved into one reuse helper.
    // Reviewed source adds no write boundary, raw mutation or alternate codec.
    // Retirable document-map foundation: roster map imports and point reads
    // move from checkpoint_generation into sealed_document_map. Full membership
    // keys compose existing maps; their shared writer handles entity and nested
    // pair upsert/removal. One shared bounded read loads the inner descriptor;
    // canonical full-key qualification reuses authenticated_map_root. Reviewed
    // the complete staged source delta: no new physical publication boundary,
    // raw mutation, alternate tree or second object codec is introduced.
    // Cold history and receiver point rows reuse the shared map reader/writer
    // and audited directory publisher. Compared the complete multiset against
    // the 404-row foundation inventory: 28 additions (18 cold resolver,
    // 10 receiver history), no removals or changes elsewhere. The cold resolver
    // adds the two durable-directory opens registered above; receiver rows use
    // the existing ObjectStore publication boundary.
    // P1 writer incarnation record: reviewed the complete 432 -> 439 row
    // multiset. Seven additions, all in writer_lane.rs: one shared directory
    // open, bounded record read, exact preserve/create/replace publications,
    // nofollow lease revalidation and their import. No removals, new codec or
    // additional raw publication path; the directory open is registered above.
    // Reconciled 2026-09-08 by running this scanner over both exact parents and
    // the merge. The query parent and merge are byte-identical inventories at
    // 457 rows / 494 occurrences; the accepted P4 parent has 444 / 482. The
    // merge-minus-P4 multiset is 24 added and 11 removed rows: query snapshots,
    // registry/export values and typed failures replace retired reader and
    // lowering sites. The P4 structural delta introduces no further tuple over
    // the query parent. All three physical-write inventories are byte-identical
    // at 23 rows / 32 occurrences (digest 0a888e83c0a962b5a86731875d779e27cd0c53cc25b964149965799e7fe35f5e).
    // The retained v0.20.0 dependency adds disposable query progress metadata,
    // no authority format. This census does not qualify either parent.
    // Re-pinned 2026-09-08 (Direct producer coverage). The independently dumped
    // d5498fb-to-working-tree multiset moves from 457 rows / 494 occurrences to
    // 458 / 495: the direct_projection.rs import row is replaced by one widened
    // with PhysicalProjectionQueryProgress and PhysicalProjectionQueryTarget,
    // and PhysicalProjectionQueryProgress::new is added once. Those are the
    // existing process-local coverage owner and its opaque target, not a SQLite
    // writer or authority. The physical-write inventory remains byte-identical
    // at 23 rows / 32 occurrences.
    // Re-pinned 2026-09-09 for current-main query reads. Reconstructed the
    // old 6a02be51 digest exactly: remove Direct progress/target bookkeeping,
    // move the Managed FTS decode into shared results, and add one read-only
    // accepted-apply metadata snapshot in oplog/sqlite. Against that recorded
    // pre-main inventory (458 rows), this retirement removes 22 tuples and
    // adds one narrowed Managed import (437 rows): pending query database
    // open/apply/validation, its registry patch binds, the second snapshot and
    // mask decoding disappear. No authority write boundary is added.
    // Final single-source cleanup removes exactly one additional tuple: the
    // compiler-only PhysicalQueryValue::Blob binding for the deleted page mask.
    // Adding that tuple back reconstructs c2cd2827 byte-for-byte (436 rows then).
    // S3 export adds one import-only tuple for the shared executor's existing
    // read-only PhysicalProjectionQuerySnapshot. Full old/new inventories
    // reconstruct both digests exactly; all write-boundary tuples are unchanged.
    // S3 page results add 14 tuples (437 -> 451): operation-owned rank errors
    // and cancellation import, bound sort/property/limit values, admitted page
    // IDs, and the existing raw page estimator in reader and walk oracle.
    // The full multiset reconstructs both digests; no write boundary changes.
    // S3 static publication and the shared Friendly reader add 21 tuples
    // (451 -> 472), reviewed one by one: 12 `PhysicalQueryValue` decodings and
    // the read-only `PhysicalProjectionQuerySnapshot` import in the new
    // `query/friendly.rs`, one more `PhysicalProjectionQuerySnapshot` import in
    // the new `query/read_execute.rs` (the publication `SnapshotQueryReader`),
    // five `MaterializationError::InvalidQuery` constructions in `query/rank.rs`,
    // and two more value decodings in `direct_projection.rs`. Nothing is
    // removed; no snapshot opener is added (`open_direct`/`open_managed` keep
    // their counts), and the write-boundary token list above is unchanged, so
    // the physical write surface is byte-for-byte the pinned one.
    // Q1's shared Friendly reader REMOVES four tuples (472 -> 468) and adds
    // none. All four belonged to the retired `DirectProjection::fuzzy_candidate_paths`
    // wrapper, which opened its own reader: one
    // `PhysicalGraphProjectionDatabase::open_read_only(` import plus its
    // `reader.lock`, `reader.is_none` and `reader.as_ref` receivers, one
    // occurrence each. The surviving `page_aliases_with_owners` keeps its own
    // copies of those three receivers, which is why they thin rather than
    // disappear. Diffing the full 472-row and 468-row multisets shows these
    // four removals and nothing else, so no write boundary moved: retiring a
    // reader is exactly one fewer place that can open the projection.
    // Q2 Print composes the existing snapshot and subtree primitives rather than
    // opening anything: it adds no import, no direct call and no receiver, so all
    // 468 tuples and this digest stand unchanged, physical writes included.
    // Q4 complete ordering/statistics adds three read-only bound-value tuples
    // (468 -> 471), all in query/sql.rs: one PhysicalQueryValue::Integer and
    // two PhysicalQueryValue::Text occurrences. Shared recency construction
    // and requested raw-property/group keys account for the net additions.
    // Full base/post multisets reconstruct both digests with no removals;
    // snapshot openers and the physical-write token inventory above stand.
    // Q3 scoped display ADDS AND REMOVES NOTHING: the row count stands at 471
    // and exactly three tuples change file, all of them read-only bound values.
    // `query/sql.rs` goes 16 -> 14 `PhysicalQueryValue::Integer(` and 35 -> 34
    // `PhysicalQueryValue::Text(`; `query/friendly.rs` goes 6 -> 8 and 2 -> 3 by
    // the same amounts. That is the shared sort vocabulary moving, not a new
    // producer: the explicit page/block statements and the Friendly sections
    // now bind their sort terms through one `SortBinder`, so the Friendly
    // implementation of it owns the bindings its own arm makes. Diffing the
    // full 471-row base and post multisets shows these three moves and nothing
    // else — no snapshot opener is added, no reader is opened anywhere new, and
    // the write-boundary token inventory above is byte-for-byte the pinned one.
    // Re-pinned 2026-09-10 for BOTH changes that landed on this file today; the
    // merge conflicted here because each side re-pinned this digest from the
    // same 471-row base, so neither side's value survives the combination.
    //
    // (a) rebaselining v2 P1 (page-shard live layout): hot_engine.rs's
    //     sealed-index `use` no longer names `AuthenticatedMapLinkV1` /
    //     `AuthenticatedMapRootV1` in production (only the test-only run-local
    //     root comparison needs them), and identity.rs gains one
    //     `AuthenticatedMapKey::from(` for `DocumentId`'s 16-byte live key.
    //
    // (b) block-level reference narrowing: `direct_projection.rs` gains two
    //     read-only tuples inside `reference_candidates` (the former
    //     `reference_candidate_paths`) —
    //       `import-associated:PhysicalEntityId::Block(`   3 -> 4
    //       `import-associated:PhysicalEntityId::Page(`    3 -> 4
    //     the two arms of one `match` over the `source` a
    //     `page_referrer_candidates_after` row already carries.
    //
    // Neither change adds a write boundary, opens a reader anywhere new, or
    // adds a statement; `PhysicalEntityId` is not in `write_capable_types`, the
    // write-crossing table above is byte-identical, and the certified
    // dependency stays at v0.20.0.
    //
    // The value below is derived, not assumed, and the derivation is the whole
    // point: a merge that combines two independent re-pins of one digest is
    // exactly where a third, unnoticed surface change would hide, because each
    // side's own guard was green in isolation. So the merged multiset was
    // dumped here (474 rows) and (b)'s two documented occurrences were removed
    // from it; the remaining 472 rows hash to 592d0bce…, which is precisely
    // what (a) pinned before the merge. Nothing else in the tine-storage
    // surface moved. Row counts reconcile end to end: 471 base, +1 from (a),
    // +2 from (b), 474 here.
    // Packet 3 v2 adds one sealed-map-node decoder/import and three
    // read_optional_regular calls to enumerate only authenticated image-map
    // objects during bounded cleanup. AuthenticatedMapRootV1::empty remains at
    // five occurrences overall. No new writer family or direct write boundary
    // is introduced; the one cleanup unlink is pinned by g_a above.
    // Packet 3 v2 §3 adds one recovery-input-journal import row, nine calls to
    // the already-audited v2 selector/WAL/publication types, and six receiver
    // calls on its retained segment/publication handles (16 rows total). The
    // three write-crossing families are enumerated above; the four WAL opens
    // distinguish activation, uncertain-append reopen, uncertain-anchor reopen,
    // and first-segment confirmation. No new writer family is introduced.
    // Packet 3 v2 §4 adds one successor prepare, one confirmation open, and
    // one exact anchor replacement within those same audited families. Its two
    // post-reinstall unlinks are separately pinned by g_a. The reconstruction
    // fence adds five read-only receiver calls: one foreground and two
    // projection-turn `selection` reads plus recovery-input `as_ref` and
    // `next_sequence`. They capture authenticated generation identity and
    // durable high-water; no writer family or write boundary is added.
    // Packet 3 v2 §5 adds two read-only `tine_storage::read_optional_regular`
    // calls that reread the inactive payload and generation for complete
    // candidate qualification before `current`. Publication itself still uses
    // the already-enumerated DurableDirectoryPublication family; no new writer
    // family or direct write boundary is introduced.
    // Packet 3 v2 §7 adds one read-only sealed-index object-store adapter and
    // reader traversal so the checkpoint worker can derive native candidate
    // frontiers from accepted evidence through E. The adapter's publication
    // method rejects every call; it adds no writer family or write boundary.
    // Rebaselining v2 P4b. The manager re-derived both multisets rather than
    // blessing the delta: 520 tuples at bd75efce, 534 here, so +14 NET --
    // 18 rows added and 4 removed, the removals being count bumps of the same
    // call families (`SealedAcceptedIndexWriter::new(` 4 -> 5, and so on).
    // (The lane's own comment said "25 additions (509 -> 534)"; the substance
    // was right and the arithmetic was not, which matters because this comment
    // is what the NEXT reviewer compares against.)
    //
    // Every changed row is in checkpoint_generation.rs: the four identity
    // domains extend the existing authenticated maps through the disposable
    // checkpoint publisher, and admission/reopen read them through a
    // read-only object-store adapter. hot_engine.rs is unchanged despite
    // gaining ~1250 lines, no file enters the surface, and the three
    // write-crossing families enumerated above are untouched.
    //
    // One row deserves naming because a census exists to catch exactly this:
    // `SealedAcceptedIndexObjectStore::publish_sealed_accepted_object(` enters
    // the surface for the FIRST time anywhere. It is a publish, so it was
    // checked rather than waved through. Its two production call sites are the
    // counting wrapper and the generation publisher writing a content-addressed
    // StatusRecord into the sealed generation directory before the marker moves
    // -- disposable checkpoint state inside a write boundary that already
    // existed, not accepted history, raw files, or the physical projection
    // database. New call-site family, no new write boundary.
    assert_eq!(
        inventory_digest(&dependency_surface),
        "c67041dfc50dc5bf2c3822ac5a7169c681ee68e6bb0d81e46409f8fbe4b0aaa1",
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

/// I-21 / I-20: an off-actor query job holds an owned snapshot of the Managed
/// projection file, so every site that closes or replaces that file drains the
/// runtime's query-job owner first. The file closes in exactly three places —
/// `RuntimeActor` dropping (its `SqliteFrontier` truncates the WAL on drop),
/// `HandleInner` dropping (which must refuse new jobs BEFORE it stops the
/// actor), and the shared-join install replacing the clean runtime — and this
/// guard pins each one to its drain. The accepted-batch apply is deliberately
/// not a site: it writes the checkpoint sidecar, never a WAL checkpoint.
/// Exemplar to imitate when adding a fourth: the shared-join install in
/// `sync_runtime.rs` (`cancel_all_and_drain()` on the line before
/// `self.clean.take()`).
#[test]
fn g_i_managed_query_jobs_drain_before_projection_file_close() {
    let files = production_rust();
    let source = |relative: &str| {
        &files
            .iter()
            .find(|file| file.relative == relative)
            .unwrap_or_else(|| panic!("{relative} is a production file"))
            .code
    };
    let runtime = source("crates/tine-core/src/sync_runtime.rs");
    let block = |header: &str| {
        let start = runtime
            .find(header)
            .unwrap_or_else(|| panic!("{header} exists in sync_runtime.rs"));
        let tail = &runtime[start..];
        &tail[..tail.find("\n}\n").expect("impl block closes")]
    };
    let actor_drop = block("impl Drop for RuntimeActor {");
    let actor_drain = actor_drop
        .find("managed_query.jobs.cancel_all_and_drain()")
        .expect(
            "I-21: RuntimeActor::drop must drain every off-actor query job before its \
             SqliteFrontier closes the projection file (see the guard's doc comment)",
        );
    assert!(
        actor_drain < actor_drop.len(),
        "I-21: RuntimeActor::drop drains before its fields close the main projection"
    );
    let handle_drop = block("impl Drop for HandleInner {");
    let close = handle_drop
        .find("managed_query.jobs.close()")
        .expect("I-21: HandleInner::drop must close the query-job owner");
    let stop = handle_drop
        .find("sender.get_mut().unwrap().take()")
        .expect("HandleInner::drop stops the actor by dropping its sender");
    assert!(
        close < stop,
        "I-21: the handle must refuse and drain query jobs BEFORE it stops the actor, \
         because the actor's exit closes the projection file"
    );
    let takes = runtime
        .match_indices("self.clean.take()")
        .collect::<Vec<_>>();
    assert_eq!(
        takes.len(),
        1,
        "a new site replaces the clean runtime; drain `managed_query.jobs` on the line \
         before it and extend this guard (I-21)"
    );
    for (at, _) in takes {
        let preceding = &runtime[at.saturating_sub(400)..at];
        let drain = preceding
            .find("managed_query.jobs.cancel_all_and_drain()")
            .expect(
                "I-21: `self.clean.take()` closes the projection file; drain the query-job \
                 owner immediately before it (exemplar: the shared-join install)",
            );
        assert!(
            drain < preceding.len(),
            "I-21: drain the query jobs before replacing the main projection"
        );
    }
    assert_eq!(
        runtime.matches("self.clean = Some(").count(),
        1,
        "the clean runtime is reinstalled in exactly one place (the shared-join install), \
         after the drained take above; a second installer needs its own drain (I-21)"
    );
    let sqlite = source("crates/tine-core/src/oplog/sqlite.rs");
    assert!(
        sqlite.contains("impl Drop for SqliteFrontier"),
        "the reason the drains exist: SqliteFrontier checkpoints the WAL on drop"
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
    assert_eq!(tests.len(), 9);
    let prefixes = tests
        .iter()
        .map(|name| name.split('_').next().unwrap())
        .collect::<BTreeSet<_>>();
    assert_eq!(
        prefixes,
        BTreeSet::from(["a", "b", "c", "d", "e", "f", "g", "h", "i"])
    );
    assert!(
        include_str!("oplog/mod.rs")
            .contains("fn oplog_external_module_surface_is_exactly_the_named_consumers()"),
        "G-14b-a public oplog surface guard must remain present"
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
