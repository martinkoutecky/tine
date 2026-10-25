import { backend } from "./backend";
import { appendToTodayJournal, captureToPage, flushPage } from "./document";
import { bindingOwner, ownedWhen, readOwned, readOwnedResource, writeOwned, type WriteOwner } from "./owned";
import { pushToast } from "./toasts";
import { msg } from "./i18n";
import type { QuickCaptureAck, QuickCaptureRequest } from "./quickCaptureAck";

/** Install the graph window's capture receiver. Each request is deduplicated
 * by id (100 completed ids retained); writes use the document capture door.
 * Work is O(captured blocks + destination page). Transport setup failures reject;
 * Requests must carry the native capture-show binding generation; stale or
 * missing generations acknowledge false. The surface owner covers registration
 * and callbacks; writes capture bindingOwner before starting. Save failure
 * acknowledges false and preserves the sender's scratch. Dispose
 * the returned listener when the app surface retires. */
export async function installQuickCaptureReceiver(live: WriteOwner = ownedWhen(() => true)): Promise<() => void> {
  const owner = ownedWhen(live);
  // Forget changes only process-memory custody. Its result remains meaningful
  // after the graph or listener that acquired the claim has retired.
  const receiptOwner = ownedWhen(() => true);
  const inFlight = new Map<string, Promise<boolean>>();
  // Pending insertions are retained independently of the completed-id cap.
  const landed = new Map<string, { name: string; owner: WriteOwner }>();
  const uncertain = new Set<string>();
  // A failed receipt lookup proves neither insertion nor non-insertion. Keep
  // the retry identity, but offer only a plain retry until custody is known.
  const unconfirmed = new Set<string>();
  const receiptStatus = async (id: string) => {
    try {
      // The listener surface owns this read, deliberately across graph bindings.
      const result = await readOwned(owner, backend().captureReceipt(id, "status", backend().graphBindingGeneration()));
      if (result.kind === "stale") { unconfirmed.add(id); return "unknown"; }
      const status = result.value;
      unconfirmed.delete(id);
      if (status === "pending") uncertain.add(id);
      return status;
    } catch {
      unconfirmed.add(id);
      return "unknown";
    }
  };
  const completed = new Map<string, boolean>();
  const completedOrder: string[] = [];
  const rememberCompleted = (id: string, ok: boolean) => {
    completed.set(id, ok);
    completedOrder.push(id);
    while (completedOrder.length > 100) {
      const old = completedOrder.shift();
      if (old) completed.delete(old);
    }
  };
  const { emitTo, listen } = await import("@tauri-apps/api/event");
  const { getCurrentWindow } = await import("@tauri-apps/api/window");
  const windowLabel = getCurrentWindow().label;
  const ack = (id: string | undefined, ok: boolean) => {
    if (id) void emitTo("capture", "quick-capture-ack", { id, ok,
      ...(!ok && (landed.has(id) || uncertain.has(id) || unconfirmed.has(id)) ? { landed: true } : {}),
      ...(!ok && uncertain.has(id) ? { closedGraph: true } : {}),
    } satisfies QuickCaptureAck);
  };
  if (!owner()) return () => {};
  const registration = await readOwnedResource(owner, listen<QuickCaptureRequest & { bindingGeneration: number }>("quick-capture", async (e) => {
    // WebKitGTK currently exposes targeted Tauri events to every graph
    // listener in this process. Treat the payload label as the authority so
    // only the selected graph can ever perform the write.
    if (!owner() || e.payload?.target !== windowLabel) return;
    const id = e.payload?.id;
    if (id && completed.has(id)) {
      ack(id, completed.get(id) ?? false);
      return;
    }
    const existing = id ? inFlight.get(id) : undefined;
    if (existing) {
      ack(id, await existing);
      return;
    }
    if (!Number.isSafeInteger(e.payload.bindingGeneration) || e.payload.bindingGeneration <= 0
        || e.payload.bindingGeneration !== backend().graphBindingGeneration()) {
      const status = id ? await receiptStatus(id) : "missing";
      ack(id, status === "saved");
      return;
    }
    const saveOwner = bindingOwner(owner);
    const text = e.payload?.text ?? "";
    if (!text.trim()) {
      ack(id, false);
      return;
    }
    // A title routes the capture to a NEW (or existing) page; empty → today.
    const title = (e.payload?.title ?? "").trim();
    const save = async () => {
      let ok = false;
      let newClaim = false;
      const forgetUninserted = async (): Promise<boolean | undefined> => {
        if (id && newClaim && !landed.has(id)) {
          const forgotten = await writeOwned(receiptOwner, backend().captureReceipt(id, "forget", e.payload.bindingGeneration));
          newClaim = false;
          return forgotten.kind === "current" ? forgotten.value === "saved" : undefined;
        }
      };
      try {
        const prior = id ? landed.get(id) : undefined;
        // Native process memory outlives this listener/WebView. A claim precedes
        // insertion, so a reload at any await cannot authorize a second insert.
        // Legacy idless deliveries are one-shot inserts, as before; they have
        // no acknowledgement/retry protocol to claim.
        if (id) {
          const claimed = await writeOwned(saveOwner, backend().captureReceipt(id, "claim", e.payload.bindingGeneration)
            .then(receipt => { newClaim = receipt === "new"; return receipt; }));
          if (claimed.kind === "stale") {
            // No insertion was called. Only this newly claimed attempt may
            // forget its pending receipt, even after its binding retired.
            const forgotten = await forgetUninserted();
            return forgotten ?? (await receiptStatus(id) === "saved");
          }
          const receipt = claimed.value;
          if (receipt === "saved") return true;
          if ((receipt === "pending" && !prior) || (prior && !prior.owner())) {
            if (receipt === "new" || receipt === "pending") uncertain.add(id);
            return false;
          }
        }
        const inserted = (name: string) => { if (id) landed.set(id, { name, owner: saveOwner }); };
        const result = await writeOwned(saveOwner, prior
          ? (prior.owner() ? flushPage(prior.name) : Promise.resolve(false))
          : title ? captureToPage(title, text, inserted) : appendToTodayJournal(text, inserted));
        ok = result.kind === "current" && result.value;
        if (ok && id) {
          const remembered = await writeOwned(saveOwner, backend().captureReceipt(id, "saved", e.payload.bindingGeneration));
          if (remembered.kind === "stale") ok = await receiptStatus(id) === "saved";
        } else {
          ok = (await forgetUninserted()) ?? false;
        }
      } catch {
        ok = false;
        // A known-new claim with no insertion callback remains safe to drop.
        // If transport fails, retain it and ask status conservatively below.
        let forgotten: boolean | undefined;
        try { forgotten = await forgetUninserted(); } catch { /* status decides custody */ }
        if (id) ok = forgotten ?? (await receiptStatus(id) === "saved");
      }
      if (saveOwner()) {
        if (!ok) pushToast(msg("app-capture-couldn-t-be-saved-its-text"), "error");
        else if (title) pushToast(msg("app-captured-to", { title }), "info");
        else pushToast(msg("app-captured-to-today-s-journal"), "info");
      }
      return ok;
    };
    const promise = save();
    if (id) inFlight.set(id, promise);
    const ok = await promise;
    if (id) {
      inFlight.delete(id);
      if (ok) { landed.delete(id); rememberCompleted(id, ok); }
    }
    ack(id, ok);
  }), (unlisten) => unlisten());
  return registration.kind === "current" ? registration.value : () => {};
}

