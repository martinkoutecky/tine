import { describe, expect, it } from "vitest";
import {
  compareGraphVerificationManifests,
  parseGraphVerificationManifest,
  type GraphVerificationManifest,
} from "./graphVerification";

const digest = (character: string) => character.repeat(64);
const manifest = (files: GraphVerificationManifest["files"], complete = true): GraphVerificationManifest => ({
  schemaVersion: 1,
  tool: "tine-graph-bytes",
  algorithm: "sha256",
  complete,
  generatedAtUnixMs: 1,
  files,
  aggregateDigest: complete ? digest("a") : undefined,
  errors: complete ? [] : [{ detail: "changed during verification" }],
});

describe("graph byte verification", () => {
  it("matches identical nested and Unicode source bytes", () => {
    const source = manifest([
      { path: "notes/deep/café.md", length: 4, digest: digest("b") },
      { path: "journals/你好.org", length: 8, digest: digest("c") },
    ]);
    expect(compareGraphVerificationManifests(source, source)).toEqual({
      matches: true,
      incomplete: false,
      localOnly: [],
      otherOnly: [],
      changed: [],
    });
  });

  it("finds same-size byte changes, additions, removals, and renames", () => {
    const local = manifest([
      { path: "pages/changed.md", length: 10, digest: digest("b") },
      { path: "pages/local.md", length: 2, digest: digest("c") },
    ]);
    const other = manifest([
      { path: "pages/changed.md", length: 10, digest: digest("d") },
      { path: "pages/renamed.md", length: 2, digest: digest("c") },
    ]);
    expect(compareGraphVerificationManifests(local, other)).toEqual({
      matches: false,
      incomplete: false,
      localOnly: ["pages/local.md"],
      otherOnly: ["pages/renamed.md"],
      changed: ["pages/changed.md"],
    });
  });

  it("never reports a match for an incomplete scan", () => {
    expect(compareGraphVerificationManifests(manifest([], false), manifest([]))).toMatchObject({
      matches: false,
      incomplete: true,
    });
  });

  it("rejects traversal, absolute, backslash, and duplicate paths", () => {
    for (const path of ["../secret.md", "/pages/a.md", "pages\\a.md", "pages//a.md"]) {
      expect(() => parseGraphVerificationManifest(JSON.stringify(manifest([
        { path, length: 1, digest: digest("a") },
      ])))).toThrow(/unsafe file path/);
    }
    expect(() => parseGraphVerificationManifest(JSON.stringify(manifest([
      { path: "pages/a.md", length: 1, digest: digest("a") },
      { path: "pages/a.md", length: 1, digest: digest("a") },
    ])))).toThrow(/duplicate file paths/);
  });
});
