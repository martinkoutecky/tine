import { Show, createEffect, createSignal, onCleanup, untrack } from "solid-js";
import { backend } from "../backend";
import { graphMeta, graphEpoch } from "../graphSession";
import { bindingOwner, readOwned, writeOwned } from "../owned";
import { writeClipboardText } from "../clipboard";
import { t } from "../i18n";
import type { OwnerProtectionIssue } from "../ownerProtection";
import { bindingIdentity } from "../binding";

/** Same persistent surface as graph recovery. No dismissal or saved preference:
 * only a native acquired lease clears the warning. Retries run natively even
 * when this webview is suspended. Polling reads a small binding-local status. */
export function OwnerProtectionNotice() {
  const [issue, setIssue] = createSignal<OwnerProtectionIssue | null>(null);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let revision = 0;
  let heldBinding = "", heldRoot: string | undefined;
  createEffect(() => {
    graphEpoch();
    const root = graphMeta()?.root;
    const read = backend().ownerProtectionStatus;
    let alive = true, reading = false;
    const owner = bindingOwner(() => alive);
    // Display epochs may change while the same graph remains unprotected.
    // Preserve the warning during the replacement status read.
    const binding = bindingIdentity();
    if (!root || binding !== heldBinding || root !== heldRoot) {
      setIssue(null); setError(""); setBusy(false);
    }
    heldBinding = binding; heldRoot = root;
    if (!root || !read) return;
    let timer: ReturnType<typeof setInterval> | undefined;
    const refresh = async () => {
      if (reading || busy()) return;
      reading = true;
      const observed = revision;
      try {
        const result = await readOwned(owner, read.call(backend()));
        if (result.kind === "current" && observed === revision) {
          setIssue(result.value);
          // A live lease lasts with its host. Poll only the fallback; a new
          // binding restarts this effect and reads its own native status.
          if (result.value === null) clearInterval(timer);
        }
      } catch {
        // Preserve an existing warning across an IPC failure.
        if (owner()) setError(t("owner-protection-status-failed"));
      } finally { reading = false; }
    };
    void untrack(refresh);
    timer = setInterval(() => void refresh(), 1000);
    onCleanup(() => { alive = false; clearInterval(timer); });
  });
  const retry = async (repair: boolean) => {
    const action = backend().retryOwnerProtection;
    if (!action || busy()) return;
    const owner = bindingOwner();
    const current = issue();
    revision++;
    setBusy(true); setError("");
    try {
      const result = await writeOwned(owner, action.call(backend(), repair));
      if (result.kind === "current" && issue() === current) setIssue(result.value);
    } catch {
      if (owner()) setError(t("owner-protection-retry-failed"));
    } finally { if (owner()) setBusy(false); }
  };
  const copy = async (value: string) => {
    const owner = bindingOwner();
    try { await writeClipboardText(value); }
    catch { if (owner()) setError(t("owner-protection-copy-failed")); }
  };
  return <Show when={issue()}>{(problem) => <div class="root-recovery-notice owner-protection-notice" role="status" data-testid="owner-protection-notice">
    <strong class="owner-protection-title">{t("owner-protection-title")}</strong>
    <Show when={problem().repairable}><button class="settings-btn" disabled={busy()} onClick={() => void retry(true)}>{t("owner-protection-repair")}</button></Show>
    <button class="settings-btn" disabled={busy()} onClick={() => void retry(false)}>{t("owner-protection-retry")}</button>
    <details>
      <summary>{t("owner-protection-details")}</summary>
      <div class="owner-protection-explanation">
        <p>{t("owner-protection-editable")}</p>
        <p>{t(`owner-protection-reason-${problem().reason}`)}</p>
        <p>{t(`owner-protection-fix-${problem().instructions}`)}</p>
        <Show when={problem().command}><p><code>{problem().command}</code></p>
          <button class="settings-btn" onClick={() => void copy(problem().command)}>{t("owner-protection-copy-command")}</button>
        </Show>
        <button class="settings-btn" onClick={() => void copy(problem().details)}>{t("owner-protection-copy-details")}</button>
        <pre>{problem().details}</pre>
      </div>
    </details>
    <Show when={error()}><p class="owner-protection-error" role="alert">{error()}</p></Show>
  </div>}</Show>;
}
