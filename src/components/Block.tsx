import { Show, Switch, Match, For, createMemo, onMount, type JSX } from "solid-js";
import {
  page,
  editingId,
  startEditing,
  setRaw,
  setEditingId,
  splitBlock,
  indentBlock,
  outdentBlock,
  mergeWithPrev,
  toggleCollapse,
  takeCaretFor,
  prevVisible,
  nextVisible,
} from "../store";
import { blockView } from "../render/block";
import { InlineText } from "../render/inline";
import { QueryMacro, EmbedMacro } from "./Macro";

// Detect a block whose entire body is a single {{query}} / {{embed}} macro.
function detectMacro(lines: string[]): { kind: "query" | "embed"; inner: string } | null {
  const text = lines.join("\n").trim();
  const m = /^\{\{(query|embed)\b([\s\S]*)\}\}$/.exec(text);
  if (!m) return null;
  return { kind: m[1] as "query" | "embed", inner: `${m[1]}${m[2]}` };
}

export function Block(props: { id: string }): JSX.Element {
  const node = () => page.byId[props.id];
  const editing = () => editingId() === props.id;
  const hasChildren = () => node().children.length > 0;
  const collapsed = () => node().collapsed;

  return (
    <div class="ls-block" classList={{ collapsed: collapsed() }} data-block-id={props.id}>
      <div class="block-main">
        <div class="block-controls">
          <span
            class="collapse-toggle"
            classList={{ "has-children": hasChildren() }}
            onClick={() => toggleCollapse(props.id)}
          >
            <Show when={hasChildren()}>
              <svg viewBox="0 0 24 24" class="triangle">
                <path d="M8 5l8 7-8 7z" />
              </svg>
            </Show>
          </span>
          <span
            class="bullet-container"
            classList={{ "bullet-closed": collapsed() && hasChildren() }}
          >
            <span class="bullet" />
          </span>
        </div>

        <div class="block-content-wrapper">
          <Show when={editing()} fallback={<Rendered id={props.id} />}>
            <Editor id={props.id} />
          </Show>
        </div>
      </div>

      <Show when={hasChildren() && !collapsed()}>
        <div class="block-children-container">
          <div class="block-children-left-border" />
          <div class="block-children">
            <For each={node().children}>{(cid) => <Block id={cid} />}</For>
          </div>
        </div>
      </Show>
    </div>
  );
}

function Rendered(props: { id: string }): JSX.Element {
  const node = () => page.byId[props.id];
  const view = createMemo(() => blockView(node().raw));

  const onClick = (e: MouseEvent) => {
    // Links/tags handle their own clicks (stopPropagation). Otherwise edit.
    startEditing(props.id, node().raw.length);
    e.stopPropagation();
  };

  const macro = createMemo(() => detectMacro(view().lines));

  const body = (
    <For each={view().lines}>
      {(line, i) => (
        <>
          <Show when={i() > 0}>
            <br />
          </Show>
          <InlineText text={line} />
        </>
      )}
    </For>
  );

  return (
    <Show
      when={!macro()}
      fallback={
        <div class="block-content macro-host" onClick={onClick}>
          <Switch>
            <Match when={macro()!.kind === "query"}>
              <QueryMacro body={macro()!.inner} />
            </Match>
            <Match when={macro()!.kind === "embed"}>
              <EmbedMacro body={macro()!.inner} />
            </Match>
          </Switch>
        </div>
      }
    >
    <div
      class="block-content"
      classList={{ done: view().done, [`heading h${view().headingLevel ?? ""}`]: view().headingLevel != null }}
      onClick={onClick}
    >
      <Show when={view().marker}>
        <span
          class={`block-marker marker-${view().marker?.toLowerCase()}`}
          onClick={(e) => {
            e.stopPropagation();
            cycleMarker(props.id);
          }}
        >
          {view().marker}
        </span>{" "}
      </Show>
      <Show when={view().headingLevel} fallback={body}>
        {(() => {
          const H = `h${view().headingLevel}`;
          return <span class={`heading-text ${H}`}>{body}</span>;
        })()}
      </Show>
      <Show when={view().properties.length > 0}>
        <span class="block-properties">
          <For each={view().properties}>
            {([k, v]) => (
              <span class="prop">
                <span class="prop-key">{k}</span>
                <span class="prop-value"> {v}</span>
              </span>
            )}
          </For>
        </span>
      </Show>
    </div>
    </Show>
  );
}

function cycleMarker(id: string) {
  // TODO -> DOING -> DONE -> (none), and NOW/LATER similarly. Minimal cycle.
  const raw = page.byId[id].raw;
  const order: Record<string, string> = {
    TODO: "DOING",
    DOING: "DONE",
    DONE: "",
    NOW: "LATER",
    LATER: "DONE",
  };
  for (const m of Object.keys(order)) {
    if (raw === m || raw.startsWith(m + " ")) {
      const rest = raw.slice(m.length).replace(/^ /, "");
      const next = order[m];
      setRaw(id, next ? `${next} ${rest}` : rest);
      return;
    }
  }
}

function Editor(props: { id: string }): JSX.Element {
  let ref!: HTMLTextAreaElement;
  const node = () => page.byId[props.id];

  const autosize = () => {
    ref.style.height = "auto";
    ref.style.height = `${ref.scrollHeight}px`;
  };

  onMount(() => {
    const offset = takeCaretFor(props.id) ?? node().raw.length;
    ref.focus();
    const o = Math.min(offset, ref.value.length);
    ref.setSelectionRange(o, o);
    autosize();
  });

  const onInput = () => {
    setRaw(props.id, ref.value);
    autosize();
  };

  const onKeyDown = (e: KeyboardEvent) => {
    const start = ref.selectionStart;
    const end = ref.selectionEnd;
    const raw = ref.value;

    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      setRaw(props.id, raw); // flush current text
      splitBlock(props.id, start);
    } else if (e.key === "Tab" && !e.shiftKey) {
      e.preventDefault();
      setRaw(props.id, raw);
      indentBlock(props.id, start);
    } else if (e.key === "Tab" && e.shiftKey) {
      e.preventDefault();
      setRaw(props.id, raw);
      outdentBlock(props.id, start);
    } else if (e.key === "Backspace" && start === 0 && end === 0) {
      setRaw(props.id, raw);
      if (mergeWithPrev(props.id)) e.preventDefault();
    } else if (e.key === "ArrowUp") {
      const before = raw.slice(0, start);
      if (!before.includes("\n")) {
        const prev = prevVisible(props.id);
        if (prev) {
          e.preventDefault();
          startEditing(prev, page.byId[prev].raw.length);
        }
      }
    } else if (e.key === "ArrowDown") {
      const after = raw.slice(start);
      if (!after.includes("\n")) {
        const next = nextVisible(props.id);
        if (next) {
          e.preventDefault();
          startEditing(next, 0);
        }
      }
    } else if (e.key === "Escape") {
      ref.blur();
    }
  };

  const onBlur = () => {
    setRaw(props.id, ref.value);
    // Only clear if no other block grabbed editing focus.
    if (editingId() === props.id) setEditingId(null);
  };

  return (
    <textarea
      ref={ref}
      class="block-editor"
      spellcheck={false}
      value={node().raw}
      onInput={onInput}
      onKeyDown={onKeyDown}
      onBlur={onBlur}
      rows={1}
    />
  );
}
