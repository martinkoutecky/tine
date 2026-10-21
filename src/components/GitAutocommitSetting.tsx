import { Show, createSignal, onCleanup, onMount, type JSX } from "solid-js";
import { t } from "../i18n";
import { backend, type GitAutocommitStatus, type GitPolicy } from "../backend";
import { graphOwner, readOwned, writeOwned } from "../owned";
import { Field } from "./settingsField";

/** Settings → Graph → Git (ADR 0078 slice 1): whether Tine commits this graph's changes to the git repository it
 *  sits in, and how often. The setting is per device and per graph (`tine-settings.json`); the backend applies it to
 *  the running worker at once. Shows the branch, why Tine can't commit, the last commit or refusal, and an
 *  `index.lock` left held for a long time (Tine never removes it). */
export function GitAutocommitSetting(): JSX.Element {
  let alive = true;
  onCleanup(() => { alive = false; });
  const [status, setStatus] = createSignal<GitAutocommitStatus | null>(null);
  const [error, setError] = createSignal<string | null>(null);
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
  onMount(() => void load());

  const change = async (policy: GitPolicy, minutes: number) => {
    const owner = graphOwner(() => alive);
    try {
      const answer = await writeOwned(owner, backend().setGitAutocommit(policy, minutes));
      if (answer.kind === "current") { setStatus(answer.value); setError(null); }
    } catch (failure) {
      if (owner()) setError(t("git-settings-save-failed", { error: message(failure) }));
    }
  };

  const at = (seconds: number) => new Date(seconds * 1000).toLocaleTimeString();
  return <Field label={t("git-settings-label")} hint={<>
    <Show when={status()?.available !== false}>{t("git-settings-hint")}</Show>
    <Show when={status()}>{(s) => <>
      <Show when={!s().available}><div>{t("git-settings-not-available")}</div></Show>
      <Show when={s().available && !s().repository}><div>{t("git-settings-not-a-repository")}</div></Show>
      <Show when={s().branch}>{(branch) => <div>{t("git-settings-branch", { branch: branch() })}</div>}</Show>
      <Show when={s().unusable}>{(why) => <div data-git-unusable={why()[0]}>{t("git-settings-unusable", { message: why()[1] })}</div>}</Show>
      <Show when={s().last?.commit && s().last?.committed_at}>
        <div data-git-last-commit={s().last!.commit!}>{t("git-settings-last-commit", { commit: s().last!.commit!.slice(0, 8), time: at(s().last!.committed_at!) })}</div>
      </Show>
      <Show when={!s().unusable && s().last?.refused}>{(why) => <div>{t("git-settings-last-refused", { message: why()[1] })}</div>}</Show>
      <Show when={s().last?.stale_lock}>{(path) => <div>{t("git-settings-stale-lock", { path: path() })}</div>}</Show>
    </>}</Show>
    <Show when={error()}>{(e) => <div role="alert">{e()}</div>}</Show>
  </>}>
    <Show when={status()?.available && status()?.repository}>
      <select class="settings-select" data-git-policy value={status()!.policy ?? "off"}
        onChange={(e) => void change(e.currentTarget.value as GitPolicy, status()!.interval_minutes ?? 10)}>
        <option value="off">{t("git-settings-policy-off")}</option>
        <option value="commit">{t("git-settings-policy-commit")}</option>
      </select>
      <Show when={status()!.policy === "commit"}>
        {t("git-settings-interval")}
        <input class="settings-input" type="number" min="1" max="1440" data-git-interval
          aria-label={t("git-settings-interval-aria")} value={status()!.interval_minutes ?? 10}
          onChange={(e) => {
            const minutes = Number(e.currentTarget.value);
            if (Number.isInteger(minutes) && minutes >= 1) void change("commit", Math.min(minutes, 1440));
          }} />
        {t("git-settings-minutes")}
      </Show>
    </Show>
  </Field>;
}
