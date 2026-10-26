import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
const source = (path: string) => readFileSync(path, "utf8");
it("I-12/I-25: publications own name/count invalidation; exemplar store/answer_changes.rs and graphAnswers.ts", () => {
  for (const file of ["src/pageIndex.ts", "src/blockRefCounts.ts"])
    expect(source(file), "text revisions cannot trigger graph-sized answer reads").not.toMatch(/\bdataRev\b/);
  expect(source("src/document/external.ts")).toContain("applyGraphAnswers");
  // Own saves have no direct reply (step 3b P2b: the page host publishes them),
  // so their answers arrive as a page-less bulk event that external.ts applies.
  const watcher = source("src-tauri/src/watcher.rs");
  expect(watcher).toContain("serde_json::to_value(&change)");
  expect(watcher).toMatch(/events\.is_empty\(\) && answers\.is_some\(\)/);
  expect(source("crates/tine-store/src/store/snapshot.rs")).toContain("snapshot.answer_changes(old, &changed_paths, name_set_changed)");
});
it("I-12: one owning-page query substitution, shared with baked queries; exemplar render_query_cache.rs", () => {
  const cache = source("crates/tine-graph-features/src/render_query_cache.rs");
  expect(cache.match(/fn substitute_current_page\(/g)).toHaveLength(1);
  expect(source("crates/tine-graph-features/src/publish_query.rs")).not.toContain("fn substitute_current_page(");
  expect(source("crates/tine-graph-features/src/publish_query.rs")).toContain("use crate::render_query_cache::substitute_current_page");
  expect(cache).toMatch(/current_page:\s*current_page.map\(str::to_owned\)/);
});
it("I-15: the model obtains document/layout together; exemplar doc::parse_with_opts", () => {
  const model = source("crates/tine-store/src/model.rs");
  expect(model).not.toContain("SerializeOpts::detect(");
  expect(model).toContain("doc::parse_with_opts(source)");
  expect(model).toContain("let (mut newdoc, opts) = parse_doc_with_opts(path, content)");
});
