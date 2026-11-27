import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import type { Owner } from "./owned";

export type LinkSchemeAnswer = "cancel" | "open" | "always";
export interface LinkSchemeRequest {
  scheme: string;
  url: string;
  live: Owner;
  /** An outside-graph file (tine#985): `lossy` when its name has characters
   * that cannot be shown; `remember` offers "allow for this graph". */
  file?: { lossy: boolean; remember: boolean };
  finish(answer: LinkSchemeAnswer): void;
}

const [request, setRequest] = createSignal<LinkSchemeRequest | null>(null);
export const linkSchemeRequest = request;
clearOnBindingInvalidated(() => request()?.finish("cancel"));

/** One pending outbound gesture per window realm. A second gesture cancels the
 * previous one rather than replacing its target under a user's confirmation.
 * An aborted `signal` (the gesture's owner went away) cancels the prompt. */
export function askLinkScheme(scheme: string, url: string, live: Owner,
  options: { file?: LinkSchemeRequest["file"]; signal?: AbortSignal } = {}): Promise<LinkSchemeAnswer> {
  request()?.finish("cancel");
  if (options.signal?.aborted) return Promise.resolve("cancel");
  return new Promise(resolve => {
    const cancel = () => pending.finish("cancel");
    const pending: LinkSchemeRequest = {
      scheme, url, live, file: options.file,
      finish(answer) {
        if (request() !== pending) return;
        options.signal?.removeEventListener("abort", cancel);
        setRequest(null);
        resolve(live() ? answer : "cancel");
      },
    };
    options.signal?.addEventListener("abort", cancel);
    setRequest(pending);
  });
}
