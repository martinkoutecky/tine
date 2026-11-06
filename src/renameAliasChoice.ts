import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";

/** The user's answer when a rename's new name is only another page's alias
 *  (M-Q5, D-20261102-MC5): merge into that page, make the new name a page of
 *  its own (it stays an alias of the owner too), or cancel (null). */
export type AliasRenameAnswer = "merge" | "own-page" | null;

export type AliasRenameRequest = {
  from: string;
  to: string;
  /** The display name of the page whose alias `to` is. */
  owner: string;
  /** False: the merge is not allowed (the owner is one of the renamed page's
   *  own namespace children, M-Q6); only "own page" and Cancel are offered. */
  merge: boolean;
  ownPage: boolean;
  finish: (answer: AliasRenameAnswer) => void;
};

let pendingRequest: AliasRenameRequest | null = null;
// A graph switch cancels an open question: its answer would rename in the old graph.
clearOnBindingInvalidated(() => pendingRequest?.finish(null));
export const [aliasRenameRequest, setAliasRenameRequest] = graphScopedSignal<AliasRenameRequest>();

/** Ask the M-Q5 question (`RenameAliasChoice` renders it). A newer question
 *  cancels an open one. Escape, layer dismissal and unmounting answer null.
 *  O(1). */
export function chooseAliasRename(from: string, to: string, owner: string, merge: boolean, ownPage: boolean): Promise<AliasRenameAnswer> {
  pendingRequest?.finish(null);
  return new Promise((resolve) => {
    const request: AliasRenameRequest = { from, to, owner, merge, ownPage, finish(answer) {
      if (pendingRequest !== request) return;
      if ((answer === "own-page" && !ownPage) || (answer === "merge" && !merge)) return;
      pendingRequest = null;
      setAliasRenameRequest(null);
      resolve(answer);
    } };
    pendingRequest = request;
    setAliasRenameRequest(request);
  });
}
