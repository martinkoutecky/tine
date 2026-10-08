import { ErrorBoundary, Show, createMemo, type JSX } from "solid-js";
import { writeClipboardText } from "../clipboard";
import { slowBackendState } from "../slowBackend";
import { dbg, recordDiagnostic } from "../debug";
import { pushToastUnique } from "../toasts";

/**
 * Independently loaded/rendered surfaces own their failure (I-20).
 * Wrap the component mount, not only its returned JSX: resource creation and
 * render effects must belong to this boundary. Cost O(rendered subtree); no
 * graph writes. Initial/reactive throws show a logged error and sticky toast.
 * Solid may discard co-batched effects before error handling; the boundary
 * stops the escaping throw and permits later updates, not atomic batching.
 * Retry remounts only the failed subtree and reissues its resources. Event
 * handlers and detached promises retain their explicit failure handlers.
 * Copy details sends the region and error stack/message to the clipboard; a
 * failed clipboard write reports a sticky error. Diagnostic records use only
 * the fixed uncaught_error kind; free text stays in opt-in logs and local UI.
 * Exemplar mounts: RightSidebar.tsx's SidebarItemView and App.tsx's dialogs.
 */
export function FailureBoundary(props: {
  /** Stable, human-readable name of what failed. Shown and logged. */
  region: string;
  children: JSX.Element;
}): JSX.Element {
  return (
    <ErrorBoundary
      fallback={(error, reset) => (
        <FailureRegion region={props.region} error={error} onRetry={reset} />
      )}
    >
      {props.children}
    </ErrorBoundary>
  );
}

function FailureRegion(props: {
  region: string;
  error: unknown;
  onRetry: () => void;
}): JSX.Element {
  // The always-on flight recorder takes only a fixed kind (privacy-safe by
  // construction); the region name and the message go to the opt-in debug log
  // and to the user's own screen, never to the always-on record.
  void recordDiagnostic("uncaught_error");
  dbg(`region failed: ${props.region}: ${String(props.error)}`);
  pushToastUnique(`${props.region} could not be displayed.`, "error");

  const message = () => {
    const error = props.error;
    if (error instanceof Error && error.message) return error.message;
    const text = typeof error === "string" ? error : "";
    return text || "Unexpected failure.";
  };

  // The backend's own slowness, said out loud. GH #332's report carried a 37s
  // startup and a wall of commands past the slow threshold, and the app
  // displayed none of it; the reporter reasonably read a blank window as lost
  // data. When the failure lands while the backend is still busy, that is
  // almost always the explanation, and it is the thing that makes Retry the
  // right next step rather than a shot in the dark.
  const busy = createMemo(() => {
    const state = slowBackendState();
    if (state.count < 1) return null;
    return { count: state.count, seconds: Math.max(1, Math.round(state.longestMs / 1000)) };
  });

  return (
    <div class="region-failure" role="alert" data-failed-region={props.region}>
      <div class="region-failure-title">{props.region} could not be displayed.</div>
      <div class="region-failure-message">{message()}</div>
      <Show when={busy()}>
        {(state) => (
          <div class="region-failure-slow">
            Tine is still waiting on {state().count === 1 ? "an operation" : `${state().count} operations`} that
            {" "}{state().count === 1 ? "has" : "have"} been running for over {state().seconds}s. This is usually
            why a region fails to appear, and your notes on disk are not affected.
          </div>
        )}
      </Show>
      <div class="region-failure-actions">
        <button type="button" onClick={() => {
          const details = `${props.region} could not be displayed.\n${props.error instanceof Error ? props.error.stack || message() : message()}`;
          void writeClipboardText(details).catch(() => pushToastUnique("Couldn’t copy error details: clipboard write failed.", "error"));
        }}>Copy details</button>
        <button type="button" class="region-failure-retry" onClick={() => props.onRetry()}>
          Retry
        </button>
      </div>
    </div>
  );
}
