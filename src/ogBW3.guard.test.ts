import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
const source = (path: string) => readFileSync(path, "utf8");
it("I-22: draft loading bounds the read before decoding; exemplar drafts::load_unlocked", () => {
  const body = source("src-tauri/src/drafts.rs").split("fn load_unlocked(")[1].split("fn write_unlocked(")[0];
  expect(body).not.toContain("std::fs::read(path)");
  expect(body).toContain("take((MAX_BYTES + 1) as u64)");
});
it("I-22: CSV drop uses the existing bounded regular-file reader; exemplar device_io::read_regular_file_bounded", () => {
  const body = source("src-tauri/src/commands.rs").split("fn read_text_file_from_path(")[1].split("/// Open an `assets/")[0];
  expect(body).toContain("read_regular_file_bounded");
  expect(body).not.toContain("std::fs::read_to_string");
});
it("I-20: snapshot projects held sources, never live pages; exemplar publish_query::snapshot", () => {
  const body = source("crates/tine-graph-features/src/publish_query.rs").split("fn snapshot(")[1].split("fn app_files(")[0];
  expect(body).not.toContain("store.page(");
});
it("I-21: sheet render latches retire with the binding; exemplar SheetBoard renderedBoardCards", () => {
  expect(source("src/components/SheetTable.tsx")).toContain("clearOnBindingInvalidated(() => renderedSheetRows.clear())");
});
it("I-22: formula consumers bound fan-out, recursive rename and inherited members; exemplar formula/eval.ts", () => {
  const evalSource = source("src/sheet/formula/eval.ts");
  expect(evalSource).toContain("memo: new Map()");
  expect(evalSource).toContain("hasOwnProperty.call(members, name)");
  expect(source("src/sheet/renameField.ts")).toContain("Formula depth exceeds 128 for field rename");
});
