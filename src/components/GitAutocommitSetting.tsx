import { For, Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { t } from "../i18n";
import { backend, type GitAutocommitStatus, type GitPolicy } from "../backend";
import { graphMeta } from "../graphSession";
import { bindingOwner, graphOwner, readOwned, writeOwned } from "../owned";
import { Field } from "./settingsField";
import { DialogActions } from "./DialogActions";
import { GitSyncFixes } from "./GitSyncNotice";
import { gitLineEndingsProblem, gitStaleLockProblem, gitSyncProblem, gitSyncResult } from "../gitSync";
import { ignoreLines } from "../gitIgnoreNotice";

const SYNCS: readonly GitPolicy[] = ["sync-on-demand", "sync-automatically"];

/** Settings → Graph → Git (ADR 0078 slices 1–2): whether Tine commits this graph's changes to the git repository it
 *  sits in, how often, and whether it syncs them with the branch's upstream. The setting is per device and per graph
 *  (`tine-settings.json`); the backend applies it to the running worker at once. Shows the branch, the remote (read
 *  from the repository config), why Tine can't commit or sync, the last commit and sync, each sync
 *  problem with a way to fix it, an `index.lock` left held for a long time (a stale one is offered for removal on
 *  the person's click, GS-LOCK; Tine never removes a lock unasked), and the sign-in
 *  for an HTTPS remote (the token goes to a private file on this device). */
export function GitAutocommitSetting(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const [status, setStatus] = createSignal<GitAutocommitStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
  const [signingIn, setSigningIn] = createSignal(false);
  const message = (failure: unknown) => failure instanceof Error ? failure.message : String(failure);

  const load = async () => {
    const owner = graphOwner(() => alive);
    try {
      const answer = await readOwned(owner, backend().gitAutocommitStatus());
      if (answer.kind === "current") { setStatus(answer.value); setError(null); }
    } catch (failure) {
      if (owner()) setError(t("git-settings-load-failed", { error: message(failure) }));
    }
  };
  onMount(() => {
    void load();
    // The last commit, a sync and a long-held lock change while Settings is open; a running sync is followed closely.
    let ticks = 0;
    const timer = setInterval(() => {
      ticks++;
      if (status()?.last?.sync?.running || ticks % 15 === 0) void load();
    }, 1000);
    onCleanup(() => clearInterval(timer));
  });

  /** A write whose answer is the new status; a failure shows the device's actual state and the error. */
  const write = async (call: () => Promise<GitAutocommitStatus>, failed: (error: string) => string, done?: () => void) => {
    const owner = bindingOwner(() => alive);
    try {
      const answer = await writeOwned(owner, call());
      if (answer.kind === "current") { setStatus(answer.value); setError(null); done?.(); }
    } catch (failure) {
      if (!owner()) return;
      // The control shows what the device has, not the refused choice.
      await load();
      // A fresh object even when nothing changed: the select is reset too.
      const now = status();
      if (now) setStatus({ ...now });
      setError(failed(message(failure)));
    }
  };
  const change = (policy: GitPolicy, minutes: number) =>
    void write(() => backend().setGitAutocommit(policy, minutes), (e) => t("git-settings-save-failed", { error: e }));
  const syncNow = () => void write(() => backend().gitSyncNow(), (e) => t("git-settings-sync-now-failed", { error: e }));

  let username!: HTMLInputElement, token!: HTMLInputElement;
  const signIn = () => {
    void write(() => backend().gitSignIn(username.value, token.value), (e) => t("git-settings-sign-in-failed", { error: e }), () => setSigningIn(false));
    // The token leaves the page as soon as it was handed over (the call above took it synchronously).
    token.value = "";
  };
  const signOut = () => void write(() => backend().gitSignOut(), (e) => t("git-settings-sign-out-failed", { error: e }));

  const at = (seconds: number) => new Date(seconds * 1000).toLocaleTimeString();
  const syncing = () => SYNCS.includes(status()?.policy ?? "off");
  const problem = () => syncing() ? gitSyncProblem(status()?.last?.sync, graphMeta()?.root) : null;
  const conversionProblem = () => gitLineEndingsProblem(status()?.line_endings);
  return <Field label={t("git-settings-label")} hint={<>
    <Show when={status()?.available !== false}>{t("git-settings-hint")}</Show>
    <Show when={status()}>{(s) => <>
      <Show when={!s().available}><div>{t("git-settings-not-available")}</div></Show>
      <Show when={s().available && !s().repository}><div>{t("git-settings-not-a-repository")}</div></Show>
      <Show when={s().branch}>{(branch) => <div>{t("git-settings-branch", { branch: branch() })}</div>}</Show>
      <Show when={s().remote}>{(remote) => <div data-git-remote={remote().name}>{t("git-settings-remote", { name: remote().name, url: remote().url })}</div>}</Show>
      <Show when={s().unusable}>{(why) => <div data-git-unusable={why()[0]}>{t("git-settings-unusable", { message: why()[1] })}</div>}</Show>
      <Show when={!s().unusable && s().repository && s().sync_unusable}>{(why) => <div data-git-sync-unusable={why()[0]}>{t("git-settings-sync-unusable", { message: why()[1] })}</div>}</Show>
      <Show when={conversionProblem()}>{(p) => <div role="status" data-git-line-endings>
        {p().text}
        <GitSyncFixes problem={{ ...p(), fixes: p().fixes.filter((fix) => fix !== "sync-now") }} onSignIn={() => setSigningIn(true)} onSyncNow={syncNow} onRepaired={(value) => setStatus(value)} />
      </div>}</Show>
      <Show when={s().last?.commit && s().last?.committed_at}>
        <div data-git-last-commit={s().last!.commit!}>{t("git-settings-last-commit", { commit: s().last!.commit!.slice(0, 8), time: at(s().last!.committed_at!) })}</div>
      </Show>
      <Show when={!s().unusable && s().last?.refused}>{(why) => <div>{t("git-settings-last-refused", { message: why()[1] })}</div>}</Show>
      <Show when={syncing()}>
        <div class="git-sync-fixes">
          <Show when={gitSyncResult(s().last?.sync)}>{(line) => <span data-git-sync-result={s().last?.sync?.running ? "running" : s().last?.sync?.code ?? ""}>{line()}</span>}</Show>
          <button class="settings-btn" data-git-sync-now disabled={!!s().last?.sync?.running} onClick={syncNow}>{t("git-settings-sync-now")}</button>
        </div>
      </Show>
      <Show when={!conversionProblem() && problem()}>{(p) => <div role="status" data-git-sync-problem={s().last?.sync?.code ?? ""}>
        {p().text}
        {/* Sync now has its own button above. */}
        <GitSyncFixes problem={{ ...p(), fixes: p().fixes.filter((fix) => fix !== "sync-now") }} onSignIn={() => setSigningIn(true)} onSyncNow={syncNow} onRepaired={(value) => setStatus(value)} />
      </div>}</Show>
      <Show when={s().last?.stale_lock}>{(path) => <div>{t("git-settings-stale-lock", { path: path() })}</div>}</Show>
      <Show when={gitStaleLockProblem(s().last?.stale_lock_offer)}>{(p) => <div role="status" data-git-stale-lock>
        {p().text}
        <GitSyncFixes problem={p()} onSignIn={() => setSigningIn(true)} onSyncNow={syncNow} onRepaired={(value) => setStatus(value)} />
      </div>}</Show>
      <Show when={s().last?.gitignore_added}>{(path) => <div role="status">{t("git-settings-ignore-added", { path: path(), which: ignoreLines(s().last) })}</div>}</Show>
      <Show when={s().last?.gitignore_failed}><div role="status">{t("git-settings-ignore-failed", { which: ignoreLines(s().last) })}</div></Show>
      <Show when={s().last?.repack_deferred}><div role="status">{t("git-settings-repack-deferred")}</div></Show>
      <Show when={s().remote?.https_origin}>{(origin) => <div data-git-sign-in>
        <Show when={s().signed_in} fallback={<>{t("git-settings-not-signed-in", { origin: origin() })}{" "}
          <Show when={!signingIn()}><button class="settings-btn" data-git-sign-in-open onClick={() => setSigningIn(true)}>{t("git-settings-sign-in")}</button></Show>
        </>}>{(user) => <>
          {t("git-settings-signed-in", { origin: origin(), username: user() })}{" "}
          <button class="settings-btn" data-git-sign-out onClick={signOut}>{t("git-settings-sign-out")}</button>{" "}
          <Show when={!signingIn()}><button class="settings-btn" data-git-sign-in-open onClick={() => setSigningIn(true)}>{t("git-settings-sign-in")}</button></Show>
        </>}</Show>
        <Show when={signingIn()}>
          <form data-git-sign-in-form onSubmit={(e) => { e.preventDefault(); signIn(); }}>
            <div>{t("git-settings-sign-in-hint")}</div>
            <input class="settings-input" ref={username} autocomplete="username" aria-label={t("git-settings-username")} placeholder={t("git-settings-username")} value={s().signed_in ?? ""} data-git-username />
            <input class="settings-input" ref={token} type="password" autocomplete="off" aria-label={t("git-settings-token")} placeholder={t("git-settings-token")} data-git-token />
            <DialogActions
              cancel={<button class="settings-btn" type="button" onClick={() => { token.value = ""; setSigningIn(false); }}>{t("git-settings-sign-in-cancel")}</button>}
              primary={<button class="settings-btn" type="submit">{t("git-settings-sign-in-save")}</button>} />
          </form>
        </Show>
      </div>}</Show>
    </>}</Show>
    <Show when={error()}>{(e) => <div role="alert">{e()}</div>}</Show>
  </>}>
    <Show when={status()?.available && status()?.repository}>
      <select class="settings-select" data-git-policy value={status()!.policy ?? "off"}
        onChange={(e) => change(e.currentTarget.value as GitPolicy, status()!.interval_minutes ?? 10)}>
        <option value="off">{t("git-settings-policy-off")}</option>
        <option value="commit">{t("git-settings-policy-commit")}</option>
        <For each={SYNCS}>{(policy) =>
          // Refused with the repository's reason (shown below); a device already set to it keeps showing it.
          <option value={policy} disabled={!!status()!.sync_unusable && status()!.policy !== policy}>
            {t(policy === "sync-on-demand" ? "git-settings-policy-sync-on-demand" : "git-settings-policy-sync-automatically")}
          </option>}</For>
      </select>
      <Show when={status()!.policy && status()!.policy !== "off"}>
        {t("git-settings-interval")}
        <input class="settings-input" type="number" min="1" max="1440" data-git-interval
          aria-label={t("git-settings-interval-aria")} value={status()!.interval_minutes ?? 10}
          onChange={(e) => {
            const minutes = Number(e.currentTarget.value);
            if (Number.isInteger(minutes) && minutes >= 1) change(status()!.policy!, Math.min(minutes, 1440));
            else e.currentTarget.value = String(status()!.interval_minutes ?? 10);
          }} />
        {t("git-settings-minutes")}
      </Show>
    </Show>
  </Field>;
}
