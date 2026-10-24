import { For, createSignal } from "solid-js";
import { t, msg } from "../i18n";
import { backend } from "../backend";
import { rootRecoveryOffers, setRootRecoveryOffers, refreshRootRecovery, type OrphanDraft } from "../rootRecoveryState";
import { refuseStaleWrite } from "../binding";
import { bindingOwner, writeOwned } from "../owned";
import { bindHost, consumedAnswer, flushAll, tryFreezeGraphRewrite } from "../document";
import { graphMeta } from "../graphSession";
import { graphTransitioning, setGraphTransitioning } from "../ui";
import { pushToast } from "../toasts";

export function RootRecoveryNotice() {
  const [busy,setBusy] = createSignal(false);
  const recover = async (orphan: OrphanDraft) => {
    const offers = rootRecoveryOffers();
    if (!offers?.includes(orphan)) return;
    const recover = backend().recoverOrphanDrafts;
    if (!recover || busy() || graphTransitioning()) return;
    const owner = bindingOwner(), root = graphMeta()?.root;
    if (!root) return;
    setBusy(true);
    setGraphTransitioning(true);
    let unfreeze = tryFreezeGraphRewrite();
    if (!unfreeze) { setBusy(false); setGraphTransitioning(false); return; }
    try {
      if (!(await flushAll()) || !owner()) {
        if (owner()) pushToast(msg("root-recovery-current-unsaved"),"error");
        return;
      }
      if (rootRecoveryOffers() !== offers) return refuseStaleWrite(t("root-recovery-recover"));
      const result = await writeOwned(owner,recover.call(backend(),orphan.id,consumedAnswer()));
      if (result.kind === "stale") return;
      const { error,reloaded } = result.value;
      if (error && !reloaded) throw error;
      // The Store and graph binding stayed in place; only the host session
      // changed. Rebind before any flush can address the stopped session. An
      // imported draft may be conflicted, so showing it cannot require a save.
      await bindHost({refreshLoaded:true});
      if (!owner()) return;
      await refreshRootRecovery();
      if (error) throw error;
    } catch (error) {
      const token = error instanceof Error ? error.message : String(error);
      if (owner() || graphMeta()?.root === root) {
        if (token === "draft-recovery-current-unsaved") pushToast(msg("root-recovery-current-unsaved"),"error");
        else if (token === "draft-recovery-owner-held") pushToast(msg("root-recovery-owner-held"),"error");
        else if (token === "draft-recovery-no-longer-available") pushToast(msg("root-recovery-unavailable"),"error");
        else if (token === "draft-recovery-lock-unavailable") pushToast(msg("root-recovery-lock-unavailable"),"error");
        else pushToast(msg("root-recovery-failed",{ error:String(error) }),"error");
      }
    } finally {
      unfreeze?.();
      if (owner() || graphMeta()?.root === root) setGraphTransitioning(false);
      setBusy(false);
    }
  };
  return <For each={rootRecoveryOffers()}>{(orphan) => <div class="root-recovery-notice" role="status" data-testid="root-recovery-notice">
    <span>{t("root-recovery-notice",{ location: orphan.old_root ?? t("root-recovery-earlier-location") })}</span>
    <button class="settings-btn" disabled={busy()} onClick={() => void recover(orphan)}>{t("root-recovery-recover")}</button>
    <button class="settings-btn" disabled={busy()} onClick={() => setRootRecoveryOffers((rootRecoveryOffers() ?? []).filter(item => item.id !== orphan.id))}>{t("root-recovery-not-now")}</button>
  </div>}</For>;
}
