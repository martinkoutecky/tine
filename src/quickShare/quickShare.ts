/** Quick share generation 2, viewer side (`docs/contracts/quick-share.md`).
 *
 *  A share link is `https://tine.page/s/2/<login>/<gist_id>#<key>[/<rest>]`.
 *  This module parses the fragment, fetches the gist's `tine-share.bin`,
 *  decrypts it with WebCrypto (HKDF-SHA256 → AES-256-GCM, header as AAD),
 *  gunzips it under a running cap and admits the share document, whose
 *  `snapshot` is the published snapshot the reader already renders.
 *  Every limit comes from `quick-share-limits.json`, which the Rust writer
 *  also checks. The fragment never leaves the browser: it is not put into a
 *  request, a log or an error message. */
import limits from "../../docs/contracts/quick-share-limits.json";
import blockedGists from "./blocked-gists.json";
import { readBounded, validateSnapshot, type PublishedSnapshot } from "../publishedSnapshot";

export const QUICK_SHARE_GENERATION = 2;
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
  | "blocked"
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
  // Cannot throw: forgiving-base64 refuses only foreign characters and a length
  // of 1 mod 4, both refused above.
  const binary = atob(text.replace(/-/g, "+").replace(/_/g, "/"));
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
  return b64uEncode(bytes) === text ? bytes : null;
}

export interface ShareLink {
  login: string;
  gist: string;
  key: Uint8Array<ArrayBuffer>;
  /** The fragment key exactly as given; navigation never changes it. */
  prefix: string;
}

/** Parse the public locator and the fragment key independently. */
export function parseShareLink(pathname: string, hash: string): ShareLink | null {
  const [root, section, version, login, gist, ...extra] = pathname.split("/");
  if (root !== "" || section !== "s" || version !== String(QUICK_SHARE_GENERATION)
    || !LOGIN.test(login ?? "") || !GIST.test(gist ?? "") || extra.length) return null;
  const prefix = hash.slice(1).split("/", 1)[0];
  if (!hash.startsWith("#") || prefix.length !== 43) return null;
  const key = b64uDecode(prefix);
  if (!key || key.length !== 32) return null;
  return { login, gist, key, prefix };
}

/** Whether this browser has what a share needs. */
export function quickShareSupported(): boolean {
  if (typeof globalThis.crypto?.subtle?.deriveKey !== "function" || typeof globalThis.DecompressionStream !== "function") return false;
  try { new DecompressionStream("gzip"); return true; } catch { return false; }
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
export async function gunzipBounded(bytes: Uint8Array<ArrayBuffer>, max: number): Promise<Uint8Array<ArrayBuffer>> {
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

/** Admit the gunzipped source document before opening the memory graph. */
export function decodeShareDocument(bytes: Uint8Array): PublishedSnapshot {
  let doc: { quick_share?: unknown; snapshot?: unknown } | null;
  try {
    doc = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(bytes));
  } catch {
    return fail("invalid", "This share is damaged.");
  }
  const generation = doc?.quick_share;
  if (typeof generation === "number" && generation > QUICK_SHARE_GENERATION) {
    fail("newer-viewer", "This share needs a newer viewer.");
  }
  if (generation !== QUICK_SHARE_GENERATION) fail("invalid", "This share is damaged.");

  let snapshot: PublishedSnapshot;
  try {
    snapshot = validateSnapshot(doc?.snapshot);
  } catch {
    return fail("invalid", "This share is damaged.");
  }
  const v = limits.viewer;
  if (snapshot.files.length > v.pages + 1) fail("too-large", "This share is larger than a viewer reads.");
  const encoder = new TextEncoder();
  const tooLong = (text: string) => text.length > v.string_bytes ||
    (text.length * 3 > v.string_bytes && encoder.encode(text).length > v.string_bytes);
  if (tooLong(snapshot.name) || tooLong(snapshot.exported_at)
    || snapshot.files.some(file => tooLong(file.path) || tooLong(file.text))) {
    fail("too-large", "This share holds a source file larger than a viewer reads.");
  }
  return snapshot;
}

type Fetch = (url: string, init?: RequestInit) => Promise<Response>;
// A redirect would leave the hosts this viewer trusts and escape the request
// count, so none is followed: GitHub serves both answers directly.
const REQUEST: RequestInit = { credentials: "omit", referrerPolicy: "no-referrer", redirect: "error" };

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

/** Fetch, authenticate and decompress the share. The worker admits the resulting bytes. */
export async function loadQuickShare(pathname: string, hash: string, fetchImpl: Fetch = (url, init) => fetch(url, init)) {
  if (!quickShareSupported()) fail("unsupported-browser", "This browser cannot open Tine shares; use a current Firefox, Chrome, Edge or Safari.");
  // Presence of subtle alone does not guarantee its HKDF/AES implementation.
  // Probe algorithms before contacting GitHub, using public all-zero bytes.
  try {
    const base = await crypto.subtle.importKey("raw", new Uint8Array(32), "HKDF", false, ["deriveKey"]);
    await crypto.subtle.deriveKey({ name: "HKDF", hash: "SHA-256", salt: new Uint8Array(16), info: ENC_INFO }, base, { name: "AES-GCM", length: 256 }, false, ["decrypt"]);
  } catch {
    fail("unsupported-browser", "This browser cannot open Tine shares; use a current Firefox, Chrome, Edge or Safari.");
  }
  const link = parseShareLink(pathname, hash) ?? fail("bad-link", "This share link is incomplete; copy the whole link.");
  if ((blockedGists as string[]).includes(link.gist)) fail("blocked", "This share is unavailable on tine.page.");
  const text = await envelopeText(link, fetchImpl);
  const payload = await openEnvelope(text, link.key);
  return { bytes: await gunzipBounded(payload, limits.viewer.document_bytes), link };
}
