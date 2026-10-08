import { bindingIdentity } from "../binding";
import { graphMeta } from "../graphSession";
import { graphTransitioning } from "../ui";
import type { PluginBlockSnapshot } from "./protocol";

/** A plugin call is owned by the graph binding, not the render epoch: a display-only
 * repaint (rename, typography, title format) must not drop plugin work in flight. */
export interface PluginGraphOwner {
  readonly graphRoot: string;
  readonly generation: string;
}

export interface OwnedPluginBlockSnapshot {
  readonly owner: PluginGraphOwner;
  readonly block: PluginBlockSnapshot;
}

export function capturePluginGraphOwner(): PluginGraphOwner | null {
  const root = graphMeta()?.root;
  if (!root || graphTransitioning()) return null;
  return Object.freeze({ graphRoot: root, generation: bindingIdentity() });
}

export function isPluginGraphOwnerCurrent(owner: PluginGraphOwner): boolean {
  return !graphTransitioning()
    && graphMeta()?.root === owner.graphRoot
    && bindingIdentity() === owner.generation;
}

export function bindPluginBlockSnapshot(block: PluginBlockSnapshot): OwnedPluginBlockSnapshot | null {
  const owner = capturePluginGraphOwner();
  if (!owner || !isPluginGraphOwnerCurrent(owner)) return null;
  return Object.freeze({ owner, block: Object.freeze({ ...block }) });
}
