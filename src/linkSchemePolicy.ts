import schemes from "../src-tauri/external-link-schemes.json";

const NATIVE_SCHEMES = [schemes.localFile, schemes.deepLink];

/** Absolute URL scheme as interpreted by the browser. A single letter is
 * reserved for Windows drives. O(bytes). */
export function linkScheme(dest: string): string | null {
  let protocol: string | undefined;
  try {
    protocol = new URL(dest).protocol;
  } catch (error) {
    if (!(error instanceof TypeError)) throw error;
  }
  return protocol && protocol.length > 2 ? protocol.slice(0, -1) : null;
}

export function refusedLinkScheme(scheme: string): boolean {
  return schemes.refused.includes(scheme);
}

/** Whether a parsed scheme can go directly to a browser or OS URL opener. */
export function isDirectExternalScheme(scheme: string): boolean {
  return schemes.web.includes(scheme) || scheme === schemes.mail;
}

/** Only exact, valid scheme names eligible for the existing prompt can be saved.
 * No host, path, prefix, direct-open or refused scheme can become consent. */
export function rememberableLinkScheme(scheme: string): boolean {
  return !isDirectExternalScheme(scheme) && !NATIVE_SCHEMES.includes(scheme)
    && !refusedLinkScheme(scheme) && linkScheme(`${scheme}:`) === scheme;
}

/** Check an outbound URL before handing it to the browser or native command.
 * A custom scheme needs confirmation for that exact scheme (D-20261103-S503).
 * Local files and Tine links have dedicated native routes. Throws on refusal.
 * Cost is O(URL bytes), with no I/O. */
export function checkExternalLink(
  dest: string,
  confirmedScheme: string | undefined,
  platform: "browser" | "native",
): void {
  const scheme = linkScheme(dest);
  if (!scheme) throw new Error("unsupported url scheme");
  const nativeOnly = NATIVE_SCHEMES.includes(scheme);
  if (refusedLinkScheme(scheme) || (platform === "browser" && nativeOnly)) {
    throw new Error(`${scheme}: links are never opened`);
  }
  if (!isDirectExternalScheme(scheme) && !nativeOnly && confirmedScheme !== scheme) {
    throw new Error(`opening a ${scheme}: link needs your confirmation`);
  }
}
