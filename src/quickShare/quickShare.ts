/** Quick share generation 1, viewer side (`docs/contracts/quick-share.md`).
 *
 *  A share link is `https://tine.page/s/1/#<login>/<gist_id>/<key>[/<rest>]`.
 *  This module parses the fragment, fetches the gist's `tine-share.bin`,
 *  decrypts it with WebCrypto (HKDF-SHA256 → AES-256-GCM, header as AAD),
 *  gunzips it under a running cap and admits the share document, whose
 *  `snapshot` is the published snapshot the reader already renders.
 *  Every limit comes from `quick-share-limits.json`, which the Rust writer
 *  also checks. The fragment never leaves the browser: it is not put into a
 *  request, a log or an error message. */
import limits from "../../docs/contracts/quick-share-limits.json";
import type { PublishedSnapshot } from "../publishedBackend";
import { readBounded, validateSnapshot } from "../publishedBackend";

export const QUICK_SHARE_GENERATION = 1;
const MAGIC = [0x54, 0x53, 0x48, 0x31]; // "TSH1"
const ENVELOPE_VERSION = 1;
const HEADER_LEN = 21;
const NONCE_LEN = 12;
const TAG_LEN = 16;
const ENC_INFO = new TextEncoder().encode("tine-share/1/enc");
const LOGIN = /^[A-Za-z0-9-]{1,39}$/;
const GIST = /^[0-9a-f]{20,32}$/;
const RAW_HOST = "gist.githubusercontent.com";

export type QuickShareReason =
  | "bad-link"
  | "unsupported-browser"
  | "not-found"
  | "unreachable"
  | "too-large"
  | "not-a-share"
  | "newer-viewer"
  | "unauthentic"
  | "invalid";

/** A refusal the viewer shows; `reason` picks the sentence. */
export class QuickShareError extends Error {
  constructor(readonly reason: QuickShareReason, message: string) {
    super(message);
    this.name = "QuickShareError";
  }
}

const fail = (reason: QuickShareReason, message: string): never => {
  throw new QuickShareError(reason, message);
};

const B64U = /^[A-Za-z0-9_-]*$/;

/** RFC 4648 §5 base64url without padding. */
export function b64uEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** Canonical base64url only: padding, whitespace, foreign characters and
 *  non-zero trailing bits are refused (null). */
export function b64uDecode(text: string): Uint8Array<ArrayBuffer> | null {
  if (!B64U.test(text) || text.length % 4 === 1) return null;
  let binary: string;
  try {
    binary = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
  } catch {
    return null;
  }
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return b64uEncode(bytes) === text ? bytes : null;
}

export interface ShareLink {
  login: string;
  gist: string;
  key: Uint8Array<ArrayBuffer>;
  /** `<login>/<gist>/<key>` exactly as given; navigation never changes it. */
  prefix: string;
  /** Everything after the prefix: "", "/page/<name>" or "/block/<uuid>". */
  rest: string;
}

/** Parse a share fragment (with or without the leading `#`). */
export function parseShareFragment(hash: string): ShareLink | null {
  const body = hash.startsWith("#") ? hash.slice(1) : hash;
  const [login = "", gist = "", keyText = "", ...rest] = body.split("/");
  if (!LOGIN.test(login) || !GIST.test(gist)) return null;
  const key = b64uDecode(keyText);
  if (!key || key.length !== 32) return null;
  return { login, gist, key, prefix: `${login}/${gist}/${keyText}`, rest: rest.length ? `/${rest.join("/")}` : "" };
}

/** Whether this browser has what a share needs. */
export function quickShareSupported(): boolean {
  return typeof globalThis.crypto?.subtle?.deriveKey === "function" && typeof globalThis.DecompressionStream === "function";
}

/** Decrypt envelope text to the compressed share document `P`, checking in
 *  the contract's order: size, canonical base64url, length, magic, version,
 *  then authentication of everything. */
export async function openEnvelope(text: string, key: Uint8Array<ArrayBuffer>): Promise<Uint8Array<ArrayBuffer>> {
  if (text.length > limits.viewer.envelope_text_bytes) fail("too-large", "This share is larger than a viewer reads.");
  const file = b64uDecode(text) ?? fail("not-a-share", "This is not a Tine share.");
  if (file.length > limits.viewer.envelope_decoded_bytes) fail("too-large", "This share is larger than a viewer reads.");
  if (file.length < HEADER_LEN + NONCE_LEN + TAG_LEN) fail("not-a-share", "This is not a Tine share.");
  if (MAGIC.some((byte, i) => file[i] !== byte)) fail("not-a-share", "This is not a Tine share.");
  if (file[4] !== ENVELOPE_VERSION) fail("newer-viewer", "This share needs a newer viewer.");
  const header = file.slice(0, HEADER_LEN);
  const shareId = file.slice(5, HEADER_LEN);
  const nonce = file.slice(HEADER_LEN, HEADER_LEN + NONCE_LEN);
  const sealed = file.slice(HEADER_LEN + NONCE_LEN);
  const subtle = globalThis.crypto.subtle;
  const base = await subtle.importKey("raw", key, "HKDF", false, ["deriveKey"]);
  const cipherKey = await subtle.deriveKey(
    { name: "HKDF", hash: "SHA-256", salt: shareId, info: ENC_INFO },
    base,
    { name: "AES-GCM", length: 256 },
    false,
    ["decrypt"],
  );
  try {
    const plain = await subtle.decrypt({ name: "AES-GCM", iv: nonce, additionalData: header, tagLength: 128 }, cipherKey, sealed);
    return new Uint8Array(plain);
  } catch {
    return fail("unauthentic", "This share could not be opened: the link's key does not match, or the file was changed.");
  }
}

