import { createSignal } from "solid-js";
import { clearOnBindingInvalidated } from "./binding";
import type { Owner } from "./owned";

export type LinkSchemeAnswer = "cancel" | "open" | "always";
export interface LinkSchemeRequest {
  scheme: string;
  url: string;
  live: Owner;
  finish(answer: LinkSchemeAnswer): void;
}

const [request, setRequest] = createSignal<LinkSchemeRequest | null>(null);
export const linkSchemeRequest = request;
clearOnBindingInvalidated(() => request()?.finish("cancel"));

/** One pending outbound gesture per window realm. A second gesture cancels the
 * previous one rather than replacing its target under a user's confirmation. */
export function askLinkScheme(scheme: string, url: string, live: Owner): Promise<LinkSchemeAnswer> {
  request()?.finish("cancel");
  return new Promise(resolve => {
    const pending: LinkSchemeRequest = {
      scheme, url, live,
      finish(answer) {
        if (request() !== pending) return;
        setRequest(null);
        resolve(live() ? answer : "cancel");
      },
    };
    setRequest(pending);
  });
}
