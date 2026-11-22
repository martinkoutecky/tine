/** Quick share generation 2, viewer side: the frozen fixtures the Rust writer
 *  made (`tests/fixtures/quick-share/g2/`, `crates/tine-graph-features/tests/quick_share.rs`)
 *  open to their frozen documents, forever; damaged, foreign, newer and
 *  oversized shares are refused readably; navigation keeps the share prefix. */
import { readFileSync, readdirSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { afterEach, describe, expect, it, vi } from "vitest";
import blockedGists from "./blocked-gists.json";
import {
  b64uDecode,
  b64uEncode,
  decodeShareDocument,
  gunzipBounded,
  loadQuickShare,
  openEnvelope,
  parseShareLink,
  QuickShareError,
} from "./quickShare";
import limits from "../../docs/contracts/quick-share-limits.json";
import { loadBrowserCore } from "../browserCore";
import { replacePublishedPermalink, setQuickSharePrefix } from "../publishedPermalink";

const FIXTURES = new URL("../../tests/fixtures/quick-share/g2/", import.meta.url);
const locator = (caseName: string) => new URL(readFileSync(new URL(`${caseName}/link.txt`, FIXTURES), "utf8").trim());
const KEY = new TextEncoder().encode("tine-quick-share-fixture-key-32b");
const SHARE_ID = new TextEncoder().encode("fixture-share-id");
const NONCE = new TextEncoder().encode("fixture-nonc");

/** The writer's construction, for refusal cases the frozen files can't show. */
async function seal(payload: Uint8Array<ArrayBuffer>, version = 1): Promise<string> {
  const header = new Uint8Array([0x54, 0x53, 0x48, 0x31, version, ...SHARE_ID]);
  const subtle = globalThis.crypto.subtle;
  const base = await subtle.importKey("raw", KEY, "HKDF", false, ["deriveKey"]);
  const key = await subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: SHARE_ID, info: new TextEncoder().encode("tine-share/1/enc") },
    base,
    { name: "AES-GCM", length: 256 },
    false,
    ["encrypt"],
  );
  const sealed = new Uint8Array(await subtle.encrypt({ name: "AES-GCM", iv: NONCE, additionalData: header, tagLength: 128 }, key, payload));
  return b64uEncode(new Uint8Array([...header, ...NONCE, ...sealed]));
}

const reason = async (promise: Promise<unknown>) => {
  try {
    await promise;
  } catch (error) {
    return error instanceof QuickShareError ? error.reason : `other: ${String(error)}`;
  }
  return "opened";
};

function gistFetch(files: Record<string, string | number>) {
  const urls: string[] = [];
  const fetchImpl = async (url: string) => {
    urls.push(url);
    const body = files[url];
    if (typeof body === "number") return new Response("", { status: body });
    return body === undefined ? new Response("", { status: 404 }) : new Response(body);
  };
  return { urls, fetchImpl };
}

afterEach(() => setQuickSharePrefix(null));

