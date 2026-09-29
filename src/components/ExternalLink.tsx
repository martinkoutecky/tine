import type { JSX } from "solid-js";
import { backend } from "../backend";
import { pushToast } from "../toasts";
import { graphOwner, readOwned } from "../owned";

/**
 * An outbound link in graph-authored content (master b61bb9d25303, I-22).
 * The href stays for presentation and copying, but the click never navigates
 * the WebView: it goes to the native opener, which owns the scheme allowlist,
 * and a refused or failed open is shown to the user while the graph that
 * showed the link is still bound.
 */
export function ExternalLink(props: { dest: string; class?: string; children: JSX.Element }): JSX.Element {
  return (
    <a
      class={props.class ?? "external-link"}
      href={props.dest}
      target="_blank"
      rel="noreferrer"
      onClick={(event) => {
        event.preventDefault();
        event.stopPropagation();
        void readOwned(graphOwner(), backend().openExternal(props.dest))
          .catch((error) => reportLinkOpenFailure(props.dest, error));
      }}
    >
      {props.children}
    </a>
  );
}

/** Every refused or failed outbound open is shown to the initiating user: a
 *  refused scheme, a missing file and a platform with no file manager otherwise
 *  all look like a dead link (GH #444, master c817fb150). */
export function reportLinkOpenFailure(dest: string, error: unknown): void {
  pushToast(`Couldn't open ${dest}. (${String(error)})`, "error");
}
