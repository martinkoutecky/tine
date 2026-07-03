import { beforeAll, describe, expect, it, vi } from "vitest";

const wasmMock = vi.hoisted(() => {
  const init = vi.fn().mockResolvedValue({});
  const parseBlockJson = vi.fn((raw: string) => {
    if (raw === "TRAP") throw new Error("mock wasm trap");
    return JSON.stringify([{ kind: "paragraph", inline: [{ k: "plain", text: raw }] }]);
  });
  const reinstantiate = vi.fn();
  const tag = vi.fn(() => "v0.4.1");
  return { init, parseBlockJson, reinstantiate, tag };
});

vi.mock("./wasm/lsdoc_wasm.js", () => ({
  default: wasmMock.init,
  parse_block_json: wasmMock.parseBlockJson,
  __tineReinstantiate: wasmMock.reinstantiate,
  lsdoc_tag: wasmMock.tag,
}));

import { initParser, isQuarantined, parseBlock } from "./parse";
import { __tineReinstantiate } from "./wasm/lsdoc_wasm.js";

beforeAll(async () => {
  await initParser();
});

describe("parseBlock crash guard", () => {
  it("reinstantiates, retries once, quarantines, and caches persistent traps", () => {
    const reinstantiate = vi.mocked(__tineReinstantiate);

    const bad = parseBlock("TRAP", false);
    expect(bad).toEqual([{ kind: "paragraph", inline: [{ k: "plain", text: "TRAP" }] }]);
    expect(isQuarantined(bad)).toBe(true);
    expect(reinstantiate).toHaveBeenCalledTimes(2);

    const good = parseBlock("good", false);
    expect(good).toEqual([{ kind: "paragraph", inline: [{ k: "plain", text: "good" }] }]);
    expect(isQuarantined(good)).toBe(false);

    const callsAfterGood = reinstantiate.mock.calls.length;
    expect(parseBlock("TRAP", false)).toBe(bad);
    expect(reinstantiate).toHaveBeenCalledTimes(callsAfterGood);
  });
});
