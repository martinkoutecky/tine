/** The one-time "Windows Defender is slowing this graph down" hint (GH #623).
 *
 *  Question answered: after a graph opens, should Tine tell the user that
 *  Defender real-time protection is probably why a cold load was slow, and what
 *  happened when they chose to act on it?
 *
 *  Rules. The backend decides (Windows only, real-time protection on, a slow
 *  cold load, not dismissed for this graph); this module only shows the answer.
 *  Nothing changes Defender without a click, and the exclusion is a UAC prompt
 *  of the user's own accord. Dismissal is per graph and stored by the backend
 *  (`notices.json`, never in the graph). An unreadable answer costs no hint:
 *  recovery over refusal (a missing hint never harms the graph). */
import { backend } from "./backend";
import { graphOwner, readOwned } from "./owned";
import { pushToast } from "./toasts";

/** The hint wording (Martin reviews this text; the Guide page quotes it). */
export const DEFENDER_HINT_TEXT =
  "Opening this graph took a while, and Windows Defender is scanning each file as Tine reads it. Excluding this graph's folder from real-time scanning usually makes later opens much faster. This changes a Windows security setting, so it asks for administrator approval and only if you choose it.";
export const DEFENDER_HINT_ACTION = "Add an exclusion for this graph folder";

export type ExclusionOutcome =
  | { outcome: "added" }
  | { outcome: "declined" }
  | { outcome: "failed"; code: number | null; message: string };

/** Words for the result of the click. Pure, so tests pin them. */
export function exclusionResultMessage(result: ExclusionOutcome): { text: string; kind: "success" | "info" | "error" } {
  switch (result.outcome) {
    case "added":
      return { text: "Windows Defender now skips this graph's folder. Reopen the graph to feel the difference.", kind: "success" };
    case "declined":
      return { text: "No change was made: the administrator prompt was not approved. The offer returns the next time this graph opens slowly.", kind: "info" };
    case "failed":
      return { text: `Windows Defender did not accept the exclusion. ${result.message}`, kind: "error" };
  }
}

/** Ask the backend whether to show the hint for the graph that just opened; show
 *  it as a sticky toast whose action performs the exclusion. */
export async function maybeShowDefenderHint(): Promise<void> {
  const owner = graphOwner();
  let show = false;
  try {
    const answer = await readOwned(owner, backend().defenderHint());
    if (answer.kind === "stale") return;
    show = answer.value.show;
  } catch {
    return;
  }
  if (!show || !owner()) return;
  let acting = false;
  pushToast(DEFENDER_HINT_TEXT, "info", {
    sticky: true,
    action: {
      label: DEFENDER_HINT_ACTION,
      run: () => {
        acting = true;
        void backend().addDefenderExclusion().then(
          (result) => {
            const { text, kind } = exclusionResultMessage(result);
            pushToast(text, kind);
          },
          (error: unknown) => pushToast(`Could not run the exclusion: ${String(error)}`, "error"),
        );
      },
    },
    // Closing the hint (the x) is the dismissal; choosing the action is not: a
    // declined or failed attempt may be retried, and a success is recorded by
    // the backend.
    onDismiss: () => {
      if (acting) return;
      void backend().dismissDefenderHint().catch(() => {});
    },
  });
}
