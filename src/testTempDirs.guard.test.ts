// RULE: a Rust test never creates a directory under `target/`. Fixture graphs and
// scratch dirs are self-deleting (`tempfile::TempDir`), bound BEFORE anything that
// uses them so they drop last. A `target/` directory made by hand is deleted only
// when the test reaches its final line: on a panic or an abort it stays, and
// hundreds of them pile up in every worktree (target/ is also what Vite once died
// watching). Exemplar: crates/tine-store/tests/i13_edit_cost.rs `graph()`.
// A line that only READS a pre-built artifact under target/ carries the marker
// comment `// target-read-only: <reason>` on the line above.
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = new URL("..", import.meta.url).pathname;
const RULE =
  "Rust tests must not create directories under target/: use a self-deleting tempfile::TempDir " +
  "(exemplar crates/tine-store/tests/i13_edit_cost.rs graph()); a read-only use of a pre-built " +
  "artifact takes a `// target-read-only: <reason>` comment on the line above.";

function rustFiles(dir: string, out: string[] = []): string[] {
  for (const name of readdirSync(dir)) {
    if (name === "target" || name === "node_modules" || name === ".git" || name === "gen") continue;
    const path = join(dir, name);
    const info = statSync(path);
    if (info.isDirectory()) rustFiles(path, out);
    else if (name.endsWith(".rs")) out.push(path);
  }
  return out;
}

// A path built into the workspace target/: any string climbing out of a crate into
// `../target`, or cargo's own target-dir variables. A bare `.join("target")` inside
// a fixture graph (a folder that happens to be named target) is not a match.
const BUILDS_INTO_TARGET = /"(?:\.\.\/)+target\b|CARGO_TARGET_TMPDIR|CARGO_TARGET_DIR/;

describe("test scratch directories", () => {
  it("no Rust test (crates/*/tests, inline test modules, src-tauri) builds a path under target/", () => {
    const files = [
      ...rustFiles(join(ROOT, "crates")),
      ...rustFiles(join(ROOT, "src-tauri")),
    ];
    const offenders: string[] = [];
    for (const file of files) {
      const lines = readFileSync(file, "utf8").split("\n");
      lines.forEach((line, index) => {
        if (/^\s*\/\//.test(line) || !BUILDS_INTO_TARGET.test(line)) return;
        if (/target-read-only:/.test(lines[index - 1] ?? "")) return;
        offenders.push(`${relative(ROOT, file)}:${index + 1}: ${line.trim()}`);
      });
    }
    expect(offenders, `${RULE}\n${offenders.join("\n")}`).toEqual([]);
  });
});
