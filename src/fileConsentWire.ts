// The outside-file Ask on the wire (tine#985). No imports: the backend decodes
// it, and the consent driver (which imports the prompt, which imports the
// binding, which imports the backend) must not sit in the backend's import graph.

/** Why a `file:` call is made. The native side answers the first `link` call
 * with an Ask when the target is not proven inside the graph. */
export type FilePurpose = "link" | "link-agreed" | "link-agreed-remember";
/** A note link's `file:` call: its purpose, the click's binding generation and,
 * in Quick Capture, the native show the click came from. */
export interface FileLinkCall { purpose: FilePurpose; bindingGeneration: number; show?: number }

/** The exact native token: `file-consent:` and serde_json's object with sorted
 * keys. The string pattern admits only valid JSON escapes, so parsing it
 * cannot throw. */
const WIRE = /^file-consent:\{"display":("(?:[^"\\\u0000-\u001f]|\\(?:["\\/bfnrt]|u[0-9a-fA-F]{4}))*"),"lossy":(true|false)\}$/;

/** The native Ask: nothing was opened or read; `display` names the file. */
export class FileConsentAsk extends Error {
  constructor(readonly display: string, readonly lossy: boolean) {
    super("file-consent");
  }
}

/** Decode the Ask wire token, or null for any other error. O(message length);
 * like `errorFamily`, only the exact token is recognized. */
export function fileConsentAsk(error: unknown): FileConsentAsk | null {
  const wire = WIRE.exec(error instanceof Error ? error.message : String(error));
  return wire ? new FileConsentAsk(JSON.parse(wire[1]) as string, wire[2] === "true") : null;
}
