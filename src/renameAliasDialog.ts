import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import type { AliasDialogOffer } from "./types";

/** The user's answer in the alias dialog (D-20261115-HELP): the merge the
 *  backend offered, "use the name for this page", or cancel (null). */
export type AliasDialogAnswer = "merge" | "own" | null;

export type AliasDialogRequest = {
  from: string;
  to: string;
  offer: AliasDialogOffer;
  finish: (answer: AliasDialogAnswer) => void;
};

let pendingRequest: AliasDialogRequest | null = null;
// A graph switch cancels an open question: its answer would act in the old graph.
clearOnBindingInvalidated(() => pendingRequest?.finish(null));
export const [aliasDialogRequest, setAliasDialogRequest] = graphScopedSignal<AliasDialogRequest>();

/** Show the backend's alias dialog (`RenameAliasDialog` renders it). Only an
 *  offered choice can be answered; a newer question cancels an open one;
 *  Escape, dismissal and unmounting answer null. O(1). */
export function askAliasDialog(from: string, to: string, offer: AliasDialogOffer): Promise<AliasDialogAnswer> {
  pendingRequest?.finish(null);
  return new Promise((resolve) => {
    const request: AliasDialogRequest = { from, to, offer, finish(answer) {
      if (pendingRequest !== request) return;
      if ((answer === "own" && !offer.own) || (answer === "merge" && !offer.merge)) return;
      pendingRequest = null;
      setAliasDialogRequest(null);
      resolve(answer);
    } };
    pendingRequest = request;
    setAliasDialogRequest(request);
  });
}
