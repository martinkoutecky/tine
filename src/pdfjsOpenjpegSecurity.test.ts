import fs from "node:fs";
import { describe, expect, it } from "vitest";

const vendor = new URL("../vendor/pdfjs-openjpeg/", import.meta.url);
const read = (file: string) => fs.readFileSync(new URL(file, vendor), "utf8");

describe("vendored OpenJPEG security patches (GH #827)", () => {
  const j2k = read("openjpeg/openjp2/j2k.c");
  const readSod = j2k.slice(j2k.lastIndexOf("static OPJ_BOOL opj_j2k_read_sod("));

  it("guards opj_j2k_read_sod's tile-part index writes", () => {
    const writes = readSod.slice(0, readSod.indexOf("opj_j2k_add_tlmarker"));
    const guard = writes.indexOf("l_current_tile_part <");
    expect(guard).toBeGreaterThan(-1);
    expect(writes.indexOf(".nb_tps) {", guard)).toBeGreaterThan(guard);
    expect(writes.indexOf(".end_header", guard)).toBeGreaterThan(guard);
    expect(writes.slice(0, guard)).not.toContain(".end_header");
    expect(writes.slice(0, guard)).not.toContain(".end_pos");
  });

  it("records each patch and its upstream commit in the provenance", () => {
    const { upstream, files } = JSON.parse(read("source.json"));
    expect(upstream.securityPatches).toEqual([expect.objectContaining({
      upstreamCommit: expect.stringContaining("91d08b11a72764f6d199f32fa0c1b1abd4edc2ad"),
      appliesTo: "openjpeg/openjp2/j2k.c",
    })]);
    expect(files[upstream.securityPatches[0].file]).toMatch(/^[0-9a-f]{64}$/);
  });
});
