import { beforeAll, describe, expect, it } from "vitest";
import { initParser, parseBlock } from "./parse";
import { __tineReinstantiate } from "./wasm/lsdoc_wasm.js";

beforeAll(async () => {
  await initParser();
});

describe("lsdoc wasm reinstantiate export", () => {
  it("rebuilds a working parser instance", () => {
    __tineReinstantiate();

    const blocks = parseBlock("# hi", false);
    expect(blocks[0]?.kind).toBe("bullet");
    const first = blocks[0];
    if (first?.kind !== "bullet") throw new Error("expected markdown heading bullet");
    expect(first.size).toBe(1);
    expect(first.inline[0]).toMatchObject({ k: "plain", text: "hi" });
  });
});
