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
        // 4 → 5: `8eb922d8` ("prune reclaims ledger entries whose blob is gone")
        // added the deliberate unlink in `prune` and did not re-pin, leaving this
        // guard red on master. The delta is intended; the pin was stale.
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
            4,
        ),
        ("crates/tine-core/src/oplog/lazy_genesis.rs", "fs.rename", 6),
        (
            "crates/tine-core/src/oplog/lazy_genesis.rs",
            "open.create_new",
            6,
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
            2,
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
        ("crates/tine-core/src/oplog/wire.rs", "cap.remove_file", 9),
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
        ("crates/tine-core/src/publish.rs", "fs.create_dir", 1),
        ("crates/tine-core/src/publish.rs", "fs.remove_dir_all", 1),
        ("crates/tine-core/src/publish.rs", "open.create_new", 1),
        ("crates/tine-core/src/sync_runtime.rs", "cap.remove_file", 7),
        (
            "crates/tine-core/src/sync_runtime.rs",
            "fs.create_dir_all",
            5,
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
        ("src-tauri/src/settings.rs", "fs.create_dir_all", 3),
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
        ("atomic_write", 13),
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
        ("commit", 6),
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
            1,
        ),
        (
            "crates/tine-core/src/oplog/hot_engine.rs",
            "journal.managed_append",
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
        .contains("tine-storage = { git = \"https://github.com/martinkoutecky/tine-storage\", tag = \"v0.12.2\""));
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
    assert_eq!(
        inventory_digest(&dependency_surface),
        "499986ea42ae3ce018f78fe845af36ba9d7aab3c521f89cb11489e2675e201f9",
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
#[test]
fn g_h_repository_root_tracks_no_lane_evidence() {
    let repo = repository_root();
    let ignore = fs::read_to_string(repo.join(".gitignore")).unwrap();
    for pattern in [
        "/RECEIPT*.md",
        "/baseline-*.txt",
        "/necessity-*.txt",
        "/*-fail-before.log",
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
            "*-fail-before.log",
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
