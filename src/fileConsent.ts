import { askLinkScheme } from "./linkSchemePrompt";
import { readOwned, type Owned, type Owner } from "./owned";
import { FileConsentAsk, type FilePurpose } from "./fileConsentWire";

export { FileConsentAsk, fileConsentAsk, type FileLinkCall, type FilePurpose } from "./fileConsentWire";

/** The one consent driver for every `file:` gesture (note links, rendered
 * graph-relative links, outside PDFs). The first call carries `link`; an Ask
 * shows the prompt naming the file; Open repeats the call with the user's
 * answer. `call` must carry the gesture's own binding generation, so an agreed
 * call after a graph switch is refused natively. `owner` retires a late first
 * reply (no dialog) and a dead dialog; `signal` (a capture show, a pending PDF
 * document) cancels a displayed dialog when its owner goes. Two IPC at most;
 * a failure rejects only while the owner is current. */
export async function withFileConsent<T>(
  owner: Owner,
  call: (purpose: FilePurpose) => Promise<T>,
  options: { signal?: AbortSignal; remember?: boolean } = {},
): Promise<Owned<T> | { kind: "cancelled" }> {
  try {
    return await readOwned(owner, call("link"));
  } catch (error) {
    if (!(error instanceof FileConsentAsk)) throw error;
    const answer = await readOwned(owner, askLinkScheme("file", error.display, owner, {
      file: { lossy: error.lossy, remember: options.remember ?? true }, signal: options.signal,
    }));
    if (answer.kind === "stale" || answer.value === "cancel") return { kind: "cancelled" };
    return readOwned(owner, call(answer.value === "always" ? "link-agreed-remember" : "link-agreed"));
  }
}
