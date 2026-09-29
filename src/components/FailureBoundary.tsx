import { ErrorBoundary, Show, createMemo, type JSX } from "solid-js";
import { slowBackendState } from "../slowBackend";
import { dbg, recordDiagnostic } from "../debug";
import { pushToastUnique } from "../toasts";

/**
 * A region that fails says so, in place, and offers to try again.
 *
 * Why this exists at all. Solid discards the WHOLE pending effect queue when a
 * render throws — `runUpdates` (solid-js 1.9.13, dist/solid.js:820) does
 * `if (!wait) Effects = null;` before it calls `handleError`, and `handleError`
 * RETHROWS when no boundary is registered. Tine registered none anywhere, so a
 * single unreadable value had three consequences at once: every effect batched
 * with it was dropped, including effects belonging to components that were
 * fine; the throw left the window as an uncaught error, which a Tauri webview
 * shows the user nothing about; and nothing rescheduled the discarded work, so
 * the app stayed blank until restart. GH #490 diagnosed exactly this inside the
 * conflict panel and worked around it locally; GH #332 is a user who has been
 * on an old release since August because his whole app came up empty and told
 * him nothing.
 *
 * What a boundary does and does not buy. `Effects = null` runs BEFORE
 * `handleError`, so co-batched effects are still lost — a boundary does not
 * make the tear disappear. What it does is stop the rethrow, render this
 * fallback, and let later updates run normally: permanent silent blank becomes
 * a visible message plus a way forward.
 *
 * Retry is the point, not a decoration. Solid's reset re-evaluates
 * `props.children`, so the subtree remounts and the resources inside it are
 * created afresh and refetch. For a failure whose cause was transient — a
 * command that lost a race with a 37-second startup — retrying is the whole
 * fix, and the user can apply it without knowing any of the above.
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
        <button type="button" class="region-failure-retry" onClick={() => props.onRetry()}>
          Retry
        </button>
      </div>
    </div>
  );
}
