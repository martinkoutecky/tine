import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

const source = (path: string) => readFileSync(
  process.env.OG_SOURCE_DIR ? `${process.env.OG_SOURCE_DIR}/${path}` : path, "utf8",
);

const commands = [
  "edit_asset_external", "empty_asset_trash", "import_asset", "import_native_capture",
  "open_asset", "open_pdf", "save_asset", "save_pdf_area_image", "trash_asset",
  "write_highlights", "write_pdf_view_state",
];

it("all asset writes and OS handoffs require an intent-time binding generation", () => {
  const backend = source("src/backend.ts");
  const rust = source("src-tauri/src/commands.rs");
  const commandBodies = [...rust.matchAll(/#\[tauri::command\]\s*pub\(crate\) (?:async )?fn (\w+)\([\s\S]*?(?=#\[tauri::command\]|$)/g)];
  const assetCommands = commandBodies.filter((match) =>
    /asset|pdf|highlight|^import_native_capture$/.test(match[1])
    || /tine_graph_features::(?:assets|pdf)::|import_asset_from_path|asset_handoff_target/.test(match[0]))
    .map((match) => match[1]).filter((name) =>
      !/^(?:read_|stream_|list_|asset_trash_stats|detect_)/.test(name));
  expect(assetCommands.sort()).toEqual([...commands].sort());
  for (const name of commands) {
    expect(backend, `${name} must use the fail-closed assetCall boundary`).toMatch(
      new RegExp(`this\\.assetCall<[^>]+>\\("${name}"`),
    );
    const body = commandBodies.find((match) => match[1] === name)?.[0];
    expect(body, `${name} must use GraphContext's stale-binding refusal`).toMatch(/state: GraphContext<'_>/);
    expect(body).toMatch(/slot_for_context\(&state\)\?/);
  }
  expect(backend).toMatch(/bindingGeneration <= 0/);
});
