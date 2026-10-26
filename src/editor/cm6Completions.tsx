import { createEffect, createSignal, createUniqueId, on, onCleanup, Show, untrack, useContext } from "solid-js";
import { EditorView, type ViewUpdate } from "@codemirror/view";
import { EditorState } from "@codemirror/state";
import { autocompleteFacets, backend } from "../backend";
import { applyCompletion, refCompletionEnd, withRefCompletionSpace, detectTrigger, pageInsert, tagInsert, orderAcItems, COMMANDS, advancedBlockInsertion, filterAdvancedBlockCommands, commandScore, codeLanguageItems, fuzzyScore, aliasOfLabel, propertyKeyFold, propertyValueKeyAfterBoundary, type Trigger } from "./autocomplete";
import { getTemplates, templateToOutline, type AcItem } from "./completionItems";
import { node, pageByName, persistBlockRefTarget, insertOutlineAfter, deleteBlock, setHeading, withUndoUnit } from "../document";
import { editingId, editingOwner, editingSurface, startEditing, endEdit } from "../editorController";
import { pinyinMatchingEnabled } from "../launcherRanking";
import { graphOwner, latestOwner, ownedWhen, readOwned } from "../owned";
import { navigationName } from "../pageIndex";
import { blockDtoExternalId } from "../blockIdentity";
import { blockFirstLine, formatForBlockId, timeStamp } from "../components/blockParts";
import { isMobilePlatform } from "../nativeChrome";
import { pluginManager } from "../plugins/manager";
import { bindPluginBlockSnapshot, isPluginGraphOwnerCurrent } from "../plugins/ownership";
import { linkAutocompletePolicy } from "./linkDefault";
import { spaceAfterRefCompletion } from "../refCompletionSettings";
import { isAnnotationBlock } from "./annotation";
import { calcSource } from "./calc";
import { setMarker } from "./marker";
import { insertLink, setPriority } from "./format";
import { runJournalSlash } from "../journalSlash";
import { youtubeTimestampMacroFor } from "../components/youtubeTimestampEdit";
import { openDatePicker, openPageProps, setQueryBuilderAutoOpen, searchRemoveAccents } from "../ui";
import { ResultWriteTargetContext, liveWriteTarget } from "../components/queryResultIdentity";
import { QUERY_MACRO_SCAFFOLD } from "./queryMacro";
import { applySheetViewSlashAction } from "../components/block/sheetViewSlash";
import { CaptureCtx } from "../components/Block";
import { registerTransientLayer } from "../transientLayers";
import { EditorAutocomplete } from "../components/EditorAutocomplete";
import { editorCommandFor, isPermittedTabGesture } from "../keybindings";
import { msg, t as translate } from "../i18n";
import { dbg } from "../debug";
import { pushToast } from "../toasts";

/** CM transactions own the buffer and selection; completion never writes files.
 * Reads retain the same ranking, baseline leases and durable ref publication as
 * the textarea. Only input transactions start completion, so model sync/history
 * cannot accidentally reopen a consumed trigger. */
