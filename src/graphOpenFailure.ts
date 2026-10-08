import { pushToast } from "./toasts";

/** Keep a graph-open failure visible until the user can retry the exact same
 * target (master 9a9122b1544d). The failure names the backend's own reason; a
 * picker has already returned its target by the time an open fails, so Retry
 * reopens that path rather than re-asking the picker. Every open entry point
 * (picked folder, known-graph row, row context menu) reports through here.
 * Cost: one toast; no I/O until Retry runs. */
export function reportGraphOpenFailure(error: unknown, retry: () => void): void {
  const detail = error instanceof Error ? error.message : String(error);
  pushToast(`Couldn't open the graph. (${detail})`, "error", {
    sticky: true,
    action: { label: "Retry", run: retry },
  });
}
