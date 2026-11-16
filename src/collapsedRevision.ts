import { createMemo, createSignal, untrack, type Accessor } from "solid-js";
import { graphEpoch } from "./graphSession";
import { bindingIdentity } from "./binding";

/** A collapsed section keeps its last answer, with explicit freshness. Opening
 * exposes the accumulated revision once; successful reads acknowledge exactly
 * the revision they loaded, so a change during a read remains stale. */
export function createCollapsedRevision(revision: Accessor<string>, collapsed: Accessor<boolean>, identity: Accessor<string>) {
  const identityScope = () => `${bindingIdentity()}\0${graphEpoch()}\0${identity()}`;
  const live = createMemo(() => `${identityScope()}\0${revision()}`);
  const [acknowledged, setAcknowledged] = createSignal(untrack(live));
  let scope = untrack(identityScope);
  const frozen = createMemo((previous: string | undefined) => {
    const current = live();
    const nextScope = identityScope();
    const same = scope === nextScope;
    scope = nextScope;
    return collapsed() && same && previous !== undefined ? previous : current;
  });
  return { revision: frozen, stale: () => collapsed() && live() !== acknowledged(),
    remember: (token: string) => setAcknowledged(token) };
}
