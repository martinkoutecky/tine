import type { JSX } from "solid-js";
import { backend } from "../backend";
import { pushToast } from "../toasts";
import { msg } from "../i18n";
import { graphOwner, ownedWhen, readOwned } from "../owned";
import { DEFERRED_HREF, anchorDecision, linkScheme, refusedLinkScheme, rememberableLinkScheme } from "../linkSchemePolicy";
import { loadLinkSchemeSettings, rememberedLinkSchemes, setLinkSchemeRemembered } from "../linkSchemeSettings";
import { askLinkScheme } from "../linkSchemePrompt";
export { linkScheme } from "../linkSchemePolicy";

/** The one owner of "open this outbound link" for graph content (I-12). http(s),
 *  mailto, `file:` and `tine:` open directly, as before. Another app's scheme
 *  (GH #503: `zotero://`, `obsidian://`, `shortcuts://` …) opens only after the
 *  user confirms a prompt naming the scheme and the full target, or remembers
 *  that exact scheme on this device (D-20261103-S503): a link in a
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
    if (scheme && refusedLinkScheme(scheme)) throw new Error(`${scheme}: links are never opened`);
    if (scheme && rememberableLinkScheme(scheme)) {
      if ((await readOwned(owner, loadLinkSchemeSettings())).kind === "stale") return;
      if (!rememberedLinkSchemes().includes(scheme)) {
        const answer = await readOwned(owner, askLinkScheme(scheme, dest, owner));
        if (answer.kind === "stale" || answer.value === "cancel") return;
        if (answer.value === "always") setLinkSchemeRemembered(scheme, true);
      }
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

/** The anchor attributes and activation of a graph-authored link (GH #984; the
 * policy is `anchorDecision`). Relative and web/mail destinations keep their
 * href; any other stays a focusable link whose Enter runs the same activation as
 * its click. Click and Enter run `activate` (the checked opener). */
export function outboundAnchorAttrs(dest: string, activate: () => void): JSX.AnchorHTMLAttributes<HTMLAnchorElement> {
  const decision = anchorDecision(dest);
  const run = (event: Event) => {
    event.preventDefault();
    event.stopPropagation();
    activate();
  };
  if (decision === "relative" || decision === "direct") return { href: dest, onClick: run };
  return {
    ...(decision === "defer" ? { [DEFERRED_HREF]: dest } : {}),
    role: "link", tabindex: 0, onClick: run, onKeyDown: (event) => { if (event.key === "Enter") run(event); },
  };
}

/** Delegated click/Enter for raw-HTML anchors whose destination the sanitizer
 * deferred (`deferAnchorHref`): opens it through the checked opener. */
export function activateDeferredAnchor(event: MouseEvent | KeyboardEvent): void {
  if ("key" in event && event.key !== "Enter") return;
  const node = event.target as Node | null;
  const element = node?.nodeType === 1 ? (node as Element) : node?.parentElement;
  const dest = element?.closest(`a[${DEFERRED_HREF}]`)?.getAttribute(DEFERRED_HREF);
  if (!dest) return;
  event.preventDefault();
  event.stopPropagation();
  void openOutboundLink(dest);
}
