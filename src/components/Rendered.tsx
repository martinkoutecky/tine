import { t } from "../i18n";
import { Show, Switch, Match, useContext, type JSX } from "solid-js";
import { pageByName, blockPageReadOnly, blockExternalId, type OutlineScope, type Node as ReadonlyNode } from "../document";
import { isPropertyLine } from "../render/block";
import { renderedProperties, type Facets } from "../render/facets";
import { AstBody } from "../render/body";
import { DeferredStandaloneMacro } from "./DeferredStandaloneMacro";
import { QueryMacro, EmbedMacro } from "./Macro";
import { timetrackingEnabled, openDatePicker } from "../ui";
import { ResultWriteTargetContext } from "./queryResultIdentity";
import { PropertyRows } from "../render/PropertyRows";
import { taskCheckboxState } from "../markers";
import { flashTaskToggle } from "../justToggled";
import { markerLabelClickable } from "../editor/repeat";
import { logbookInfo, type LogbookInfo } from "../logbook";
import { annotationInfo } from "../editor/annotation";
import { AnnotationBody } from "./AnnotationBody";
import { forbidsEditEntry } from "../editor/editTargets";
import { blockBackgroundColor } from "../blockColors";
import { queryMacroExtent, type MacroExtent } from "../editor/queryMacro";
import { isQuarantined, parseBlock, parserReady, soleBlockMacro } from "../render/parse";
import { isQueryMacroName } from "../editor/queryMacro";
import { beginEditGesture, renderedClickOffset } from "./blockGestures";
import { HIDE_ALL, splitProps } from "../editor/properties";
import { CalGlyph, ClockBadge, toggleBlockMarkerLabel, toggleBlockCheckbox } from "./blockParts";
import { OccurrenceHostContext } from "../refOccurrence";
import type { Span } from "../render/ast";
import { BlockRenderBoundary } from "./BlockRenderBoundary";

// The rendered (non-editing) body of one block: header facets, body, chips and
// properties. Extracted from Block.tsx (og-F) with no behaviour change.

// Detect a block whose entire body is a single {{query}}/{{tine-query}}/{{embed}} macro.
export function detectMacro(raw: string, format: "md" | "org" = "md"): { kind: "query" | "embed"; inner: string; sourceExtent?: MacroExtent; span?: Span } | null {
  if (!parserReady() || !raw.includes("{{") || isQuarantined(parseBlock(raw, format === "org"))) return null;
  // lsdoc's inline macro stops at the first `}}` (inside an options map),
  // leaving a phantom closing brace and denying the standalone host. Use the
  // shared query reader's complete extent and the parser's visible body, so
  // option-bearing queries keep the same reactive host as bare queries.
  const queryExtent = queryMacroExtent(raw);
  if (queryExtent && splitProps(raw, HIDE_ALL, format).visible.trim() === raw.slice(queryExtent.start, queryExtent.end)) {
    return { kind: "query", inner: `${queryExtent.name} ${queryExtent.argument}`, sourceExtent: queryExtent };
  }
  const macro = soleBlockMacro(raw, format);
  if (!macro) return null;
  if (isQueryMacroName(macro.name)) {
    const sourceExtent = queryMacroExtent(raw) ?? undefined;
    return sourceExtent ? { kind: "query", inner: `${sourceExtent.name} ${sourceExtent.argument}`, sourceExtent } : null;
  }
  return macro.name === "embed" ? { kind: "embed", inner: `${macro.name} ${macro.args.join(", ")}`, span: macro.span } : null;
}

// `Block` already keeps the node, page format, header facets, heading level and
// macro detection of this very block, so they arrive as accessors instead of
// being recomputed here: one store read, one parent walk, one facet lookup and
// one macro detection per block rather than two (master 0350c00b6).
export function Rendered(props: Parameters<typeof RenderedContent>[0]): JSX.Element {
  // One Solid boundary per displayed block, no extra healthy DOM, parser work,
  // graph reads or writes. The editor is outside it, so raw text remains repairable.
  return <BlockRenderBoundary raw={props.node().raw}><RenderedContent {...props} /></BlockRenderBoundary>;
}