/** Gunzip with a running count, aborting as soon as the output passes `max`
 *  (a valid encrypted gzip bomb is authorable by anyone who can share). */
export async function gunzipBounded(bytes: Uint8Array<ArrayBuffer>, max: number): Promise<Uint8Array> {
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream("gzip"));
  try {
    return await readBounded(new Response(stream), max, "share document");
  } catch (error) {
    if (error instanceof Error && /larger than/.test(error.message)) {
      return fail("too-large", "This share is larger than a viewer reads.");
    }
    return fail("invalid", "This share is damaged.");
  }
}

export interface ShareDocument {
  title: string;
  createdAt: string;
  snapshot: PublishedSnapshot;
}

function admitCaps(snapshot: PublishedSnapshot): void {
  const v = limits.viewer;
  if (snapshot.pages.length > v.pages || snapshot.queries.length > v.queries) fail("too-large", "This share is larger than a viewer reads.");
  let blocks = 0;
  const pending: unknown[] = [snapshot];
  while (pending.length) {
    const node = pending.pop();
    if (typeof node === "string") {
      // UTF-16 length bounds UTF-8 bytes from below; 3× bounds it from above.
      if (node.length * 3 > v.string_bytes && new TextEncoder().encode(node).length > v.string_bytes) {
        fail("too-large", "This share holds a block larger than a viewer reads.");
      }
    } else if (node && typeof node === "object") {
      for (const child of Object.values(node)) pending.push(child);
    }
  }
  const count = (list: unknown): void => {
    if (!Array.isArray(list)) return;
    for (const block of list) {
      blocks += 1;
      count((block as { children?: unknown }).children);
    }
  };
  for (const page of snapshot.pages) count(page.blocks);
  if (blocks > v.blocks) fail("too-large", "This share holds more blocks than a viewer reads.");
}

/** Admit a gunzipped share document. */
export function decodeShareDocument(bytes: Uint8Array): ShareDocument {
  let value: unknown;
  try {
    value = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    return fail("invalid", "This share is damaged.");
  }
  const doc = value as { quick_share?: unknown; created_at?: unknown; title?: unknown; snapshot?: unknown };
  if (!doc || typeof doc !== "object") fail("invalid", "This share is damaged.");
  if (typeof doc.quick_share === "number" && doc.quick_share > QUICK_SHARE_GENERATION) fail("newer-viewer", "This share needs a newer viewer.");
  if (doc.quick_share !== QUICK_SHARE_GENERATION || typeof doc.created_at !== "string" || typeof doc.title !== "string") {
    fail("invalid", "This share is damaged.");
  }
  const snapshot = doc.snapshot as PublishedSnapshot;
  try {
    validateSnapshot(snapshot);
  } catch {
    fail("invalid", "This share is damaged.");
  }
  for (const key of ["pages", "entries", "aliases", "queries"] as const) {
    if (!Array.isArray(snapshot[key])) fail("invalid", "This share is damaged.");
  }
  admitCaps(snapshot);
  return { title: doc.title as string, createdAt: doc.created_at as string, snapshot };
}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;
const REQUEST: RequestInit = { credentials: "omit", referrerPolicy: "no-referrer", redirect: "follow" };

async function envelopeText(link: ShareLink, fetchImpl: Fetch): Promise<string> {
  const read = async (response: Response) => {
    const bytes = await readBounded(response, limits.viewer.envelope_text_bytes + 1, "share").catch(() =>
      fail("too-large", "This share is larger than a viewer reads."),
    );
    const text = new TextDecoder().decode(bytes);
    // A text file that gained one final newline on the way is the same file.
    return text.endsWith("\n") ? text.slice(0, -1) : text;
  };
  const get = (url: string) => fetchImpl(url, REQUEST).catch(() => fail("unreachable", "GitHub could not be reached; try again."));
  const raw = await get(`https://${RAW_HOST}/${link.login}/${link.gist}/raw/tine-share.bin`);
  if (raw.ok) return read(raw);
  if (raw.status !== 404) fail("unreachable", `GitHub answered ${raw.status}; try again later.`);
  // The login in the link may be stale (a renamed account): ask the API once
  // where the file is, and accept only GitHub's raw host.
  const api = await get(`https://api.github.com/gists/${link.gist}`);
  if (api.status === 404) fail("not-found", "This share was deleted, or the link is wrong.");
  if (!api.ok) fail("unreachable", `GitHub answered ${api.status}; try again later.`);
  let rawUrl: URL | null = null;
  try {
    const meta = JSON.parse(new TextDecoder().decode(await readBounded(api, limits.viewer.api_response_bytes, "gist")));
    rawUrl = new URL(String(meta?.files?.["tine-share.bin"]?.raw_url ?? ""));
  } catch {
    rawUrl = null;
  }
  if (!rawUrl || rawUrl.protocol !== "https:" || rawUrl.host !== RAW_HOST) return fail("not-a-share", "This is not a Tine share.");
  const again = await get(rawUrl.href);
  if (!again.ok) fail("not-found", "This share was deleted, or the link is wrong.");
  return read(again);
}

export interface LoadedShare extends ShareDocument {
  link: ShareLink;
}

/** Load the share a fragment names: at most three requests, one at a time. */
export async function loadQuickShare(hash: string, fetchImpl: Fetch = (url, init) => fetch(url, init)): Promise<LoadedShare> {
  if (!quickShareSupported()) fail("unsupported-browser", "This browser cannot open Tine shares; use a current Firefox, Chrome, Edge or Safari.");
  const link = parseShareFragment(hash) ?? fail("bad-link", "This share link is incomplete; copy the whole link.");
  const text = await envelopeText(link, fetchImpl);
  const payload = await openEnvelope(text, link.key);
  const document = decodeShareDocument(await gunzipBounded(payload, limits.viewer.document_bytes));
  return { ...document, link };
}
