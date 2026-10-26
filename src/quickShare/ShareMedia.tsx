import { t } from "../i18n";

function mediaDestination(value: string): { kind: "external"; url: URL } | { kind: "not-included" } {
  try {
    const url = new URL(value);
    return url.protocol === "https:" || url.protocol === "http:" ? { kind: "external", url } : { kind: "not-included" };
  } catch { return { kind: "not-included" }; }
}

/** No resource element is ever mounted; following a link is a reader action. */
export function ShareMedia(props: { url: string; label?: string }) {
  const url = () => { const destination = mediaDestination(props.url); return destination.kind === "external" ? destination.url : null; };
  return <span class="quick-share-media">{url()
    ? <a href={url()!.href} target="_blank" rel="noreferrer">{t("quick-share-viewer-remote-media", { host: url()!.host })}</a>
    : t("quick-share-viewer-asset", { name: props.url.split("/").pop() || props.label || props.url })}</span>;
}

/** Operate on inert, already sanitized HTML before insertion into the live DOM.
 * Also covers Hiccup and raw HTML images, not just the Markdown media path. */
export function shareHtml(html: string, localImages: (string | null)[] = []): string {
  const template = document.createElement("template");
  template.innerHTML = html;
  let imageIndex = 0;
  for (const resource of template.content.querySelectorAll("img,video,audio,iframe,source,object,embed")) {
    const localSource = resource.tagName === "IMG" ? localImages[imageIndex++] : null;
    const source = resource.getAttribute("src") ?? resource.getAttribute("data") ?? localSource ?? "";
    const destination = mediaDestination(source);
    const url = destination.kind === "external" ? destination.url : null;
    const replacement = document.createElement(url ? "a" : "span");
    replacement.className = "quick-share-media";
    if (url) {
      replacement.setAttribute("href", url.href); replacement.setAttribute("target", "_blank"); replacement.setAttribute("rel", "noreferrer");
    }
    replacement.textContent = url ? t("quick-share-viewer-remote-media", { host: url.host })
      : t("quick-share-viewer-asset", { name: source.replaceAll("\\", "/").split("/").pop() || resource.getAttribute("alt") || source });
    resource.replaceWith(replacement);
  }
  return template.innerHTML;
}
