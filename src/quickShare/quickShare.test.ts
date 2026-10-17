/** Quick share generation 1, viewer side: the frozen fixtures the Rust writer
 *  made (`tests/fixtures/quick-share/g1/`, `crates/tine-graph-features/tests/quick_share.rs`)
 *  open to their frozen documents, forever; damaged, foreign, newer and
 *  oversized shares are refused readably; navigation keeps the share prefix. */
import { readFileSync, readdirSync } from "node:fs";
import { gzipSync } from "node:zlib";
import { afterEach, describe, expect, it } from "vitest";
import {
  b64uDecode,
  b64uEncode,
  decodeShareDocument,
  gunzipBounded,
  loadQuickShare,
  openEnvelope,
  parseShareFragment,
  QuickShareError,
} from "./quickShare";
import limits from "../../docs/contracts/quick-share-limits.json";
import { openPublishedPermalink, replacePublishedPermalink, setQuickSharePrefix } from "../publishedPermalink";
import type { PublishedSnapshot } from "../publishedBackend";

const FIXTURES = new URL("../../tests/fixtures/quick-share/g1/", import.meta.url);
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

describe("quick share generation 1", () => {
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
      const link = readFileSync(new URL("link.txt", dir), "utf8").trim();
      const bin = readFileSync(new URL("tine-share.bin", dir), "utf8");
      const frozen = JSON.parse(readFileSync(new URL("document.json", dir), "utf8"));
      const hash = new URL(link).hash;
      const parsed = parseShareFragment(hash)!;
      const { urls, fetchImpl } = gistFetch({
        [`https://gist.githubusercontent.com/${parsed.login}/${parsed.gist}/raw/tine-share.bin`]: bin,
      });
      const share = await loadQuickShare(hash, fetchImpl);
      expect(urls).toHaveLength(1);
      expect(share.snapshot).toEqual(frozen.snapshot);
      expect(share.title).toBe(frozen.title);
      expect(share.createdAt).toBe(frozen.created_at);
      expect(urls.join(" ")).not.toContain(b64uEncode(parsed.key));
    }
  });

  it("finds a share whose login changed through the API, and only on GitHub's raw host", async () => {
    const dir = new URL("one-page/", FIXTURES);
    const hash = new URL(readFileSync(new URL("link.txt", dir), "utf8").trim()).hash;
    const bin = readFileSync(new URL("tine-share.bin", dir), "utf8");
    const { gist } = parseShareFragment(hash)!;
    const moved = `https://gist.githubusercontent.com/renamed/${gist}/raw/abc/tine-share.bin`;
    const ok = gistFetch({
      [`https://api.github.com/gists/${gist}`]: JSON.stringify({ files: { "tine-share.bin": { raw_url: moved } } }),
      [moved]: `${bin}\n`,
    });
    expect((await loadQuickShare(hash, ok.fetchImpl)).snapshot.home).toBe("Project");
    expect(ok.urls).toHaveLength(3);
    const evil = gistFetch({
      [`https://api.github.com/gists/${gist}`]: JSON.stringify({ files: { "tine-share.bin": { raw_url: "https://example.com/x" } } }),
    });
    expect(await reason(loadQuickShare(hash, evil.fetchImpl))).toBe("not-a-share");
    expect(evil.urls).toHaveLength(2);
    expect(await reason(loadQuickShare(hash, gistFetch({}).fetchImpl))).toBe("not-found");
    expect(await reason(loadQuickShare(hash, gistFetch({ [`https://gist.githubusercontent.com/tine-fixture/${gist}/raw/tine-share.bin`]: 503 }).fetchImpl))).toBe("unreachable");
    // Review g1 R3: no request follows a redirect off GitHub's hosts.
    const inits: RequestInit[] = [];
    await loadQuickShare(hash, async (url, init) => {
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

  it("admits only generation-1 documents within the caps", () => {
    const frozen = readFileSync(new URL("pages/document.json", FIXTURES));
    expect(decodeShareDocument(frozen).snapshot.schema).toBe(1);
    const doc = JSON.parse(frozen.toString());
    const bytes = (value: unknown) => new TextEncoder().encode(JSON.stringify(value));
    const refusal = (value: unknown) => {
      try {
        decodeShareDocument(bytes(value));
      } catch (error) {
        return (error as QuickShareError).reason;
      }
      return "admitted";
    };
    expect(refusal({ ...doc, quick_share: 2 })).toBe("newer-viewer");
    expect(refusal({ ...doc, title: 7 })).toBe("invalid");
    expect(refusal({ ...doc, snapshot: { ...doc.snapshot, schema: 2 } })).toBe("invalid");
    expect(refusal({ ...doc, snapshot: { ...doc.snapshot, queries: Array(limits.viewer.queries + 1).fill(doc.snapshot.queries[0]) } })).toBe("too-large");
    const huge = structuredClone(doc);
    huge.snapshot.pages[1].blocks[0].raw = "x".repeat(limits.viewer.string_bytes + 1);
    expect(refusal(huge)).toBe("too-large");
    expect(() => decodeShareDocument(new TextEncoder().encode("{"))).toThrow(QuickShareError);
    // Review g1 R1: nested field types are admitted only as generation 1 writes them.
    const broken = (edit: (d: typeof doc) => void) => {
      const copy = structuredClone(doc);
      edit(copy);
      return refusal(copy);
    };
    expect(broken((d) => { d.snapshot.queries = [null]; })).toBe("invalid");
    expect(broken((d) => { d.snapshot.pages[1].blocks[0].raw = 42; })).toBe("invalid");
    expect(broken((d) => { d.snapshot.name = 42; })).toBe("invalid");
    expect(broken((d) => { delete d.snapshot.exported_at; })).toBe("invalid");
    expect(broken((d) => { d.snapshot.aliases = [["a"]]; })).toBe("invalid");
    expect(broken((d) => { d.snapshot.backlinks = { Project: [{ page: 1 }] }; })).toBe("invalid");
    expect(broken((d) => { d.title = "t".repeat(limits.viewer.string_bytes + 1); })).toBe("too-large");
    // Review g1 R2: a home list named like a shared page or alias is refused.
    expect(broken((d) => { d.snapshot.pages[0].name = "project"; })).toBe("invalid");
    expect(broken((d) => { d.snapshot.pages[0].name = "PROJ"; })).toBe("invalid");
    // Review g1 r2 B1: the key is the writer's page_key. A BOM is not Rust whitespace, so the writer accepts
    // "Project\uFEFF" and the viewer must too; one boundary slash per side folds, as in the writer.
    expect(broken((d) => { d.snapshot.pages[0].name = "Project\uFEFF"; })).toBe("admitted");
    expect(broken((d) => { d.snapshot.pages[0].name = "/Project/"; })).toBe("invalid");
  });

  it("parses only well-formed share fragments", () => {
    const key = b64uEncode(KEY);
    expect(parseShareFragment(`#me/0123456789abcdef0123/${key}`)?.rest).toBe("");
    expect(parseShareFragment(`#me/0123456789abcdef0123/${key}/page/A%20b`)?.rest).toBe("/page/A%20b");
    expect(parseShareFragment(`#me/0123456789ABCDEF0123/${key}`)).toBeNull();
    expect(parseShareFragment(`#me/0123456789abcdef0123/${key.slice(1)}`)).toBeNull();
    expect(parseShareFragment(`#bad_login/0123456789abcdef0123/${key}`)).toBeNull();
    expect(parseShareFragment("#/page/Project")).toBeNull();
  });

  it("keeps the share prefix through navigation, refresh and copied links", () => {
    const prefix = `me/0123456789abcdef0123/${b64uEncode(KEY)}`;
    setQuickSharePrefix(prefix);
    const snapshot = JSON.parse(readFileSync(new URL("pages/document.json", FIXTURES), "utf8")).snapshot as PublishedSnapshot;
    const opened: unknown[] = [];
    const router = {
      openPageTarget: (target: unknown) => opened.push(target),
      openPageAtBlock: (target: unknown) => opened.push(target),
      route: () => ({ kind: "page", name: "Notes ü" }),
    } as unknown as Parameters<typeof openPublishedPermalink>[2];
    expect(openPublishedPermalink(snapshot, `#${prefix}`, router).status).toBe("ignored");
    expect(openPublishedPermalink(snapshot, `#${prefix}/page/${encodeURIComponent("Notes ü")}`, router).status).toBe("opened");
    expect(opened).toEqual([{ name: "Notes ü", pageKind: "page", path: "pages/Notes ü.md" }]);
    const history: string[] = [];
    const browser = {
      location: { href: `https://tine.page/s/1/#${prefix}`, hash: `#${prefix}` },
      history: { state: null, replaceState: (_: unknown, __: string, url: string) => history.push(url) },
    } as unknown as Pick<Window, "location" | "history">;
    replacePublishedPermalink({ kind: "page", page: "Notes ü" }, browser);
    replacePublishedPermalink({ kind: "block", block: "6512a000-0000-4000-8000-000000000002" }, browser);
    expect(history).toEqual([
      `https://tine.page/s/1/#${prefix}/page/Notes%20%C3%BC`,
      `https://tine.page/s/1/#${prefix}/block/6512a000-0000-4000-8000-000000000002`,
    ]);
    // No page shown: the address returns to the bare share link, never to "".
    (browser.location as { hash: string }).hash = `#${prefix}/page/X`;
    replacePublishedPermalink(null, browser);
    expect(history[2]).toBe(`https://tine.page/s/1/#${prefix}`);
  });
});
