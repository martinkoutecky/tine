import { Show, Switch, Match, For, type JSX } from "solid-js";
import { pageByName, blockPageReadOnly, blockExternalId, type OutlineScope, type Node as ReadonlyNode } from "../document";
import { isRenderHiddenProp, isPropertyLine, propertyKeyNorm } from "../render/block";
import type { Facets } from "../render/facets";
import { AstBody } from "../render/body";
import { InlineText } from "../render/inline";
import { DeferredStandaloneMacro } from "./DeferredStandaloneMacro";
import { QueryMacro, EmbedMacro } from "./Macro";
import { timetrackingEnabled, openDatePicker } from "../ui";
import { graphMeta } from "../graphSession";
import { taskCheckboxState } from "../markers";
import { markerLabelClickable } from "../editor/repeat";
import { logbookInfo, type LogbookInfo } from "../logbook";
import { annotationInfo } from "../editor/annotation";
import { AnnotationBody } from "./AnnotationBody";
import { forbidsEditEntry } from "../editor/editTargets";
import { blockBackgroundColor } from "../blockColors";
import { queryMacroExtents, singleQueryMacroExtent, type MacroExtent } from "../editor/queryMacro";
import { beginEditGesture, renderedClickOffset } from "./blockGestures";
import { CalGlyph, ClockBadge, toggleBlockMarkerLabel, toggleBlockCheckbox } from "./blockParts";

// The rendered (non-editing) body of one block: header facets, body, chips and
// properties. Extracted from Block.tsx (og-F) with no behaviour change.

// Detect a block whose entire body is a single {{query}}/{{tine-query}}/{{embed}} macro.
export function detectMacro(raw: string): { kind: "query" | "embed"; inner: string; sourceExtent?: MacroExtent } | null {
  // The visible body: property lines stripped so `{{query}}\nid:: …` still matches.
  const text = raw.split("\n").filter((l) => !isPropertyLine(l)).join("\n").trim();
  const [q, ...rest] = queryMacroExtents(text); // shared reader: a second macro or a `}}` in a string never merges
  if (q && !rest.length && q.start === 0 && q.end === text.length) return { kind: "query", inner: `${q.name} ${q.argument}`, sourceExtent: singleQueryMacroExtent(raw, q) };
  const m = /^\{\{(embed)\b([\s\S]*)\}\}$/.exec(text);
  return m ? { kind: "embed", inner: `${m[1]}${m[2]}` } : null;
}

