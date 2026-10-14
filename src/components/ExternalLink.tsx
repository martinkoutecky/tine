import type { JSX } from "solid-js";
import { backend } from "../backend";
import { pushToast } from "../toasts";
import { msg, t } from "../i18n";
import { graphOwner, ownedWhen, readOwned } from "../owned";

/** Schemes whose links open as before, with no prompt: web, mail, a local
 *  `file:` (GH #444) and Tine's own `tine:` routes. */
const DIRECT_SCHEMES = new Set(["http", "https", "mailto", "file", "tine"]);
/** Schemes no outbound link ever opens, confirmed or not (I-22); the native
 *  opener refuses them too. */
const NEVER_OPENED_SCHEMES = new Set(["javascript", "vbscript", "data", "blob", "about"]);

/** The RFC 3986 scheme of a link destination, lowercased, or null. A single
 *  letter is a Windows drive (`C:\notes`), not a scheme. O(dest bytes). */
export function linkScheme(dest: string): string | null {
  const match = /^([a-z][a-z0-9+.-]+):/i.exec(dest);
  return match ? match[1].toLowerCase() : null;
}

/** The one owner of "open this outbound link" for graph content (I-12). http(s),
 *  mailto, `file:` and `tine:` open directly, as before. Another app's scheme
 *  (GH #503: `zotero://`, `obsidian://`, `shortcuts://` …) opens only after the
 *  user confirms a prompt naming the scheme and the full target: a link in a
 *  note is untrusted content (malformed or imported Markdown, a synced note),
 *  and the prompt is the boundary before an arbitrary registered app launches.
 *  The native opener re-checks that the confirmed scheme is the link's. Code-
 *  or content-carrying schemes are never opened. Every refusal or failure is
 *  shown while the graph that showed the link is still bound (`graphScoped`,
 *  the default) or always (a caret-followed URL, which stays meaningful across
 *  a graph switch). Resolves when handled; never rejects. */
export async function openOutboundLink(dest: string, graphScoped = true): Promise<void> {
  const owner = ownedWhen(...(graphScoped ? [graphOwner()] : []));
  const scheme = linkScheme(dest);
  try {
    if (scheme && NEVER_OPENED_SCHEMES.has(scheme)) throw new Error(`${scheme}: links are never opened`);
    if (scheme && !DIRECT_SCHEMES.has(scheme)) {
      const answer = await readOwned(owner, backend().confirm(
        t("links-open-custom-scheme-confirm", { scheme, url: dest }),
        t("links-open-custom-scheme-title", { scheme }),
      ));
      if (answer.kind === "stale" || !answer.value) return;
      await readOwned(owner, backend().openExternal(dest, scheme));
      return;
    }
    await readOwned(owner, backend().openExternal(dest));
  } catch (error) {
    let current = true;
    try { current = owner(); } catch { /* an unknowable owner still reports */ }
    if (current) reportLinkOpenFailure(dest, error);
  }
}

/**
 * An outbound link in graph-authored content (master b61bb9d25303, I-22).
 * The href stays for presentation and copying, but the click never navigates
 * the WebView: it goes through `openOutboundLink`, and a refused or failed
 * open is shown to the user while the graph that showed the link is still
 * bound.
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
        void openOutboundLink(props.dest);
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
  pushToast(msg("external-link-couldn-t-open", { dest: String(dest), error: String(error) }), "error");
}
