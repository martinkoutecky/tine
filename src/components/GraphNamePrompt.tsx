import { Show, createSignal, onCleanup, onMount } from "solid-js";
import { graphNameRequest, type GraphNameRequest } from "../graphNamePrompt";
import { registerTransientLayer } from "../transientLayers";
import { refuseStaleWrite } from "../binding";

export function GraphNamePrompt() {
  return <Show when={graphNameRequest()}>{request => <NameForm request={request()} />}</Show>;
}

function NameForm(props: { request: GraphNameRequest }) {
  const request = props.request;
  onCleanup(() => request.finish(null));
  const [name, setName] = createSignal(request.suggestion);
  const [error, setError] = createSignal("");
  const [busy, setBusy] = createSignal(false);
  let root!: HTMLFormElement;
  let input!: HTMLInputElement;
  const cancel = () => { if (!busy()) request.finish(null); };
  onMount(() => {
    input.focus(); input.select();
    onCleanup(registerTransientLayer({ id: "graph-name", root: () => root,
      dismiss: () => { cancel(); return true; } }));
  });
  const submit = async (event: SubmitEvent) => {
    event.preventDefault();
    if (busy()) return;
    if (graphNameRequest() !== request) return refuseStaleWrite("Graph creation");
    setBusy(true); setError("");
    try { request.finish(await request.create(name())); }
    catch (failure) {
      setError(failure instanceof Error ? failure.message : String(failure));
    } finally { setBusy(false); if (error()) input.focus(); }
  };
  return <div class="modal-overlay graph-name-overlay" onClick={cancel}>
    <form ref={root} class="export-modal graph-name-prompt" role="dialog" aria-modal="true"
      aria-label="Create a new graph" onClick={event => event.stopPropagation()} onSubmit={submit}>
      <div class="export-head">Create a new graph</div>
      <label class="graph-name-field">Graph name
        <input ref={input} value={name()} disabled={busy()} aria-invalid={!!error()}
          aria-describedby={error() ? "graph-name-error" : undefined}
          onInput={event => { setName(event.currentTarget.value); setError(""); }} />
      </label>
      <Show when={error()}><p id="graph-name-error" class="graph-name-error" role="alert">{error()}</p></Show>
      <div class="export-foot">
        <button type="button" class="export-btn-secondary" disabled={busy()} onClick={cancel}>Cancel</button>
        <button type="submit" class="export-btn-primary" disabled={busy()}>{busy() ? "Creating…" : "Create graph"}</button>
      </div>
    </form>
  </div>;
}
