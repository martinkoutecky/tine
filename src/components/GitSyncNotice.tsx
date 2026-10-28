import { For, Show, createEffect, createSignal, onCleanup, untrack, type JSX } from "solid-js";
import { backend } from "../backend";
import type { GitSyncStatus } from "../backendTypes";
import { graphEpoch, graphMeta } from "../graphSession";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { writeClipboardText } from "../clipboard";
import { canRevealInFileManager } from "../nativeChrome";
import { t } from "../i18n";
import { openSettings } from "../ui";
import { gitSyncProblem, type GitSyncProblem } from "../gitSync";

/** How often the notice reads the worker's last sync (a small in-memory status; no repository access), in
 *  seconds; every second while a sync it started runs. */
const POLL_SECONDS = 15;

/** The ways to fix a sync problem (D-20261027-OF), shared by Settings → Graph → Git and the app's notice. */
export function GitSyncFixes(props: { problem: GitSyncProblem; onSignIn: () => void; onSyncNow: () => void }): JSX.Element {
  const [error, setError] = createSignal("");
  const copy = async (command: string) => {
    try { await writeClipboardText(command); setError(""); }
    catch { setError(t("git-sync-copy-failed")); }
  };
  const reveal = async () => {
    const root = graphMeta()?.root;
    if (!root) return;
    try { await readOwned(graphOwner(), backend().revealKnownGraph(root)); setError(""); }
    catch (failure) { setError(t("git-sync-open-folder-failed", { error: String(failure) })); }
  };
  return <span class="git-sync-fixes">
    <Show when={props.problem.command}>{(command) => <code data-git-sync-command>{command()}</code>}</Show>
    <For each={props.problem.fixes}>{(fix) => {
      switch (fix) {
        case "sync-now": return <button class="settings-btn" data-git-sync-fix={fix} onClick={() => props.onSyncNow()}>{t("git-settings-sync-now")}</button>;
        case "sign-in": return <button class="settings-btn" data-git-sync-fix={fix} onClick={() => props.onSignIn()}>{t("git-sync-fix-sign-in")}</button>;
        case "copy-command": return <Show when={props.problem.command}>{(command) =>
          <button class="settings-btn" data-git-sync-fix={fix} onClick={() => void copy(command())}>{t("git-sync-fix-copy-command")}</button>}</Show>;
        case "open-folder": return <Show when={canRevealInFileManager()}>
          <button class="settings-btn" data-git-sync-fix={fix} onClick={() => void reveal()}>{t("git-sync-fix-open-folder")}</button></Show>;
      }
    }}</For>
    <Show when={error()}><span role="alert">{error()}</span></Show>
  </span>;
}

/** A sync problem that persists (ADR 0078 §4: one that needs the user at once, or one that kept failing), on the
 *  app's persistent graph-notice surface. No dismissal: it clears when a sync succeeds or sync is turned off. */
export function GitSyncNotice(): JSX.Element {
  const [sync, setSync] = createSignal<GitSyncStatus | null>(null);
  createEffect(() => {
    graphEpoch();
    const root = graphMeta()?.root;
    let alive = true;
    const owner = bindingOwner(() => alive);
    setSync(null);
    if (!root) return;
    const refresh = async () => {
      try {
        const answer = await readOwned(owner, backend().gitSyncNotice());
        if (answer.kind === "current") setSync(answer.value);
      } catch {
        // Keep what is shown across an IPC failure.
      }
    };
    void untrack(refresh);
    let ticks = 0;
    const timer = setInterval(() => {
      ticks++;
      if (untrack(sync)?.running || ticks % POLL_SECONDS === 0) void refresh();
    }, 1000);
    onCleanup(() => { alive = false; clearInterval(timer); });
  });
  const syncNow = async () => {
    const owner = bindingOwner();
    try {
      const answer = await writeOwned(owner, backend().gitSyncNow());
      if (answer.kind === "current") setSync(answer.value.last?.sync?.persistent ? answer.value.last.sync : null);
    } catch {
      openSettings("graph");
    }
  };
  const problem = () => gitSyncProblem(sync(), graphMeta()?.root);
  return <Show when={sync()?.persistent && problem()}>{(p) =>
    <div class="root-recovery-notice" role="status" data-testid="git-sync-notice" data-git-sync-code={sync()?.code ?? ""}>
      <strong>{t("git-sync-notice-title")}</strong>
      <span>{p().text}</span>
      <GitSyncFixes problem={p()} onSignIn={() => openSettings("graph")} onSyncNow={() => void syncNow()} />
      <button class="settings-btn" data-git-sync-settings onClick={() => openSettings("graph")}>{t("git-sync-notice-settings")}</button>
    </div>}</Show>;
}
