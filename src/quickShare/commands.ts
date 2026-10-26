// Quick share (DESIGN §3.9, docs/contracts/quick-share.md): the shapes the native
// `quick_share_*` commands answer, and the sentence each failure shows. Every
// GitHub call, the token and every share key stay in Rust; a link reaches the
// clipboard from Rust and never this webview.
import { t } from "../i18n";

/** A command's failure (`QsError` in src-tauri/src/quick_share/mod.rs). */
export type QuickShareError =
  | { kind: "disabled" }
  | { kind: "notConnected" }
  | { kind: "reconnect" }
  | { kind: "rateLimited"; minutes: number }
  | { kind: "github"; message: string }
  | { kind: "network" }
  | { kind: "unavailable" }
  | { kind: "maybeUploaded"; shareId: string }
  | { kind: "maybeUpdated" }
  | { kind: "shareChanged" }
  | { kind: "previewExpired" }
  | { kind: "previewCapacity" }
  | { kind: "wrongAccount"; login: string }
  | { kind: "notFound" }
  | { kind: "refused"; message: string }
  | { kind: "records"; message: string };

export type QuickShareConnectStatus =
  | { state: "pending" }
  | { state: "connected"; login: string }
  | { state: "expired" }
  | { state: "denied" }
  | { state: "failed"; reason: "superseded" | "scope" | "refused" | "records" | string };

export interface QuickShareConnectStart { attemptId: string; userCode: string; verificationUri: string; expiresIn: number }
export interface QuickSharePreview {
  title: string;
  pages: { name: string; path: string; journal: boolean }[];
  leftOut: { name: string; path: string; reason: "private" | "exportsPage" }[];
  payloadBytes: number;
  envelopeBytes: number;
}
/** `shareId`: the record id the create will use; the share dialog marks the query block `tine.share:: pending/<shareId>`
 *  with it before the upload (D-20261018-QS3). Not secret (it is in the envelope header). */
export interface QuickSharePrepared { prepareId: string; shareId: string; preview: QuickSharePreview; membership?: { added: string[]; removed: string[]; currentSnapshotAt?: string } | null }
export interface QuickShareCreated { shareId: string; gistId: string; login: string; copied: boolean }
export interface QuickShareListed {
  shareId: string;
  state: "pending" | "live";
  login: string;
  gistId: string | null;
  createdAt: string;
  title: string;
  pages: number;
  /** The connected account owns it: Check and Delete are possible. */
  accountMatches: boolean;
}
export interface QuickShareListing {
  /** Quick share is on in this build (DESIGN §5: off until the viewer is deployed). */
  enabled: boolean;
  /** Native debug builds only; never initiates a request by itself. */
  connectionTestAvailable?: boolean;
  account: { login: string; accountId: number } | null;
  shares: QuickShareListed[];
  /** The shares file could not be read; `shares` is then empty. */
  error: string | null;
}
export type QuickShareConnectionTest = "connected" | "tls" | "network" | "refused" | "badAnswer" | "disabled";
export type QuickShareCheckOutcome = "live" | "notFoundYet" | { current: { title: string; createdAt: string; pages: string[] } };
export type QuickShareDeleteOutcome = "deleted" | "gone";
export interface QuickShareFound { gistId: string; createdAt: string }

/** The disabled listing: what a backend without quick share answers. */
export const QUICK_SHARE_OFF: QuickShareListing = Object.freeze({ enabled: false, account: null, shares: [], error: null });

/** GitHub's page for revoking Tine's grant (DESIGN §3.9; OAUTH-APP.md client id). */
export const GITHUB_APPLICATION_SETTINGS = "https://github.com/settings/connections/applications/Ov23liMHqsgTkxqKqFG6";

function isQuickShareError(error: unknown): error is QuickShareError {
  return typeof error === "object" && error !== null && typeof (error as { kind?: unknown }).kind === "string";
}

/** The sentence a failure shows: it names the cause (DESIGN §3.9). */
export function quickShareErrorText(error: unknown): string {
  if (!isQuickShareError(error)) return t("quick-share-error-other", { error: error instanceof Error ? error.message : String(error) });
  switch (error.kind) {
    case "disabled": return t("quick-share-error-disabled");
    case "notConnected": return t("quick-share-error-not-connected");
    case "reconnect": return t("quick-share-error-reconnect");
    case "rateLimited": return t("quick-share-error-rate-limited", { minutes: Math.max(1, error.minutes) });
    case "github": return t("quick-share-error-github", { message: error.message });
    case "network": return t("quick-share-error-network");
    case "unavailable": return t("quick-share-error-unavailable");
    case "maybeUploaded": return t("quick-share-error-maybe-uploaded");
    case "maybeUpdated": return t("quick-share-error-maybe-updated");
    case "shareChanged": return t("quick-share-error-share-changed");
    case "previewExpired": return t("quick-share-error-preview-expired");
    case "previewCapacity": return t("quick-share-error-preview-capacity");
    case "wrongAccount": return t("quick-share-error-wrong-account", { login: error.login });
    case "notFound": return t("quick-share-error-not-found");
    case "refused": return t("quick-share-error-refused", { message: error.message });
    case "records": return t("quick-share-error-records", { message: error.message });
  }
}

/** The sentence an ended connection attempt shows. */
export function quickShareConnectText(status: QuickShareConnectStatus): string {
  switch (status.state) {
    case "pending": return t("quick-share-connect-waiting");
    case "connected": return t("quick-share-connected-as", { login: status.login });
    case "expired": return t("quick-share-connect-expired");
    case "denied": return t("quick-share-connect-denied");
    case "failed":
      return status.reason === "scope" ? t("quick-share-connect-no-gist-scope")
        : status.reason === "records" ? t("quick-share-connect-records")
        : status.reason === "superseded" ? t("quick-share-connect-superseded")
        : t("quick-share-connect-refused");
  }
}

/** The share's size as shown before upload (the envelope text GitHub stores). */
export function quickShareSize(bytes: number): string {
  return bytes < 1024 ? t("quick-share-size-bytes", { bytes })
    : bytes < 1024 * 1024 ? t("quick-share-size-kib", { size: (bytes / 1024).toFixed(1) })
    : t("quick-share-size-mib", { size: (bytes / (1024 * 1024)).toFixed(1) });
}