export function cm6Completions(props: {
  id: string; surface: string; format: "md" | "org"; sheet: boolean;
  current: () => boolean; view: () => EditorView | undefined;
  visible: () => string; rawBeforeCommit: () => string;
}) {
  const cap = useContext(CaptureCtx);
  const inResults = useContext(ResultWriteTargetContext);
  const view = () => props.view()!;
  const text = () => view().state.doc.toString();
  const caret = () => view().state.selection.main.head;
  const current = () => props.current() && !!props.view()?.dom.isConnected;
  const [ac, setAc] = createSignal<Trigger | null>(null);
  const [acItems, setAcItems] = createSignal<AcItem[]>([]);
  const [acBlockState, setAcBlockState] = createSignal<"pending" | "error" | "ready" | null>(null);
  const [acIndex, setAcIndex] = createSignal(0);
  const [propertyValueKey, setPropertyValueKey] = createSignal<string | null>(null);
  const acVisible = () => !!ac() && (acItems().length > 0 || acBlockState() !== null);
  let propertyFacets: [string, string[]][] = [];
  let acListRef: HTMLDivElement | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let applying = false, pluginInvocation = 0;
  let completionOriginalRaw: string | null = null;
  let completionLease: { pageName: string; blockId: string; originalRaw: string; session: string } | null = null;
  const releaseLease = (lease: NonNullable<typeof completionLease>) => {
    void readOwned(ownedWhen(() => true), backend().quickSwitch("", 0, lease, false))
      .catch(error => dbg(`completion baseline release: ${String(error)}`));
  };
  const releaseCompletionLease = () => {
    const lease = completionLease; completionLease = null;
    if (lease) releaseLease(lease);
  };
  const closeAc = () => {
    clearTimeout(timer); releaseCompletionLease(); completionOriginalRaw = null;
    setAc(null); setAcBlockState(null); setAcItems([]); setAcIndex(0); setPropertyValueKey(null);
  };
  const sameAcTrigger = (left: Trigger | null, right: Trigger) => left !== null
    && left.kind === right.kind && left.query === right.query && left.start === right.start
    && left.end === right.end && left.property === right.property
    && (left.propertyValues ?? []).join("\0") === (right.propertyValues ?? []).join("\0");
  const detectEditorTrigger = () => props.sheet || isAnnotationBlock(node(props.id).raw, props.format) || calcSource(text()) !== null
    ? null : detectTrigger(text(), caret(), propertyValueKey(), props.format);
  const propertyValueItems = (key: string, query: string, used: readonly string[] = []): AcItem[] => {
    const values = propertyFacets.find(([candidate]) => candidate === key)?.[1] ?? [];
    const q = query.trim(), excluded = new Set(used.map(value => value.toLocaleLowerCase()));
    const ranked: AcItem[] = values.map((value, index) => ({ value, index, score: q ? fuzzyScore(q, value) : 1 }))
      .filter(({ value, score }) => score > 0 && !excluded.has(value.toLocaleLowerCase()))
      .sort((a, b) => b.score - a.score || a.index - b.index).slice(0, 100)
      .map(({ value }) => ({ label: value, propertyValue: value }));
    if (q && !values.some(value => value.toLowerCase() === q.toLowerCase()))
      ranked.push({ get label() { return translate("editor-block-create", { name: String(q) }); }, propertyValue: q });
    return ranked;
  };
  const autocompleteScope = {};
  const updateAutocomplete = async () => {
    if (!current()) return;
    const t = detectEditorTrigger();
    if (!t) {
      closeAc();
      return;
    }
    setAc(t);
    setAcBlockState(null);
    setAcIndex(0);
    const requestOwner = latestOwner(autocompleteScope, "suggestions", graphOwner(() => current() && sameAcTrigger(ac(), t)));
    if (t.kind === "property-name") {
      let facets: [string, string[]][];
      try {
        facets = await autocompleteFacets();
      } catch (error) {
        // Completion is an optional aid: a transient facet-query failure must
        // never reject the editor input event or pile up global error toasts.
        dbg(`property-autocomplete: ${String(error)}`);
        if (requestOwner()) setAcItems([]);
        return;
      }
      if (!requestOwner()) return;
      propertyFacets = facets;
      const q = t.query.trim();
      const ranked = facets
        .map(([key], index) => ({ key, index, score: q ? fuzzyScore(q, key) : 1 }))
        .filter(({ score }) => score > 0)
        .sort((left, right) => right.score - left.score || left.index - right.index)
        .slice(0, 100)
        .map(({ key }) => ({ label: key, propertyName: key }));
      const created = propertyKeyFold(t.query);
      if (created && !facets.some(([key]) => key === created)) {
        ranked.unshift({ get label() { return translate("editor-block-create", { name: String(created) }); }, propertyName: created });
      }
      setAcItems(ranked);
      return;
    }
    if (t.kind === "property-value") {
      setAcItems(propertyValueItems(t.property!, t.query, t.propertyValues));
      return;
    }
    if (t.kind === "code-language") {
      setAcItems(codeLanguageItems(t.query).map((language) => ({
        get label() { return language.label; },
        sub: [language.id, ...language.aliases].join(" · "),
        insert: language.id,
        // If a completed fence scaffold already follows, land on its code line;
        // a hand-typed one-line fence stays at the end and Enter behaves normally.
        caret: language.id.length + (text()[t.end] === "\n" ? 1 : 0),
      })));
      return;
    }
    if (t.kind === "advanced-command") {
      const fmt = formatForBlockId(props.id);
      setAcItems(filterAdvancedBlockCommands(t.query).map((command) => {
        const insertion = advancedBlockInsertion(command, fmt);
        return { get label() { return command.label; }, insert: insertion.insert, caret: insertion.caret };
      }));
      return;
    }
    if (t.kind === "command") {
      const q = t.query;
      const tmpls = await getTemplates();
      if (!requestOwner()) return;
      // Commands AND templates in one fuzzy-ranked list, so a strong template
      // match can outrank a weak command (and vice-versa). Empty query (bare `/`)
      // lists all commands in defined order, no templates. `idx` preserves the
      // defined order — commands before templates — as the stable tiebreaker.
      const showAllTemplates = !!q && "template".startsWith(q.toLowerCase()); // /t…/template lists them all
      const scored: { item: AcItem; s: number; idx: number }[] = [];
      COMMANDS.forEach((c) => {
        // /drawio launches an external editor — desktop only (GH #38).
        if (c.action === "drawio" && isMobilePlatform) return;
        const s = q ? commandScore(q, c) : 1;
        if (s > 0)
          scored.push({ item: { get label() { return c.label; }, insert: c.insert, caret: c.caret, action: c.action, taskMarker: c.taskMarker }, s, idx: q ? c.matchTieOrder : c.bareOrder });
      });
      pluginManager.slashCommands().forEach(({ pluginId, contribution }, i) => {
        const s = q ? fuzzyScore(q, contribution.title) : 1;
        if (s > 0) {
          scored.push({
            item: {
              label: contribution.title,
              sub: "Plugin",
              plugin: { pluginId, contributionId: contribution.id, insertText: contribution.insertText },
            },
            s,
            idx: COMMANDS.length + i,
          });
        }
      });
      if (q) {
        tmpls.forEach((tp, j) => {
          const s = showAllTemplates ? 1 : fuzzyScore(q, tp.name);
          if (s > 0)
            scored.push({
              item: { get label() { return translate("editor-block-template", { name: String(tp.name) }); }, templateNodes: tp.blocks },
              s,
              idx: COMMANDS.length + pluginManager.slashCommands().length + j,
            });
        });
      }
      scored.sort((a, b) => b.s - a.s || a.idx - b.idx);
      setAcItems(scored.map((x) => x.item));
      return;
    }
    if (t.kind === "block") {
      // `((` searches blocks by page; bare `((` stays hidden. Selection inserts
      // the target's durable external ID (see selectAc).
      setAcItems([]);
      setAcBlockState("pending");
      let result: import("../owned").Owned<import("../types").RefGroup[]>;
      try {
        result = await readOwned(requestOwner, backend().search(t.query, 20, "block-picker"));
      } catch (error) {
        if (requestOwner()) {
          dbg(`block-picker: ${String(error)}`);
          setAcBlockState("error");
        }
        return;
      }
      if (result.kind === "stale") return;
      setAcBlockState("ready");
      const items: AcItem[] = [];
      for (const g of result.value) {
        for (const b of g.blocks) {
          items.push({
            label: blockFirstLine(b.raw, pageByName(g.page)?.format === "org" ? "org" : "md") || g.page,
            sub: g.page,
            blockRef: { uuid: b.id, externalId: blockDtoExternalId(b), page: g.page, kind: g.kind },
          });
        }
      }
      setAcItems(items);
      return;
    }
    // OG leaves blank page/tag search active but renders no rows and does not
    // ask the backend for its legacy all-pages result.
    const q = t.query.trim();
    if (!q) {
      setAcItems([]);
      return;
    }
    completionOriginalRaw ??= node(props.id).raw;
    const editor = { pageName: node(props.id).page, blockId: props.id, originalRaw: completionOriginalRaw };
    if (!cap && pinyinMatchingEnabled() && !completionLease) {
      completionLease = { ...editor, session: crypto.randomUUID() };
    }
    const lease = completionLease;
    // Closing can overtake a blocking native query. Release again after its
    // reply (including failure) so it cannot retain obsolete baseline keys.
    let result: import("../owned").Owned<import("../types").PageEntry[]>;
    try {
      result = await readOwned(requestOwner, cap ? cap.quickSwitch(t.query, 100) : backend().quickSwitch(t.query, 100, lease ?? editor, pinyinMatchingEnabled()));
    } finally {
      if (lease && lease !== completionLease) releaseLease(lease);
    }
    if (result.kind === "stale") return;
    const pageItem = (name: string): AcItem =>
      t.kind === "page"
        ? { label: name, insert: pageInsert(name), get sub() { return aliasOfLabel(name, navigationName(name)); } }
        : { label: `#${name}`, insert: tagInsert(name), get sub() { return aliasOfLabel(name, navigationName(name)); } }; // tag context reads "#name"
    const createItem: AcItem =
      t.kind === "page"
        ? { get label() { return translate("editor-block-create", { name: String(q) }); }, insert: pageInsert(q) }
        : { get label() { return translate("editor-block-create-tag", { name: String(q) }); }, insert: tagInsert(q) };
    setAcItems(orderAcItems(
      result.value.map((page) => ({ name: page.name, matchClass: page.match_class, item: pageItem(page.name) })),
      { name: q, item: createItem },
      { query: q, policy: linkAutocompletePolicy(), removeAccents: searchRemoveAccents() },
    ));
  };

  // Completion edits are a single CM transaction, with the caret restored before
  // any picker takes focus. The document update listener uses the ordinary commit.
  const apply = (raw: string, start: number, end = start, focus = true) => {
    if (!current()) return;
    applying = true;
    try {
      view().dispatch({ changes: { from: 0, to: view().state.doc.length, insert: raw },
        selection: { anchor: start, head: end }, userEvent: "input.complete", scrollIntoView: true });
    } finally { applying = false; }
    if (focus) view().focus();
  };
  const replaceTrigger = (insert: string, caretOffset?: number) => {
    const trigger = ac(); if (!trigger || !current()) return;
    const result = applyCompletion(text(), trigger.start, refCompletionEnd(text(), trigger.end, insert), insert, caretOffset);
    const spaced = caretOffset === undefined
      ? withRefCompletionSpace(result.raw, result.caret, insert, spaceAfterRefCompletion()) : result;
    closeAc(); apply(spaced.raw, spaced.caret);
  };
  const openPlanning = (kind: "scheduled" | "deadline") => {
    const rect = view().contentDOM.getBoundingClientRect();
    openDatePicker(props.id, kind, rect.left, rect.bottom + 4, inResults(props.id) ? liveWriteTarget(props.id) : undefined);
  };
  const openJournal = () => {
    const owner = graphOwner(current);
    const before = text(), position = caret();
    const rect = view().contentDOM.getBoundingClientRect();
    openDatePicker(props.id, { insertJournal: title => {
      if (!owner() || text() !== before) {
        pushToast(msg("journal-slash-the-date-was-not-inserted-because-the-graph"), "info"); return;
      }
      const insert = pageInsert(title), edit = applyCompletion(before, position, position, insert);
      const result = withRefCompletionSpace(edit.raw, edit.caret, insert, spaceAfterRefCompletion());
      apply(result.raw, result.caret, result.caret, false);
    } }, rect.left, rect.bottom + 4);
  };
  const selectAc = (item: AcItem) => {
    const t = ac(); if (!t || !current() || view().compositionStarted) return;
    if (item.propertyName) {
      const key = propertyKeyFold(item.propertyName);
      const result = applyCompletion(text(), t.start, t.end, `${key}:: `);
      apply(result.raw, result.caret);
      setPropertyValueKey(key);
      setAc({ kind: "property-value", query: "", start: result.caret, end: result.caret, property: key });
      setAcIndex(0); setAcItems(propertyValueItems(key, ""));
      return;
    }
    if (item.propertyValue !== undefined) { replaceTrigger(item.propertyValue); return; }
    if (item.blockRef) {
      const { uuid, externalId, page, kind } = item.blockRef;
      const owner = graphOwner(current), trigger = ac(), before = text();
      let inserted = false;
      const intended = () => owner() && ac() === trigger && text() === before;
      void persistBlockRefTarget(uuid, page, kind, undefined, externalId, () => {
        if (!intended()) return null;
        const sourcePage = node(props.id)?.page; if (!sourcePage) return null;
        replaceTrigger(`((${externalId}))`); inserted = true; return sourcePage;
      }, intended).then(saved => {
        if (!saved && owner() && (inserted || ac() === trigger))
          pushToast(msg("editor-block-could-not-save-the-block-reference-resolve-the-page-save-and-try-again"), "error");
      }).catch(error => { if (owner()) pushToast(msg("editor-block-could-not-save-the-block-reference", { detail: String(error) }), "error"); });
      return;
    }
    if (item.plugin) {
      const before = text(), selection = view().state.selection.main;
      const n = node(props.id); let depth = 0, parent = n.parent;
      while (parent && node(parent) && depth < 1_000) { depth++; parent = node(parent).parent; }
      const snapshot = bindPluginBlockSnapshot({ id: n.id, raw: n.raw, parentId: n.parent, depth, format: props.format });
      if (!snapshot) return;
      const token = ++pluginInvocation, ownerId = editingOwner(), surface = editingSurface(), block = editingId();
      const plugin = item.plugin;
      const intended = () => {
        if (!current() || token !== pluginInvocation || !isPluginGraphOwnerCurrent(snapshot.owner)
          || editingId() !== block || editingOwner() !== ownerId || editingSurface() !== surface
          || text() !== before || node(props.id).raw !== snapshot.block.raw) return false;
        const s = view().state.selection.main;
        return s.anchor === selection.anchor && s.head === selection.head
          && sameAcTrigger(detectTrigger(text(), caret(), propertyValueKey(), props.format), t);
      };
      closeAc();
      void pluginManager.invokeSlashCommand(plugin.pluginId, plugin.contributionId, snapshot).then(effects => {
        if (!intended()) {
          if (current() && isPluginGraphOwnerCurrent(snapshot.owner) && text() !== before)
            pushToast(msg("editor-block-plugin-result-was-not-inserted-because-the-block-changed-while-it-ran"), "info");
          return;
        }
        const effect = effects.find(candidate => candidate.kind === "insert-at-caret");
        const insert = effect?.kind === "insert-at-caret" ? effect.text : plugin.insertText;
        if (insert === undefined) return;
        const result = applyCompletion(before, t.start, t.end, insert);
        apply(result.raw, result.caret);
      }).catch(error => pushToast(msg("editor-block-plugin-slash-command-failed", { detail: String(error) }), "error"));
      return;
    }
    if (item.templateNodes) {
      const result = applyCompletion(text(), t.start, t.end, "");
      closeAc(); apply(result.raw, result.caret);
      const nodes = item.templateNodes.map(n => templateToOutline(n, node(props.id).page));
      const wasEmpty = node(props.id).raw.trim() === "" && node(props.id).children.length === 0;
      const last = insertOutlineAfter(props.id, nodes);
      if (!last) { pushToast(msg("editor-block-outline-is-too-deep-to-insert"), "error"); return; }
      if (wasEmpty) deleteBlock(props.id);
      startEditing(last, node(last).raw.length); return;
    }
    switch (item.action) {
      case "task-marker": {
        if (!item.taskMarker) return;
        const removed = applyCompletion(text(), t.start, t.end, "");
        const raw = setMarker(removed.raw, item.taskMarker);
        closeAc(); apply(raw, raw.length); return;
      }
      case "heading-auto": case "heading-1": case "heading-2": case "heading-3": case "heading-4": {
        const state = item.action === "heading-auto" ? true : Number(item.action.slice("heading-".length)) as 1 | 2 | 3 | 4;
        const removed = applyCompletion(text(), t.start, t.end, "");
        closeAc();
        withUndoUnit(`heading:${props.id}`, [node(props.id).page], () => {
          apply(removed.raw, removed.caret); setHeading(props.id, state);
        });
        const raw = props.visible();
        apply(raw, Math.min(raw.length, removed.caret + (state === true ? 0 : state + 1))); return;
      }
      case "scheduled": case "deadline":
        replaceTrigger(""); openPlanning(item.action); return;
      case "now-time": replaceTrigger(timeStamp()); return;
      case "youtube-timestamp": replaceTrigger(youtubeTimestampMacroFor(view().contentDOM) ?? ""); return;
      case "page-reference":
        replaceTrigger("[[]]", 2); void updateAutocomplete(); return;
      case "insert-link": {
        const removed = applyCompletion(text(), t.start, t.end, "", 0);
        const edit = insertLink(removed.raw, t.start, t.start, props.format);
        closeAc(); apply(edit.text, edit.start, edit.end); return;
      }
      case "query-builder":
        replaceTrigger(QUERY_MACRO_SCAFFOLD); setQueryBuilderAutoOpen(props.id); endEdit("query-builder"); return;
      case "code-block": {
        const result = applyCompletion(text(), t.start, t.end, "```\n\n```", 3);
        closeAc(); apply(result.raw, result.caret);
        setAc({ kind: "code-language", query: "", start: result.caret, end: result.caret });
        setAcItems(codeLanguageItems("").map(language => ({
          get label() { return language.label; }, sub: [language.id, ...language.aliases].join(" · "),
          insert: language.id, caret: language.id.length + 1,
        }))); return;
      }
      case "page-props": {
        replaceTrigger(""); const rect = view().contentDOM.getBoundingClientRect();
        openPageProps(node(props.id).page, rect.left, rect.bottom + 4); return;
      }
      case "sheet-grid": case "sheet-table": case "sheet-board":
        replaceTrigger(""); applySheetViewSlashAction(props.id, item.action === "sheet-grid" ? "grid" : item.action === "sheet-table" ? "table" : "board"); return;
      case "date-picker": replaceTrigger(""); openJournal(); return;
      case "tomorrow": case "yesterday": case "today": case "thatday":
        runJournalSlash(item.action, node(props.id).page, replaceTrigger); return;
      // Native asset actions remain owned by the later assets/native slice.
      case "upload-asset": case "record": case "drawio": return;
      case "priority-a": case "priority-b": case "priority-c": {
        const lines = (text().slice(0, t.start) + text().slice(t.end)).split("\n");
        lines[0] = setPriority(lines[0], item.action === "priority-a" ? "A" : item.action === "priority-b" ? "B" : "C") + " ";
        closeAc(); apply(lines.join("\n"), lines[0].length); return;
      }
    }
    replaceTrigger(item.insert ?? "", item.caret);
  };

  const refresh = () => {
    if (!current() || view().compositionStarted) return;
    const next = detectEditorTrigger();
    if (!next) { closeAc(); return; }
    const previous = ac();
    if (next.kind !== "page" && next.kind !== "tag") releaseCompletionLease();
    if ((next.kind === "page" || next.kind === "tag")
      && (!previous || previous.kind !== next.kind || previous.start !== next.start)) {
      releaseCompletionLease(); completionOriginalRaw = props.rawBeforeCommit();
    }
    setAc(next); setAcIndex(0); setAcBlockState(next.kind === "block" ? "pending" : null);
    if (next.kind === "block" || !previous || previous.kind !== next.kind || previous.start !== next.start
      || ((next.kind === "page" || next.kind === "tag") && !next.query.trim())) setAcItems([]);
    clearTimeout(timer);
    timer = setTimeout(() => { void updateAutocomplete().catch(error => dbg(`autocomplete: ${String(error)}`)); }, 90);
  };
  const selectionChanged = () => {
    const open = ac();
    if (!open || !["page", "tag", "block"].includes(open.kind)) return;
    const here = detectEditorTrigger();
    if (!here || here.kind !== open.kind || here.start !== open.start) closeAc();
  };
  const changed = (update: ViewUpdate) => {
    if (applying || !current()) return;
    if (update.transactions.some(tr => tr.isUserEvent("input") || tr.isUserEvent("delete"))) {
      if (view().compositionStarted || update.transactions.some(tr => tr.isUserEvent("input.type.compose"))) return;
      const value = text(), pos = caret();
      const key = propertyValueKeyAfterBoundary(value, pos, value[pos - 1] ?? "");
      if (key) setPropertyValueKey(key);
      refresh();
    } else if (update.selectionSet) selectionChanged();
  };
  const input = (event: InputEvent) => {
    if (!current() || event.isComposing || view().compositionStarted || applying) return;
    // A platform input may describe an unchanged state (fixture/IME notification).
    // Actual native edits are reconciled by CM and handled by changed().
    const key = propertyValueKeyAfterBoundary(text(), caret(), event.data ?? "");
    if (key) setPropertyValueKey(key);
    refresh();
  };
  // The property-name workflow authors name|::, starting with an empty ::.
  // Adjust the input transaction itself so observers never see a stale caret.
  const inputSelection = EditorState.transactionFilter.of(tr => {
    if (props.sheet || !tr.isUserEvent("input.type") || tr.isUserEvent("input.type.compose")) return tr;
    const pos = tr.newSelection.main.head, line = tr.newDoc.lineAt(pos);
    if (pos === line.from + 2 && line.text.startsWith("::"))
      return [tr, { selection: { anchor: line.from }, sequential: true }];
    return tr;
  });
  const command = (id: string): boolean => {
    if (!current() || view().compositionStarted) return false;
    if (id === "editor/open-date-picker") { openPlanning("scheduled"); return true; }
    if (id === "editor/insert-youtube-timestamp") {
      const macro = youtubeTimestampMacroFor(view().contentDOM);
      if (macro) {
        const s = view().state.selection.main, insert = macro + " ";
        closeAc(); apply(text().slice(0, s.from) + insert + text().slice(s.to), s.from + insert.length);
      }
      return true;
    }
    if (id !== "editor/open-slash-menu" && id !== "editor/insert-page-ref" && id !== "editor/insert-block-ref") return false;
    const s = view().state.selection.main, selected = text().slice(s.from, s.to);
    const page = id === "editor/insert-page-ref", slash = id === "editor/open-slash-menu";
    const insert = slash ? "/" : (page ? "[[" : "((") + selected + (page ? "]]" : "))");
    apply(text().slice(0, s.from) + insert + text().slice(s.to), s.from + (slash ? 1 : selected.length + 2));
    void updateAutocomplete(); return true;
  };
  const keydown = (event: KeyboardEvent): boolean => {
    if (acVisible() && event.key === "Escape") { closeAc(); return true; }
    if (ac() && acItems().length) {
      const n = acItems().length;
      if (event.key === "ArrowDown") { setAcIndex((acIndex() + 1) % n); return true; }
      if (event.key === "ArrowUp") { setAcIndex((acIndex() - 1 + n) % n); return true; }
      if (event.key === "Enter" || isPermittedTabGesture(event)) { selectAc(acItems()[acIndex()]); return true; }
    }
    const id = editorCommandFor(event);
    return !!id && command(id);
  };
  const [rect, setRect] = createSignal<{ left: number; top: number; bottom: number } | null>(null);
  const reanchor = () => {
    if (!props.view() || !acVisible()) return;
    const bounds = view().contentDOM.getBoundingClientRect();
    const line = view().coordsAtPos(caret());
    setRect({ left: bounds.left, top: line?.top ?? bounds.top, bottom: line?.bottom ?? bounds.bottom });
  };
  const style = (): Record<string, string> => {
    const r = rect(); if (!r) return {};
    const win = view().dom.ownerDocument.defaultView!;
    const below = win.innerHeight - r.bottom, openUp = below < 300 && r.top > below;
    const left = Math.max(8, Math.min(r.left, win.innerWidth - (acListRef?.offsetWidth || 240) - 8));
    return openUp ? { left: `${left}px`, bottom: `${win.innerHeight - r.top + 2}px` }
      : { left: `${left}px`, top: `${r.bottom + 2}px` };
  };
  createEffect(() => { ac(); if (acVisible()) queueMicrotask(() => { if (current()) reanchor(); }); });
  const layerId = `cm6-completion-${createUniqueId()}`;
  createEffect(() => {
    if (!acVisible()) return;
    onCleanup(registerTransientLayer({ id: layerId, root: () => acListRef ?? null,
      trigger: () => props.view()?.contentDOM ?? null,
      dismiss: () => { closeAc(); if (current()) view().focus(); return true; } }));
  });
  createEffect(on(pinyinMatchingEnabled, () => {
    const trigger = untrack(ac);
    if (trigger?.kind === "page" || trigger?.kind === "tag") {
      if (!pinyinMatchingEnabled()) releaseCompletionLease();
      void updateAutocomplete();
    }
  }, { defer: true }));
  const mount = () => {
    const win = view().dom.ownerDocument.defaultView!;
    win.addEventListener("scroll", reanchor, true); win.addEventListener("resize", reanchor);
    onCleanup(() => { win.removeEventListener("scroll", reanchor, true); win.removeEventListener("resize", reanchor); });
  };
  onCleanup(() => { pluginInvocation++; clearTimeout(timer); releaseCompletionLease(); });
  const Popup = () => <Show when={acVisible()}><EditorAutocomplete items={acItems()} index={acIndex()} style={style()}
    listRef={element => { acListRef = element; }} select={selectAc}
    blockState={acBlockState()} retry={() => void updateAutocomplete()} /></Show>;
  return { mount, inputSelection, changed, input, keydown, command, selectionChanged, close: closeAc,
    pause: () => clearTimeout(timer), refresh, Popup };
}

