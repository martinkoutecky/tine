import { msg } from "./i18n";
import { createSignal } from "solid-js";
import { backend } from "./backend";
import { writeGraphSignal } from "./graphPreferences";

// Task workflow from config.edn (:preferred-workflow): every editor surface
// reads this signal. Each independent WebView seeds it from its bound graph.
export const [workflow, setWorkflow] = createSignal<"now" | "todo">("now");

export function applyTaskWorkflow(meta: { preferred_workflow: string }) {
  setWorkflow(meta.preferred_workflow === "todo" ? "todo" : "now");
}

/** Apply Settings immediately; the guarded config write rolls back on failure. */
export function changeWorkflow(wf: "now" | "todo") {
  if (wf === workflow()) return;
  writeGraphSignal("workflow", workflow, setWorkflow, wf,
    (next) => backend().setPreferredWorkflow(next), msg("toast-save-workflow-failed"));
}
