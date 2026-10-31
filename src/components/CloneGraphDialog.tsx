import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { backend } from "../backend";
import { chooseNewGraphParent, loadGraphPath } from "../graph";
import { bindingOwner, readOwned, writeOwned } from "../owned";
import { registerTransientLayer } from "../transientLayers";
import { t } from "../i18n";
import { DialogActions } from "./DialogActions";

/** One explicit clone, then open. A failed open retains the completed clone
 * so Retry never clones over it. Sign-in uses sync's existing token store. */
export function CloneGraphDialog(props: { onClose: () => void; onOpened?: () => void }) {
  let alive = true;
  onCleanup(() => { alive = false; });
  const owner = bindingOwner(() => alive);
  const [url, setUrl] = createSignal("");
  const [parent, setParent] = createSignal<string | null>(null);
  const [cloned, setCloned] = createSignal<string | null>(null);
  const [auth, setAuth] = createSignal(false);
  const [busy, setBusy] = createSignal(false);
  const [error, setError] = createSignal("");
  let root!: HTMLFormElement, input!: HTMLInputElement, username!: HTMLInputElement, token!: HTMLInputElement;
  const close = () => { if (!busy()) props.onClose(); };
  onMount(() => {
    input.focus();
    onCleanup(registerTransientLayer({ id: "git-clone", root: () => root, dismiss: () => { close(); return true; } }));
  });
  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (busy() || !owner()) return;
    setBusy(true); setError("");
    try {
      if (!cloned()) {
        const repository = url().trim();
        const parsed = new URL(repository);
        if (parsed.protocol !== "https:" || parsed.username || parsed.password) throw new Error(t("git-setup-https"));
        if (auth()) {
          const signing = writeOwned(owner, backend().gitSetupSignIn(repository, username.value, token.value));
          token.value = "";
          if ((await signing).kind === "stale") return;
          setAuth(false);
        }
        let dir = parent();
        if (!dir) {
          const picked = await readOwned(owner, chooseNewGraphParent(true));
          if (picked.kind === "stale" || picked.value.kind === "aborted") return;
          dir = picked.value.dir; setParent(dir);
        }
        const answer = await writeOwned(owner, backend().gitCloneGraph(dir, repository));
        if (answer.kind === "stale") return;
        if (answer.value.kind === "auth-required") { setAuth(true); setError(t("git-sync-auth-required")); return; }
        setCloned(answer.value.root);
      }
      const loaded = await loadGraphPath(cloned()!);
      if (loaded.kind === "aborted") { setError(t("git-clone-open-failed", { path: cloned()! })); return; }
      props.onOpened?.(); props.onClose();
    } catch (failure) {
      if (!owner()) return;
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally { if (alive) setBusy(false); }
  };
  return <div class="modal-overlay" onClick={close}>
    <form ref={root} class="export-modal" role="dialog" aria-modal="true" aria-label={t("git-clone-label")}
      onClick={e => e.stopPropagation()} onSubmit={submit}>
      <div class="export-head">{t("git-clone-label")}</div>
      <p>{t("git-setup-off")}</p>
      <label>{t("git-setup-url")}<input ref={input} class="settings-input" type="url" required value={url()}
        disabled={busy() || !!cloned()} onInput={e => { setUrl(e.currentTarget.value); setAuth(false); setError(""); }} /></label>
      <Show when={parent()}>{dir => <p>{dir()} <button type="button" class="settings-btn" disabled={busy() || !!cloned()}
        onClick={() => setParent(null)}>{t("git-clone-change-location")}</button></p>}</Show>
      <Show when={cloned()}>{path => <p>{path()}</p>}</Show>
      <Show when={auth() && !cloned()}>
        <p>{t("git-settings-sign-in-hint")}</p>
        <input ref={username} class="settings-input" required autocomplete="username" aria-label={t("git-settings-username")} placeholder={t("git-settings-username")} disabled={busy()} />
        <input ref={token} class="settings-input" required type="password" autocomplete="off" aria-label={t("git-settings-token")} placeholder={t("git-settings-token")} disabled={busy()} />
      </Show>
      <Show when={error()}>{message => <p role="alert">{message()}</p>}</Show>
      <Show when={busy()}><p role="status">{t("git-setup-working")}</p></Show>
      <DialogActions cancel={<button type="button" class="export-btn-secondary" disabled={busy()} onClick={close}>{t("git-settings-sign-in-cancel")}</button>}
        primary={<button type="submit" class="export-btn-primary" disabled={busy()}>{cloned() ? t("git-clone-open") : auth() ? t("git-clone-sign-in") : t("git-clone-label")}</button>} />
    </form>
  </div>;
}
