import { graphEpoch, graphMeta, graphTransitioning } from "../ui";
import type { PluginBlockSnapshot } from "./protocol";

export interface PluginGraphOwner {
  readonly graphRoot: string;
  readonly generation: number;
}

export interface OwnedPluginBlockSnapshot {
  readonly owner: PluginGraphOwner;
  readonly block: PluginBlockSnapshot;
}

export function capturePluginGraphOwner(): PluginGraphOwner | null {
  const root = graphMeta()?.root;
  if (!root || graphTransitioning()) return null;
  return Object.freeze({ graphRoot: root, generation: graphEpoch() });
}

export function isPluginGraphOwnerCurrent(owner: PluginGraphOwner): boolean {
  return !graphTransitioning()
    && graphMeta()?.root === owner.graphRoot
    && graphEpoch() === owner.generation;
}

export function bindPluginBlockSnapshot(block: PluginBlockSnapshot): OwnedPluginBlockSnapshot | null {
  const owner = capturePluginGraphOwner();
  if (!owner || !isPluginGraphOwnerCurrent(owner)) return null;
  return Object.freeze({ owner, block: Object.freeze({ ...block }) });
}
