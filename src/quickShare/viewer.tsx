import { t, type MessageId } from "../i18n";
import { QuickShareError, type QuickShareReason } from "./quickShare";

const errors: Record<QuickShareReason, MessageId> = {
  "bad-link": "quick-share-viewer-bad-link",
  "unsupported-browser": "quick-share-viewer-unsupported-browser",
  "not-found": "quick-share-viewer-not-found",
  unreachable: "quick-share-viewer-unreachable",
  "too-large": "quick-share-viewer-too-large",
  "not-a-share": "quick-share-viewer-not-a-share",
  "newer-viewer": "quick-share-viewer-newer-viewer",
  unauthentic: "quick-share-viewer-unauthentic",
  invalid: "quick-share-viewer-invalid",
  blocked: "quick-share-viewer-blocked",
};

export function QuickShareBanner() {
  return <aside class="quick-share-banner" aria-label={t("quick-share-viewer-user-content")}>
    <span>{t("quick-share-viewer-user-content")}</span>{" · "}
    <a href="mailto:support@tine.page" rel="noreferrer">{t("quick-share-viewer-report-abuse")}</a>
  </aside>;
}

export function QuickShareFailure(props: { error: unknown }) {
  // Never render exception text: a transport/browser error may include a URL.
  const reason = props.error instanceof QuickShareError ? props.error.reason : "invalid";
  return <main class="quick-share-failure" role="alert">
    <h1>{t("quick-share-viewer-title")}</h1>
    <p>{t(errors[reason])}</p>
  </main>;
}