// `Block` already keeps the node, page format, header facets, heading level and
// macro detection of this very block, so they arrive as accessors instead of
// being recomputed here: one store read, one parent walk, one facet lookup and
// one macro detection per block rather than two (master 0350c00b6).
export function Rendered(props: {
  id: string;
  node: () => ReadonlyNode;
  fmt: () => "md" | "org";
  facets: () => Facets;
  headingLevel: () => number | null;
  macro: () => ReturnType<typeof detectMacro>;
  owner?: string;
  // The reference-count badge. It is a RIGHT FLOAT and must be the FIRST child of
  // `.block-content`: a float attaches to the line box current where the browser
  // reaches it, so emitting it last parked it on a wrapped block's LAST line
  // (GH #454).
  refCountBadge?: JSX.Element;
  outlineScope?: OutlineScope | null;
}): JSX.Element {
  const node = props.node;
  const fmt = props.fmt;
  // Header facets (marker/priority/heading/scheduled/deadline/properties) off the
  // ONE lsdoc parse — read from the cache the store seeded from the backend DTO (no
  // parse on load), recomputed from a single wasm parse only for the edited block.
  const facets = props.facets;
  const headingLevel = props.headingLevel;
  // Plain functions, not memos: a leaf block allocates no reactive node for them.
  const clock = (): LogbookInfo | null => {
    if (!timetrackingEnabled()) return null;
    const marker = facets().marker;
    if (marker !== "DONE" && marker !== "TODO" && marker !== "LATER") return null;
    const info = logbookInfo(node().raw);
    return info.seconds > 0 ? info : null;
  };
  const readOnly = () => blockPageReadOnly(props.id);

  const macro = props.macro;

  // PDF highlight (annotation) blocks render a colored, clickable swatch
  // (AnnotationBody) that opens the PDF at the highlight's page; notes go in
  // child blocks. The detection + rendering live in editor/annotation +
  // components/AnnotationBody.
  const annotation = () => annotationInfo(facets().properties);
  // The highlight text shown in the annotation swatch = the first visible (non-
  // property) line of the block (cheap; the shared line recognizer).
  const annotationLine = () => node().raw.split("\n").find((l) => !isPropertyLine(l) && l.trim() !== "") ?? "";

  // Click edits the block, placing the caret WHERE you clicked when lsdoc span
  // data can map the rendered leaf back through source bytes and hidden props.
  // Anything without trustworthy span data (chips, macro hosts, parser fallback)
  // keeps the old end-of-block behavior.
  let contentRef: HTMLDivElement | undefined;
  const clickOffset = (e: MouseEvent): number | null =>
    contentRef ? renderedClickOffset(contentRef, node().raw, pageByName(node().page)?.format === "org" ? "org" : "md", e) : null;
  // For annotation blocks the editor shows only the highlight text (metadata
  // stays hidden); the colored prefix still jumps to the PDF.
  //
  // The caret offset must be computed at MOUSEDOWN — before the previously-
  // focused editor blurs and reflows the layout. Editing starts immediately;
  // continuing the gesture within the block selects editor text, while crossing
  // into another block escalates to outline selection (see beginEditGesture).
  const onMouseDown = (e: MouseEvent) => {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey || e.altKey) return;
    if (readOnly()) return; // read-only org page — never enter the editor
    if (forbidsEditEntry(e)) return;
    e.stopPropagation(); // keep the row wrapper from arming a second gesture
    beginEditGesture(
      e,
      props.id,
      clickOffset(e) ?? node().raw.length,
      props.owner ?? null,
      props.outlineScope ?? null,
    );
  };

  const displayProps = () => {
    const extra = graphMeta()?.block_hidden_properties ?? [];
    return facets().properties.filter(([k]) => !isRenderHiddenProp(k, extra));
  };
  const bgColor = () => {
    return blockBackgroundColor(facets().properties);
  };

  const body = (
    <Show when={annotation()} fallback={<AstBody raw={node().raw} blockId={props.id} format={fmt()} headingLevel={headingLevel()} />}>
      <AnnotationBody
        highlightId={blockExternalId(props.id) ?? props.id}
        color={annotation()!.color}
        hlPage={annotation()!.hlPage}
        line={annotationLine()}
        page={node().page}
      />
    </Show>
  );

  return (
    <Show
      when={!macro()}
      fallback={
        <div class="block-content macro-host" onMouseDown={onMouseDown}>
          <DeferredStandaloneMacro blockId={props.id} raw={node().raw}>
            <Switch>
              <Match when={macro()!.kind === "query"}>
                <QueryMacro body={macro()!.inner} blockId={props.id} sourceExtent={macro()!.sourceExtent} sourceRaw={macro()!.sourceExtent && node().raw.slice(macro()!.sourceExtent!.start, macro()!.sourceExtent!.end)} />
              </Match>
              <Match when={macro()!.kind === "embed"}>
                <EmbedMacro body={macro()!.inner} blockId={props.id} />
              </Match>
            </Switch>
          </DeferredStandaloneMacro>
        </div>
      }
    >
    <div
      ref={contentRef}
      class="block-content"
      classList={{ done: facets().done, "has-bg": !!bgColor(), [`heading h${headingLevel() ?? ""}`]: headingLevel() != null }}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={onMouseDown}
    >
      {props.refCountBadge}
      {/* One gate for the whole leading chip group: every chip below needs a
          marker or a priority, so an ordinary prose block (most of a large page)
          evaluates one condition and allocates nothing for chips it never shows. */}
      <Show when={facets().marker || facets().priority}>
      <Show when={taskCheckboxState(facets().marker) !== null}>
        <span
          class="block-task-checkbox"
          classList={{ checked: taskCheckboxState(facets().marker) === true }}
          role="checkbox"
          aria-checked={taskCheckboxState(facets().marker) === true}
          title={taskCheckboxState(facets().marker) === true ? "Mark undone" : "Mark done"}
          // Mouse-DOWN (not click) + preventDefault so toggling never enters the
          // block editor (OG parity — matches the block-ref/chip mousedown model).
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            toggleBlockCheckbox(props.id);
          }}
        />{" "}
      </Show>
      <Show when={facets().marker}>
        <span
          class={`block-marker marker-${facets().marker?.toLowerCase()}`}
          classList={{ "marker-clickable": markerLabelClickable(facets().marker) }}
          onClick={(e) => {
            e.stopPropagation();
            toggleBlockMarkerLabel(props.id);
          }}
        >
          {facets().marker}
        </span>{" "}
      </Show>
      <Show when={facets().priority}>
        <span class={`block-priority priority-${facets().priority}`}>[#{facets().priority}]</span>{" "}
      </Show>
      </Show>
      {/* Heading size is applied inside AstBody to ONLY the heading's first line
          (see renderBlocks headingLevel), so a `> quote`/table/etc. continuation in
          the same block renders at normal size — matching OG. */}
      {body}
      {/* Same gate for the trailing chips. `clock()` is only non-null for a
          DONE/TODO/LATER block, so keying on the marker also keeps `logbookInfo`
          off every ordinary block's raw text. */}
      <Show when={facets().marker || facets().scheduled || facets().deadline || displayProps().length > 0}>
      <Show when={clock()}>
        {(info) => <ClockBadge info={info()} />}
      </Show>
      <Show when={facets().scheduled}>
        <span
          class="date-chip scheduled"
          title="Scheduled — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "scheduled", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {facets().scheduled}
        </span>
      </Show>
      <Show when={facets().deadline}>
        <span
          class="date-chip deadline"
          title="Deadline — click to change"
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "deadline", e.clientX, e.clientY);
          }}
        >
          <CalGlyph /> {facets().deadline}
        </span>
      </Show>
      <Show when={displayProps().length > 0}>
        <span class="block-properties">
          <For each={displayProps()}>
            {([k, v]) => (
              <span class="prop">
                <span class="prop-key">{propertyKeyNorm(k)}</span>
                {/* Render the value through the inline parser so a `[[wiki]]`/`#tag`
                    property value becomes a clickable link, matching OG and the
                    page-property path (Page.tsx). Issue #10. */}
                <span class="prop-value"><InlineText text={v} format={fmt()} /></span>
              </span>
            )}
          </For>
        </span>
      </Show>
      </Show>
    </div>
    </Show>
  );
}
