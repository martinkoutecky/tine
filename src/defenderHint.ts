import { msg, type Msg } from "./i18n";
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
import { bindingOwner, readOwned, writeOwned } from "./owned";
import { pushToast } from "./toasts";

/** The hint wording (Martin reviews this text; the Guide page quotes it). */
export const DEFENDER_HINT_TEXT =
  msg("defender-hint-offer");
export const DEFENDER_HINT_ACTION = msg("defender-hint-action");

export type ExclusionOutcome =
  | { outcome: "added" }
  | { outcome: "declined" }
  | { outcome: "failed"; code: number | null; message: string };

/** Words for the result of the click. Pure, so tests pin them. */
export function exclusionResultMessage(result: ExclusionOutcome): { text: Msg; kind: "success" | "info" | "error" } {
  switch (result.outcome) {
    case "added":
      return { text: msg("defender-hint-result", { outcome: "added", detail: "" }), kind: "success" };
    case "declined":
      return { text: msg("defender-hint-result", { outcome: "declined", detail: "" }), kind: "info" };
    case "failed":
      return { text: msg("defender-hint-result", { outcome: "failed", detail: result.message }), kind: "error" };
  }
}

/** Ask the backend whether to show the hint for the graph that just opened; show
 *  it as a sticky toast whose action performs the exclusion. */
export async function maybeShowDefenderHint(): Promise<void> {
  const owner = bindingOwner();
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
  pushToast(msg(DEFENDER_HINT_TEXT.id), "info", {
    sticky: true,
    action: {
      label: DEFENDER_HINT_ACTION,
      run: () => {
        // The backend acts on the graph that is open now; a hint left over from a
        // graph that has since been closed must not change anything.
        if (!owner()) return;
        acting = true;
        void writeOwned(owner, backend().addDefenderExclusion()).then(
          (answer) => {
            // A graph switch during the elevation prompt only drops the success
            // toast; the setting itself was changed (or not) by the backend.
            if (answer.kind !== "current") return;
            const { text, kind } = exclusionResultMessage(answer.value);
            pushToast(text, kind);
          },
          (error: unknown) => pushToast(msg("defender-hint-could-not-run-the-exclusion", { error: String(error) }), "error"),
        );
      },
    },
    // Closing the hint (the x) is the dismissal; choosing the action is not: a
    // declined or failed attempt may be retried, and a success is recorded by
    // the backend.
    onDismiss: () => {
      if (acting || !owner()) return;
      void writeOwned(owner, backend().dismissDefenderHint()).then(
        () => undefined,
        (error: unknown) => pushToast(msg("defender-hint-could-not-remember-that-you-closed-the-hint", { error: String(error) }), "error"),
      );
    },
  });
}
