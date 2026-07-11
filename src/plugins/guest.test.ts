import { describe, expect, it } from "vitest";
import { instantiatePluginGuest, invokePluginGuest } from "./guest";
import { PLUGIN_PROTOCOL_VERSION, type PluginEvent } from "./protocol";

const utf8 = (value: string) => [...new TextEncoder().encode(value)];
const uleb = (value: number): number[] => {
  const bytes: number[] = [];
  do {
    let byte = value & 0x7f;
    value >>>= 7;
    if (value) byte |= 0x80;
    bytes.push(byte);
  } while (value);
  return bytes;
};
const sleb = (value: number): number[] => {
  const bytes: number[] = [];
  let more = true;
  while (more) {
    let byte = value & 0x7f;
    value >>= 7;
    const sign = (byte & 0x40) !== 0;
    more = !((value === 0 && !sign) || (value === -1 && sign));
    if (more) byte |= 0x80;
    bytes.push(byte);
  }
  return bytes;
};
const vector = (...items: number[][]) => [...uleb(items.length), ...items.flat()];
const name = (value: string) => [...uleb(utf8(value).length), ...utf8(value)];
const section = (id: number, payload: number[]) => [id, ...uleb(payload.length), ...payload];
const funcType = (params: number[], results: number[]) => [0x60, ...vector(...params.map((v) => [v])), ...vector(...results.map((v) => [v]))];
const body = (instructions: number[]) => {
  const payload = [0x00, ...instructions, 0x0b];
  return [...uleb(payload.length), ...payload];
};

function responseGuest(response: unknown): ArrayBuffer {
  const output = utf8(JSON.stringify(response));
  const pointer = 8_192;
  const bytes = [
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    ...section(1, vector(funcType([0x7f], [0x7f]), funcType([0x7f, 0x7f], [0x7f]), funcType([], [0x7f]))),
    ...section(2, vector([...name("env"), ...name("memory"), 0x02, 0x01, 0x01, ...uleb(256)])),
    ...section(3, vector([0x00], [0x01], [0x02])),
    ...section(
      7,
      vector(
        [...name("tine_alloc"), 0x00, 0x00],
        [...name("tine_handle"), 0x00, 0x01],
        [...name("tine_result_len"), 0x00, 0x02]
      )
    ),
    ...section(
      10,
      vector(
        body([0x41, 0x00]),
        body([0x41, ...sleb(pointer)]),
        body([0x41, ...sleb(output.length)])
      )
    ),
    ...section(11, vector([0x00, 0x41, ...sleb(pointer), 0x0b, ...uleb(output.length), ...output])),
  ];
  return new Uint8Array(bytes).buffer;
}

describe("plugin WebAssembly guest", () => {
  it("accepts only the inert ABI and parses a bounded response", async () => {
    const response = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      effects: [{ kind: "notice", message: "hello" }],
    };
    const guest = await instantiatePluginGuest(responseGuest(response));
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "activate",
      platform: "desktop",
      capabilities: [],
      settings: {},
    };
    expect(invokePluginGuest(guest, event)).toEqual(response);
  });

  it("rejects a module without exactly the bounded memory import", async () => {
    const emptyModule = new Uint8Array([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]).buffer;
    await expect(instantiatePluginGuest(emptyModule)).rejects.toThrow(/exactly env.memory/);
  });

  it("rejects invalid effects even though the guest is valid WebAssembly", async () => {
    const guest = await instantiatePluginGuest(
      responseGuest({ protocolVersion: PLUGIN_PROTOCOL_VERSION, effects: [{ kind: "run-shell", command: "oops" }] })
    );
    const event: PluginEvent = {
      protocolVersion: PLUGIN_PROTOCOL_VERSION,
      kind: "activate",
      platform: "desktop",
      capabilities: [],
      settings: {},
    };
    expect(() => invokePluginGuest(guest, event)).toThrow(/unsupported kind/);
  });
});
