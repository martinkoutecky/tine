import { graphScopedSignal } from "./binding";
import { backend } from "./backend";
import { bindingOwner, readOwned } from "./owned";

export type OrphanDraft = { id: string; old_root: string | null };
export const [rootRecoveryOffers, setRootRecoveryOffers] = graphScopedSignal<OrphanDraft[]>("binding");

/** Per-open notice state; Not now never writes a preference. Native discovery
 * runs off the open path and reads only device namespaces and old root metadata. */
export async function refreshRootRecovery(): Promise<void> {
  setRootRecoveryOffers([]);
  const read = backend().orphanDrafts;
  if (!read) return;
  const owner = bindingOwner();
  try {
    const result = await readOwned(owner, read.call(backend()));
    if (result.kind === "current") setRootRecoveryOffers(result.value);
  } catch { console.warn("Could not inspect device-local graph drafts"); }
}