describe("quick share generation 2", () => {
  it("refuses a blocked gist before any request", async () => {
    const { pathname, hash } = locator("pages");
    const link = parseShareLink(pathname, hash)!;
    const blocked = blockedGists as string[];
    const fetchImpl = vi.fn();
    blocked.push(link.gist);
    try {
      expect(await reason(loadQuickShare(pathname, hash, fetchImpl))).toBe("blocked");
      expect(fetchImpl).not.toHaveBeenCalled();
    } finally { blocked.pop(); }
  });

  it("explains missing crypto algorithms before contacting GitHub", async () => {
    const derive = vi.spyOn(crypto.subtle, "deriveKey").mockRejectedValueOnce(new Error("algorithm unavailable"));
    const fetchImpl = vi.fn();
    try {
      expect(await reason(loadQuickShare("/s/2/", "#incomplete", fetchImpl))).toBe("unsupported-browser");
      expect(fetchImpl).not.toHaveBeenCalled();
    } finally { derive.mockRestore(); }
  });
  it("opens the Rust known-answer envelope", async () => {
    const envelope = "VFNIMQFmaXh0dXJlLXNoYXJlLWlkZml4dHVyZS1ub25jxlR5aY52hv6DfbD-0w-Gt7EoF0a_kyGqrXYGhJDTl1KKHQ";
    expect(new TextDecoder().decode(await openEnvelope(envelope, KEY))).toBe("the share document");
    expect(await seal(new TextEncoder().encode("the share document"))).toBe(envelope);
  });

  it("opens every frozen fixture to its frozen document through the raw gist URL", async () => {
    const cases = readdirSync(FIXTURES);
    expect(cases.length).toBeGreaterThanOrEqual(2);
    for (const name of cases) {
      const dir = new URL(`${name}/`, FIXTURES);
      const bin = readFileSync(new URL("tine-share.bin", dir), "utf8");
      const frozen = JSON.parse(readFileSync(new URL("document.json", dir), "utf8"));
      const { pathname, hash } = locator(name);
      const parsed = parseShareLink(pathname, hash)!;
      const { urls, fetchImpl } = gistFetch({
        [`https://gist.githubusercontent.com/${parsed.login}/${parsed.gist}/raw/tine-share.bin`]: bin,
      });
      const share = await loadQuickShare(pathname, hash, fetchImpl);
      expect(urls).toHaveLength(1);
      expect(decodeShareDocument(share.bytes)).toEqual(frozen.snapshot);
      expect(urls.join(" ")).not.toContain(b64uEncode(parsed.key));
    }
  });

  it("finds a share whose login changed through the API, and only on GitHub's raw host", async () => {
    const dir = new URL("reduced-config/", FIXTURES);
    const { pathname, hash } = locator("reduced-config");
    const bin = readFileSync(new URL("tine-share.bin", dir), "utf8");
    const { gist } = parseShareLink(pathname, hash)!;
    const moved = `https://gist.githubusercontent.com/renamed/${gist}/raw/abc/tine-share.bin`;
    const ok = gistFetch({
      [`https://api.github.com/gists/${gist}`]: JSON.stringify({ files: { "tine-share.bin": { raw_url: moved } } }),
      [moved]: `${bin}\n`,
    });
    expect(decodeShareDocument((await loadQuickShare(pathname, hash, ok.fetchImpl)).bytes).name).toBe("Configured");
    expect(ok.urls).toHaveLength(3);
    const evil = gistFetch({
      [`https://api.github.com/gists/${gist}`]: JSON.stringify({ files: { "tine-share.bin": { raw_url: "https://example.com/x" } } }),
    });
    expect(await reason(loadQuickShare(pathname, hash, evil.fetchImpl))).toBe("not-a-share");
    expect(evil.urls).toHaveLength(2);
    expect(await reason(loadQuickShare(pathname, hash, gistFetch({}).fetchImpl))).toBe("not-found");
    expect(await reason(loadQuickShare(pathname, hash, gistFetch({ [`https://gist.githubusercontent.com/tine-fixture/${gist}/raw/tine-share.bin`]: 503 }).fetchImpl))).toBe("unreachable");
    // Review g1 R3: no request follows a redirect off GitHub's hosts.
    const inits: RequestInit[] = [];
    await loadQuickShare(pathname, hash, async (url, init) => {
      inits.push(init ?? {});
      return gistFetch({ [`https://gist.githubusercontent.com/tine-fixture/${gist}/raw/tine-share.bin`]: bin }).fetchImpl(url);
    });
    expect(inits.map((init) => init.redirect)).toEqual(["error"]);
  });

  it("refuses damaged, foreign, newer and wrong-key envelopes readably", async () => {
    const good = await seal(new Uint8Array(gzipSync(Buffer.from("{}"))));
    const file = b64uDecode(good)!;
    const flip = (at: number) => {
      const copy = file.slice();
      copy[at] ^= 1;
      return b64uEncode(copy);
    };
    expect(await reason(openEnvelope(flip(0), KEY))).toBe("not-a-share");
    expect(await reason(openEnvelope(await seal(new Uint8Array([1]), 2), KEY))).toBe("newer-viewer");
    for (const at of [5, 20, 21, 32, 33, file.length - 1]) expect(await reason(openEnvelope(flip(at), KEY))).toBe("unauthentic");
    const other = KEY.slice();
    other[0] ^= 1;
    expect(await reason(openEnvelope(good, other))).toBe("unauthentic");
    expect(await reason(openEnvelope(`${good}=`, KEY))).toBe("not-a-share");
    expect(await reason(openEnvelope(good.slice(0, 60), KEY))).toBe("not-a-share");
    expect(await reason(openEnvelope("A".repeat(limits.viewer.envelope_text_bytes + 1), KEY))).toBe("too-large");
  });

  it("decodes canonical base64url only", () => {
    expect(b64uDecode("AAA")).toEqual(new Uint8Array([0, 0]));
    expect(b64uDecode("AAB")).toBeNull(); // non-zero trailing bits
    expect(b64uDecode("AA==")).toBeNull();
    expect(b64uDecode("A A")).toBeNull();
    expect(b64uDecode("AA+/")).toBeNull();
    expect(b64uDecode("A")).toBeNull();
  });

  it("aborts an encrypted gzip bomb at the expansion cap", async () => {
    const bomb = gzipSync(Buffer.alloc(limits.viewer.document_bytes + 1024 * 1024));
    const payload = await openEnvelope(await seal(new Uint8Array(bomb)), KEY);
    expect(await reason(gunzipBounded(payload, limits.viewer.document_bytes))).toBe("too-large");
  });

  it("recognizes the generation-2 document shape", () => {
    const frozen = readFileSync(new URL("pages/document.json", FIXTURES));
    expect(decodeShareDocument(frozen).schema).toBe(2);
    const doc = JSON.parse(frozen.toString());
    const refusal = (value: unknown) => {
      try {
        decodeShareDocument(new TextEncoder().encode(JSON.stringify(value)));
      } catch (error) {
        return (error as QuickShareError).reason;
      }
      return "admitted";
    };
    expect(refusal({ ...doc, quick_share: 3 })).toBe("newer-viewer");
    expect(refusal({ ...doc, quick_share: 1 })).toBe("invalid");
    expect(refusal({ ...doc, snapshot: { ...doc.snapshot, schema: 1 } })).toBe("invalid");
    const changed = (files: unknown) => ({ ...doc, snapshot: { ...doc.snapshot, files } });
    expect(refusal(changed([null]))).toBe("invalid");
    expect(refusal(changed([{ path: "pages/plain.md", text: 42 }]))).toBe("invalid");
    expect(() => decodeShareDocument(new TextEncoder().encode("{"))).toThrow(QuickShareError);
  });

  it("checks the JSON depth before discarding unknown fields", () => {
    const doc = JSON.parse(readFileSync(new URL("pages/document.json", FIXTURES), "utf8"));
    const nested = (depth: number) => {
      let value: unknown = 'literal [[[{\\\"';
      for (let i = 0; i < depth; i++) value = [value];
      return new TextEncoder().encode(JSON.stringify({ ...doc, extra: value }));
    };
    expect(decodeShareDocument(nested(limits.viewer.depth - 1)).schema).toBe(2);
    expect(() => decodeShareDocument(nested(limits.viewer.depth))).toThrow(/nested too deeply/);
  });

  it("enforces byte limits and counts opened blocks at share open", async () => {
    const { BrowserGraph } = await loadBrowserCore();
    const encoder = new TextEncoder();
    for (const [reason, texts] of [
      ["too many files", Array<string>(limits.viewer.pages + 2).fill("- small")],
      ["1 MiB", ["é".repeat(limits.viewer.string_bytes / 2 + 1)]],
      ["too many blocks", ["- x\n".repeat(limits.viewer.blocks + 1)]],
    ] as const) {
      const files = texts.map((text, i) => ({ path: `pages/Public-${i}.md`, text }));
      const snapshot = decodeShareDocument(encoder.encode(JSON.stringify({
        quick_share: 2, snapshot: { schema: 2, name: "Example", exported_at: "", files },
      })));
      expect(() => new BrowserGraph(
        snapshot.files.map(file => file.path),
        snapshot.files.map(file => encoder.encode(file.text)),
        JSON.stringify({ share: { name: snapshot.name, exported_at: snapshot.exported_at } }),
      )).toThrow(new RegExp(`share-too-large:.*${reason}`));
    }
    const graph = new BrowserGraph(
      ["pages/Queries.md", "pages/Containers.md"],
      [
        "- {{query (task TODO)}}\n".repeat(limits.viewer.queries + 1),
        "- #+BEGIN_QUERY\n  {:query (task TODO)}\n  #+END_QUERY\n".repeat(limits.viewer.queries + 1),
      ].map(text => encoder.encode(text)),
      JSON.stringify({ share: { name: "Example", exported_at: "" } }),
    );
    try {
      expect(JSON.parse(graph.page_inventory_json()).entries.map((entry: { name: string }) => entry.name))
        .toEqual(expect.arrayContaining(["Queries", "Containers"]));
    } finally {
      graph.free();
    }
  });

  it("parses the public locator and key independently", () => {
    const key = b64uEncode(KEY);
    const path = "/s/2/me/0123456789abcdef0123";
    expect(parseShareLink(path, `#${key}`)).toMatchObject({ login: "me", prefix: key });
    expect(parseShareLink(path, `#${key}/page/A%20b`)?.prefix).toBe(key);
    expect(parseShareLink("/s/2/me/0123456789ABCDEF0123", `#${key}`)).toBeNull();
    expect(parseShareLink("/s/2/bad_login/0123456789abcdef0123", `#${key}`)).toBeNull();
    expect(parseShareLink("/s/1/", `#me/0123456789abcdef0123/${key}`)).toBeNull();
    expect(parseShareLink(path, `#${key.slice(1)}`)).toBeNull();
    expect(parseShareLink(path, "#/page/Project")).toBeNull();
  });

  it("keeps the share prefix through navigation, refresh and copied links", () => {
    const prefix = b64uEncode(KEY);
    const url = "https://tine.page/s/2/me/0123456789abcdef0123";
    setQuickSharePrefix(prefix);
    const history: string[] = [];
    const browser = {
      location: { href: `${url}#${prefix}`, hash: `#${prefix}` },
      history: { state: null, replaceState: (_: unknown, __: string, url: string) => history.push(url) },
    } as unknown as Pick<Window, "location" | "history">;
    replacePublishedPermalink({ kind: "page", page: "Notes ü" }, browser);
    replacePublishedPermalink({ kind: "block", block: "6512a000-0000-4000-8000-000000000002" }, browser);
    expect(history).toEqual([
      `${url}#${prefix}/page/Notes%20%C3%BC`,
      `${url}#${prefix}/block/6512a000-0000-4000-8000-000000000002`,
    ]);
    // No page shown: the address returns to the bare share link, never to "".
    (browser.location as { hash: string }).hash = `#${prefix}/page/X`;
    replacePublishedPermalink(null, browser);
    expect(history[2]).toBe(`${url}#${prefix}`);
  });
});
