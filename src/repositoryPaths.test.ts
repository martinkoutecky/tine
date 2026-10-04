import { execFileSync } from "node:child_process";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { expect, it } from "vitest";

const root = fileURLToPath(new URL("../", import.meta.url));
// Build these literals so the guard can scan itself without an exception.
const forbidden = ["/aux/", "/home/"].map((prefix) => prefix + "koutecky");

it("GH #579: tracked files use portable paths", () => {
  const files = execFileSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" })
    .split("\0").filter(Boolean);
  const violations = files.flatMap((file) => {
    const text = readFileSync(path.join(root, file), "utf8");
    return text.split("\n").flatMap((line, index) =>
      forbidden.some((prefix) => line.includes(prefix)) ? [`${file}:${index + 1}`] : []);
  });
  expect(violations, "Use repository-relative paths, environment variables with documented defaults, "
    + "or neutral wording for historical machine-path evidence (GH #579).").toEqual([]);
}, 30_000);

it("touched scripts expose help from a different working directory", () => {
  for (const script of ["e2e-journal-rollover.mjs", "e2e-multigraph.mjs",
    "e2e-plugin-graph-ownership.mjs", "gh619-repro.mjs"]) {
    const output = execFileSync(process.execPath, [path.join(root, "scripts", script), "--help"], {
      cwd: path.join(root, "src"), encoding: "utf8", timeout: 10_000,
    });
    expect(output).toContain("Usage:");
    expect(output).toContain("TAURI_DRIVER:");
  }
}, 30_000);

it("driver help honors overrides and portable fallback defaults", () => {
  for (const script of ["e2e-multigraph.mjs", "e2e-plugin-graph-ownership.mjs", "gh619-repro.mjs"]) {
    const help = (driver: string, cargo: string) => execFileSync(process.execPath,
      [path.join(root, "scripts", script), "--help"], {
        cwd: root, encoding: "utf8", timeout: 10_000,
        env: { ...process.env, TAURI_DRIVER: driver, CARGO_HOME: cargo },
      });
    expect(help("custom-driver", "custom-cargo")).toContain("TAURI_DRIVER: custom-driver");
    expect(help("", "custom-cargo")).toContain(`TAURI_DRIVER: ${path.join("custom-cargo", "bin", "tauri-driver")}`);
    expect(help("", "")).toContain("TAURI_DRIVER: tauri-driver");
  }
}, 60_000);