function RenderedContent(props: {
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
  // A host that owns the click (a sheet cell: select on press, edit on double-click or
  // through its own caret mapping) shows the body with the block's chips and properties
  // but starts no edit gesture of its own.
  inertBody?: boolean;
}): JSX.Element {
  // In a query result, Scheduled/Deadline resolve the result row again at commit.
  const resultTarget = useContext(ResultWriteTargetContext);
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
    const info = logbookInfo(node().raw, fmt());
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

  const displayProps = () => renderedProperties(node().raw, fmt());
  const bgColor = () => {
    return blockBackgroundColor(facets().properties);
  };

  const body = () => (
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
    <OccurrenceHostContext.Provider value={props.owner ? { id: props.id, owner: props.owner } : null}>
    <Show
      when={!macro()}
      fallback={
        <div class="block-content macro-host" onMouseDown={props.inertBody ? undefined : onMouseDown}>
          <DeferredStandaloneMacro blockId={props.id} raw={node().raw}>
            <Switch>
              <Match when={macro()!.kind === "query"}>
                <QueryMacro body={macro()!.inner} blockId={props.id} sourceExtent={macro()!.sourceExtent} sourceRaw={macro()!.sourceExtent && node().raw.slice(macro()!.sourceExtent!.start, macro()!.sourceExtent!.end)} />
              </Match>
              <Match when={macro()!.kind === "embed"}>
                <EmbedMacro body={macro()!.inner} blockId={props.id} span={macro()!.span} />
              </Match>
            </Switch>
          </DeferredStandaloneMacro>
        </div>
      }
    >
    <div
      ref={contentRef}
      class="block-content"
      // GH #375: the rendered-selection toolbar formats text selected here
      // (RenderedSelectionToolbar; the name is RENDERED_BLOCK_ATTR). An
      // annotation swatch edits through HIDE_ALL, which the BUILTIN_HIDE span
      // mapping does not describe, so it opts out.
      data-rendered-block={annotation() ? undefined : props.id}
      classList={{ done: facets().done, "has-bg": !!bgColor(), [`heading h${headingLevel() ?? ""}`]: headingLevel() != null }}
      style={bgColor() ? { background: bgColor() } : undefined}
      onMouseDown={props.inertBody ? undefined : onMouseDown}
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
          title={taskCheckboxState(facets().marker) === true ? t("rendered-mark-undone") : t("rendered-mark-done")}
          // Mouse-DOWN (not click) + preventDefault so toggling never enters the
          // block editor (OG parity — matches the block-ref/chip mousedown model).
          onMouseDown={(e) => {
            e.stopPropagation();
            e.preventDefault();
            const box = e.currentTarget;
            toggleBlockCheckbox(props.id);
            flashTaskToggle(box);
          }}
          // A direct listener also makes the checked box a Chromium touch
          // adjustment candidate, rather than redirecting a near-edge tap to
          // an adjacent bullet/task. The toggle still belongs to mousedown.
          on:click={(e) => { e.stopPropagation(); e.preventDefault(); }}
        />{" "}
      </Show>
      <Show when={facets().marker}>
        <span
          class={`block-marker marker-${facets().marker?.toLowerCase()}`}
          classList={{ "marker-clickable": markerLabelClickable(facets().marker) }}
          onClick={(e) => {
            e.stopPropagation();
            const chip = e.currentTarget;
            toggleBlockMarkerLabel(props.id);
            flashTaskToggle(chip);
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
      {body()}
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
          title={t("rendered-scheduled-click-to-change")}
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "scheduled", e.clientX, e.clientY, resultTarget(props.id));
          }}
        >
          <CalGlyph /> {facets().scheduled}
        </span>
      </Show>
      <Show when={facets().deadline}>
        <span
          class="date-chip deadline"
          title={t("rendered-deadline-click-to-change")}
          onClick={(e) => {
            e.stopPropagation();
            openDatePicker(props.id, "deadline", e.clientX, e.clientY, resultTarget(props.id));
          }}
        >
          <CalGlyph /> {facets().deadline}
        </span>
      </Show>
      <PropertyRows entries={displayProps()} format={fmt()} blockId={props.id} />
      </Show>
    </div>
    </Show>
    </OccurrenceHostContext.Provider>
  );
}
