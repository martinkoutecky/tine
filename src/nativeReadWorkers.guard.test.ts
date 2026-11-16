import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

function rustFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap(entry => {
    const file = `${dir}/${entry.name}`;
    return entry.isDirectory() ? rustFiles(file) : file.endsWith(".rs") ? [file] : [];
  });
}
const handlers = rustFiles("src-tauri/src").flatMap(file => [...readFileSync(file, "utf8").matchAll(/^[ \t]*#\[tauri::command[^\n]*\]\s*(?:#\[[^\n]*\]\s*)*(?:pub(?:\([^)]+\))? )?(async )?fn (\w+)\([\s\S]*?\n\}/gm)]);
describe("L7s4-03 native read dispatch", () => {
  it.each(["read_asset", "read_local_image", "read_text_file", "read_journal_file", "sync_conflict_diff"])("dispatches %s through the blocking pool", name => {
    const handler = handlers.find(h => h[2] === name);
    expect(handler?.[1]).toBe("async ");
    expect(handler?.[0]).toMatch(/off_ui\(|spawn_blocking\(/);
  });
  it("requires review for every new synchronous command", () => {
    expect(handlers.filter(h => !h[1]).map(h => h[2]).sort()).toMatchSnapshot();
  });
});
