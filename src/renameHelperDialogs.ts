import { clearOnBindingInvalidated, graphScopedSignal } from "./binding";
import type { ContestedOffer, DayMoveOffer } from "./types";

/** An open day-move or contested-name question (D-20261115-HELP), rendered by
 *  `RenameHelperDialog`. `finish(null)` cancels; the day move answers `true`,
 *  the contested dialog the file the user keeps. */
export type RenameHelperRequest =
  | { kind: "dayMove"; from: string; to: string; offer: DayMoveOffer; finish: (answer: true | null) => void }
  | { kind: "contested"; from: string; to: string; offer: ContestedOffer; finish: (answer: string | null) => void };

let pending: { finish: (answer: null) => void } | null = null;
// A graph switch cancels an open question: its answer would act in the old graph.
clearOnBindingInvalidated(() => pending?.finish(null));
export const [renameHelperRequest, setRenameHelperRequest] = graphScopedSignal<RenameHelperRequest>();

function ask<A>(make: (finish: (answer: A | null) => void) => RenameHelperRequest, valid: (answer: A) => boolean): Promise<A | null> {
  pending?.finish(null);
  return new Promise((resolve) => {
    const entry = { finish(answer: A | null) {
      if (pending !== entry || (answer !== null && !valid(answer))) return;
      pending = null;
      setRenameHelperRequest(null);
      resolve(answer);
    } };
    pending = entry;
    setRenameHelperRequest(make((answer) => entry.finish(answer)));
  });
}

/** Ask whether to move the day's notes (`offer`); true to move. O(1). */
export function askDayMove(from: string, to: string, offer: DayMoveOffer): Promise<true | null> {
  return ask<true>((finish) => ({ kind: "dayMove", from, to, offer, finish }), () => true);
}

/** Ask which of the claiming files to keep (the others merge into it); only
 *  a listed file answers. O(files). */
export function askContested(from: string, to: string, offer: ContestedOffer): Promise<string | null> {
  return ask<string>((finish) => ({ kind: "contested", from, to, offer, finish }), (file) => offer.files.includes(file));
}
