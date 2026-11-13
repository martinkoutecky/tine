import { createSignal } from "solid-js";
import { backend } from "./backend";
import { bindingOwner, readOwned } from "./owned";
import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import { loadPreference, seedPreference, writePreference } from "./preferenceWrites";
import { commentAuthorName, commentNameLoaded, normalizeAuthorName, setCommentAuthorName } from "./commentIdentity";
import { msg } from "./i18n";
import { pushToast } from "./toasts";
import { dbg } from "./debug";

// Ask for "Your name" once (D-20261101-CN, tine#841). Someone who never set it
// writes no `tine.author::`, and such a comment reads "you" on every device, also to
// the people who did not write it. In a graph shared through git that hides who
// wrote what, and the setting is easy to miss. So the first margin comment or
// reply in a git graph, with no name set, opens a small dialog first.
//
// The name is a Tine setting on this device (commentIdentity.ts), used in every
// graph. Declining, by the Skip button, Escape or a click outside, is remembered
// on the device the same way, so the person is never asked again; Settings keeps
// the field. The comment is never lost to the prompt: it is created after the
// dialog closes, with the name when one was given and without `tine.author::` when not.
//
// No prompt beats a wrong prompt: nothing is asked until the name and the
// declined flag have loaded from the device settings and the graph has answered
// that it sits in a git work tree. An answer still on its way means the comment
// is written at once, as before.

const DECLINED_KEY = "comment_name_prompt_declined";

const [declinedSig, setDeclinedSig] = createSignal(false);
const [declinedLoadedSig, setDeclinedLoadedSig] = createSignal(false);
// true = the graph is inside a git work tree; false = it answered it is not;
// undefined = not asked, not answered yet, or the question failed.
const [repositorySig, setRepositorySig] = createSignal<boolean | undefined>(undefined);
let probing = false;
clearOnBindingInvalidated(() => { setRepositorySig(undefined); probing = false; });

/** Load the "declined" flag at startup; a read failure toasts and leaves it unknown (no prompt). */
export function initCommentNamePrompt(): void {
  loadPreference(declinedSig, (v) => { setDeclinedSig(v); setDeclinedLoadedSig(true); },
    () => backend().getAppString(DECLINED_KEY, ""), (v) => v === "1", "comment name prompt");
}

/** Ask the backend once whether the open graph sits in a git work tree. Never
 * throws; a failed or stale answer leaves it unknown. The source is the same
 * status Settings > Graph > Git shows (`gitAutocommitStatus().repository`). */
export function probeGraphGit(): void {
  if (probing) return;
  probing = true;
  const owner = bindingOwner();
  void readOwned(owner, backend().gitAutocommitStatus())
    .then((answer) => { if (answer.kind === "current") setRepositorySig(answer.value.available !== false && answer.value.repository === true); })
    .catch((error) => dbg(`comment name: git probe failed: ${String(error)}`)) // unknown means no prompt
    .finally(() => { probing = false; });
}

function nameWanted(): boolean {
  return commentNameLoaded() && declinedLoadedSig() && commentAuthorName() === "" && !declinedSig();
}

/** True when the next comment or reply should first ask for a name. Reactive. */
export function shouldAskCommentName(): boolean {
  return nameWanted() && repositorySig() === true;
}

export interface CommentNameRequest {
  /** A name saves it; `null` or a blank name declines (remembered). Runs the held comment either way. */
  finish(name: string | null): void;
}
let pending: CommentNameRequest | null = null;
export const [commentNameRequest, setCommentNameRequest] = graphScopedSignal<CommentNameRequest>("binding");
// The graph changed under the dialog: the held comment names a block of the old
// graph, so it is dropped, with a notice.
clearOnBindingInvalidated(() => {
  if (!pending) return;
  pending = null;
  setCommentNameRequest(null);
  pushToast(msg("comment-name-prompt-graph-changed"), "error");
});

/** Run `run` (create the comment or reply) now, or, when this is the first one
 * of a person with no name in a git graph, after the name dialog closes. `run`
 * is called exactly once, whatever the answer; the dialog never blocks it. It
 * re-checks anything it captured, since the dialog is open for a while. */
export function runWithCommentName(run: () => void): void {
  if (!nameWanted()) return run();
  if (repositorySig() !== true) { probeGraphGit(); return run(); }
  if (pending) return; // the dialog is open for the gesture that opened it
  const request: CommentNameRequest = {
    finish(name) {
      if (pending !== request) return;
      pending = null;
      setCommentNameRequest(null);
      const normalized = normalizeAuthorName(name ?? "");
      if (normalized) setCommentAuthorName(normalized);
      else writePreference(declinedSig, setDeclinedSig, true, () => backend().setAppString(DECLINED_KEY, "1"), msg("toast-save-comment-prompt-failed"));
      run();
    },
  };
  pending = request;
  setCommentNameRequest(request);
}

/** Test seam: set the state without persisting. */
export function resetCommentNamePromptForTests(state: { declined?: boolean; loaded?: boolean; repository?: boolean | undefined } = {}): void {
  pending = null;
  setCommentNameRequest(null);
  probing = false;
  setDeclinedSig(state.declined ?? false);
  setDeclinedLoadedSig(state.loaded ?? true);
  setRepositorySig(state.repository);
  seedPreference(declinedSig);
}
