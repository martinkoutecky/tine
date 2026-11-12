import { createEffect, createSignal, on, onCleanup, untrack } from "solid-js";
import { backend } from "../backend";
import { propertyCompletionSession } from "./propertyCompletionSession";
import { acceptBlockRefPick, blockPickItems } from "./blockRefPick";
import { applyCompletion, refCompletionEnd, withRefCompletionSpace, detectTrigger, pageInsert, tagInsert, orderAcItems, COMMANDS, advancedBlockInsertion, filterAdvancedBlockCommands, commandScore, codeLanguageItems, fuzzyScore, aliasOfLabel, propertyKeyFold, type Trigger } from "./autocomplete";
import { getTemplates, templateToOutline, type AcItem } from "./completionItems";
import { node, insertEditorOutline, setHeading, withUndoUnit, type PendingEdit } from "../document";
import { editingId, editingOwner, editingSurface, startEditing, endEdit } from "../editorController";
import { pinyinMatchingEnabled } from "../launcherRanking";
import { graphEpoch } from "../graphSession";
import { graphOwner, latestOwner, ownedWhen, readOwned } from "../owned";
import { navigationName } from "../pageIndex";
import { formatForBlockId, timeStamp } from "../components/blockParts";
import { isMobilePlatform } from "../nativeChrome";
import { pluginManager } from "../plugins/manager";
import { bindPluginBlockSnapshot, isPluginGraphOwnerCurrent } from "../plugins/ownership";
import { linkAutocompletePolicy } from "./linkDefault";
import { spaceAfterRefCompletion } from "../refCompletionSettings";
import { codeFences } from "./fences";
import { setMarker } from "./marker";
import { insertLink, setPriority } from "./format";
import { runJournalSlash } from "../journalSlash";
import { youtubeTimestampMacroFor } from "../components/youtubeTimestampEdit";
import { openDatePicker, openPageProps, setQueryBuilderAutoOpen, searchRemoveAccents } from "../ui";
import type { liveWriteTarget } from "../components/queryResultIdentity";
import type { PageEntry } from "../types";
import { QUERY_MACRO_SCAFFOLD } from "./queryMacro";
import { applySheetViewSlashAction } from "../components/block/sheetViewSlash";
import type { editorAssets } from "./editorAssets";
import { msg, t as translate } from "../i18n";
import { dbg } from "../debug";
import { pushToast } from "../toasts";
import { coalescedRead } from "../coalescedRead";

/** The buffer, selection and projected-region transitions belong to the editor.
 * All suggestion sessions and acceptance decisions belong to this core. */
export interface CompletionAdapter {
  id: string;
  format(): "md" | "org";
  current(): boolean;
  composing(): boolean;
  suppressTriggers(): boolean;
  /** CM reconciles the scaffold transaction through refresh; textarea returns
   * directly from its scaffold input handler. Preserve that empty opener. */
  keepEmptyLanguagePicker?: boolean;
  text(): string;
  selection(): readonly [number, number];
  caret(): number;
  rawBeforeCommit(): string;
  visible(): string;
  /** Taking `from..to` out of the editor and the block, decided but not made: a gesture that removes its
   *  trigger text and changes the outline asks the kernel about the text this leaves and writes it only
   *  on an answer (og-enter-family-1109). */
  stageDelete(from: number, to: number): PendingEdit;
  apply(raw: string, start: number, end?: number, focus?: boolean, restoreGuard?: () => boolean): void;
  replace(raw: string, caret: number, close: () => void): void;
  afterEdit(callback: () => void): void;
  languagePicker(active: boolean): void;
  closed(): void;
  element(): HTMLElement;
  resultTarget(): ReturnType<typeof liveWriteTarget> | undefined;
  openJournal(): void;
  assets: Pick<ReturnType<typeof editorAssets>, "uploadAsset" | "desktopVoiceMemoToggle" | "createDrawioDiagram">;
  quickSwitch?: (query: string, limit: number) => Promise<PageEntry[]>;
  refOwner?(): () => boolean;
}

/** OG property-name authoring moves before the delimiter only for a single
 * colon input. A multi-character input must keep the browser's selection. */
export function propertyNameInputCaret(value: string, caret: number, inserted: string): number | null {
  if (inserted !== ":" || caret < 2) return null;
  const start = value.lastIndexOf("\n", caret - 3) + 1;
  return caret - start === 2 && value.slice(start, caret) === "::" ? start : null;
}

export function editorCompletions(props: CompletionAdapter) {
  const current = props.current, text = props.text, caret = props.caret;
  const apply = props.apply;
  const [ac, setAc] = createSignal<Trigger | null>(null);
  const [acItems, setAcItems] = createSignal<AcItem[]>([]);
  const [acBlockState, setAcBlockState] = createSignal<"pending" | "error" | "ready" | null>(null);
  const [acIndex, setAcIndex] = createSignal(0);
  const acVisible = () => !!ac() && (acItems().length > 0 || acBlockState() !== null);
  const propertyCompletion = propertyCompletionSession();
  const pageReads = coalescedRead<import("../owned").Owned<PageEntry[]>>();
  let timer: ReturnType<typeof setTimeout> | undefined;
  let pluginInvocation = 0;
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
    latestOwner(autocompleteScope, "suggestions"); pageReads.cancel();
    setAc(null); setAcBlockState(null); setAcItems([]); setAcIndex(0);
    propertyCompletion.end();
    props.closed();
  };
  const sameAcTrigger = (left: Trigger | null, right: Trigger) => left !== null
    && left.kind === right.kind && left.query === right.query && left.start === right.start
    && left.end === right.end && left.property === right.property
    && (left.propertyValues ?? []).join("\0") === (right.propertyValues ?? []).join("\0");
  const detectEditorTrigger = () => {
    if (props.suppressTriggers()) return null;
    const trigger = detectTrigger(text(), caret(), props.format());
    if (trigger) return trigger;
    // The explicit scaffold picker also accepts an empty query on its opener.
    const open = ac();
    return props.keepEmptyLanguagePicker && open?.kind === "code-language" && caret() === open.start
      && codeFences(text(), props.format()).some(f => f.delimEnd === caret() && caret() < f.openEnd)
      ? { ...open, query: "", end: caret() } : null;
  };
  const propertyValueItems = (key: string, query: string, used: readonly string[] = []): AcItem[] => {
    const values = propertyCompletion.values(key);
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
      let keys: string[] | null;
      try {
        keys = await propertyCompletion.keys();
      } catch (error) {
        // Completion is an optional aid: a transient facet-query failure must
        // never reject the editor input event or pile up global error toasts.
        dbg(`property-autocomplete: ${String(error)}`);
        if (requestOwner()) setAcItems([]);
        return;
      }
      if (!keys || !requestOwner()) return;
      const q = t.query.trim();
      const ranked = keys
        .map((key, index) => ({ key, index, score: q ? fuzzyScore(q, key) : 1 }))
        .filter(({ score }) => score > 0)
        .sort((left, right) => right.score - left.score || left.index - right.index)
        .slice(0, 100)
        .map(({ key }) => ({ label: key, propertyName: key }));
      const created = propertyKeyFold(t.query);
      if (created && !keys.includes(created)) {
        ranked.unshift({ get label() { return translate("editor-block-create", { name: String(created) }); }, propertyName: created });
      }
      setAcItems(ranked);
      return;
    }
    if (t.kind === "property-value") {
      setAcItems(propertyValueItems(t.property!, t.query, t.propertyValues));
      void propertyCompletion.load(t.property!).then((loaded) => {
        if (loaded && requestOwner()) setAcItems(propertyValueItems(t.property!, t.query, t.propertyValues));
      });
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
      setAcItems(blockPickItems(result.value, props.id));
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
    if (!props.quickSwitch && !completionLease) {
      completionLease = { ...editor, session: crypto.randomUUID() };
    }
    const lease = completionLease;
    // Closing can overtake a blocking native query. Release again after its
    // reply (including failure) so it cannot retain obsolete baseline keys.
    let result: import("../owned").Owned<PageEntry[]> | undefined;
    try {
      result = await pageReads.run(requestOwner, () => readOwned(requestOwner,
        props.quickSwitch ? props.quickSwitch(t.query, 100) : backend().quickSwitch(t.query, 100, lease ?? editor, pinyinMatchingEnabled())));
    } finally {
      if (lease && lease !== completionLease) releaseLease(lease);
    }
    if (!result || result.kind === "stale") return;
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

  const replaceTrigger = (insert: string, caretOffset?: number) => {
    const trigger = ac(); if (!trigger || !current()) return;
    const result = applyCompletion(text(), trigger.start, refCompletionEnd(text(), trigger.end, insert), insert, caretOffset);
    const spaced = caretOffset === undefined
      ? withRefCompletionSpace(result.raw, result.caret, insert, spaceAfterRefCompletion()) : result;
    // Keep the raw view until the completion edit is committed, then map its
    // source caret into the parser-owned code/calc body.
    props.replace(spaced.raw, spaced.caret, closeAc);
  };
  const openPlanning = (kind: "scheduled" | "deadline") => {
    const rect = props.element().getBoundingClientRect();
    openDatePicker(props.id, kind, rect.left, rect.bottom + 4, props.resultTarget());
  };
  const selectAc = (item: AcItem) => {
    const t = ac(); if (!t || !current() || props.composing()) return;
    if (item.propertyName) {
      const key = propertyKeyFold(item.propertyName);
      const result = applyCompletion(text(), t.start, t.end, `${key}:: `);
      apply(result.raw, result.caret);
      const valueTrigger: Trigger = { kind: "property-value", query: "", start: result.caret, end: result.caret, property: key };
      setAc(valueTrigger);
      setAcIndex(0); setAcItems(propertyValueItems(key, ""));
      void propertyCompletion.load(key).then((loaded) => {
        if (loaded && current() && sameAcTrigger(ac(), valueTrigger)) setAcItems(propertyValueItems(key, ""));
      });
      return;
    }
    if (item.propertyValue !== undefined) { replaceTrigger(item.propertyValue); return; }
    if (item.blockRef) {
      const owner = props.refOwner?.() ?? graphOwner(current), trigger = ac(), before = text();
      acceptBlockRefPick(item.blockRef, {
        intended: () => owner() && ac() === trigger && text() === before, current: () => owner(),
        triggerOpen: () => ac() === trigger, sourcePage: () => node(props.id)?.page, insert: (ref) => replaceTrigger(ref),
      });
      return;
    }
    if (item.plugin) {
      const before = text(), selection = props.selection();
      const n = node(props.id); let depth = 0, parent = n.parent;
      while (parent && node(parent) && depth < 1_000) { depth++; parent = node(parent).parent; }
      const snapshot = bindPluginBlockSnapshot({ id: n.id, raw: n.raw, parentId: n.parent, depth, format: props.format() });
      if (!snapshot) return;
      const token = ++pluginInvocation, ownerId = editingOwner(), surface = editingSurface(), block = editingId();
      const plugin = item.plugin;
      const intended = () => {
        if (!current() || token !== pluginInvocation || !isPluginGraphOwnerCurrent(snapshot.owner)
          || editingId() !== block || editingOwner() !== ownerId || editingSurface() !== surface
          || text() !== before || node(props.id).raw !== snapshot.block.raw) return false;
        const s = props.selection();
        return s[0] === selection[0] && s[1] === selection[1]
          && sameAcTrigger(detectTrigger(text(), caret(), props.format()), t);
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
        apply(result.raw, result.caret, result.caret, true, () => current() && token === pluginInvocation
          && isPluginGraphOwnerCurrent(snapshot.owner) && editingId() === block
          && editingOwner() === ownerId && editingSurface() === surface && text() === result.raw);
      }).catch(error => pushToast(msg("editor-block-plugin-slash-command-failed", { detail: String(error) }), "error"));
      return;
    }
    if (item.templateNodes) {
      // One gesture: the trigger's removal and the template's outline are written together, once the
      // kernel has answered; a refusal or a parser failure leaves the trigger in the editor and the
      // block (og-enter-family-1109, HONP4). A block the removal leaves empty is replaced by the template.
      const nodes = item.templateNodes.map(n => templateToOutline(n, node(props.id).page));
      closeAc();
      const tooDeep = msg("editor-block-outline-is-too-deep-to-insert");
      const last = insertEditorOutline(props.id, nodes, props.stageDelete(t.start, t.end), `template:${props.id}`, { tooDeep, refused: tooDeep });
      if (last && node(last)) startEditing(last, node(last).raw.length);
      return;
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
        props.afterEdit(() => {
          const raw = props.visible();
          apply(raw, Math.min(raw.length, removed.caret + (state === true ? 0 : state + 1)));
        }); return;
      }
      case "scheduled": case "deadline":
        replaceTrigger(""); openPlanning(item.action); return;
      case "now-time": replaceTrigger(timeStamp()); return;
      case "youtube-timestamp": replaceTrigger(youtubeTimestampMacroFor(props.element()) ?? ""); return;
      case "page-reference":
        replaceTrigger("[[]]", 2); props.afterEdit(() => void updateAutocomplete()); return;
      case "insert-link": {
        const removed = applyCompletion(text(), t.start, t.end, "", 0);
        const edit = insertLink(removed.raw, t.start, t.start, props.format());
        closeAc(); apply(edit.text, edit.start, edit.end); return;
      }
      case "query-builder":
        apply(applyCompletion(text(), t.start, t.end, QUERY_MACRO_SCAFFOLD).raw, t.start, t.start, false);
        closeAc(); setQueryBuilderAutoOpen(props.id); endEdit("query-builder"); return;
      case "code-block": {
        const result = applyCompletion(text(), t.start, t.end, "```\n\n```", 3);
        openLanguagePicker(result.raw, result.caret); return;
      }
      case "page-props": {
        replaceTrigger(""); const rect = props.element().getBoundingClientRect();
        openPageProps(node(props.id).page, rect.left, rect.bottom + 4); return;
      }
      case "sheet-grid": case "sheet-table": case "sheet-board":
        replaceTrigger(""); applySheetViewSlashAction(props.id, item.action === "sheet-grid" ? "grid" : item.action === "sheet-table" ? "table" : "board"); return;
      case "date-picker": replaceTrigger(""); props.openJournal(); return;
      case "tomorrow": case "yesterday": case "today": case "thatday":
        runJournalSlash(item.action, node(props.id).page, replaceTrigger); return;
      case "upload-asset": replaceTrigger(""); void props.assets.uploadAsset(); return;
      case "record": replaceTrigger(""); void props.assets.desktopVoiceMemoToggle(); return;
      case "drawio": replaceTrigger(""); void props.assets.createDrawioDiagram(); return;
      case "priority-a": case "priority-b": case "priority-c": {
        const lines = (text().slice(0, t.start) + text().slice(t.end)).split("\n");
        lines[0] = setPriority(lines[0], item.action === "priority-a" ? "A" : item.action === "priority-b" ? "B" : "C") + " ";
        closeAc(); apply(lines.join("\n"), lines[0].length); return;
      }
    }
    replaceTrigger(item.insert ?? "", item.caret);
  };

  const refresh = () => {
    if (!current() || props.composing()) return;
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
    const update = () => { void updateAutocomplete().catch(error => dbg(`autocomplete: ${String(error)}`)); };
    if (next.kind === "page" || next.kind === "tag") update();
    else timer = setTimeout(update, 90);
  };
  const selectionChanged = () => {
    if (!current() || props.composing()) return;
    const open = ac();
    const here = detectEditorTrigger();
    // Moving the caret never opens value suggestions (OG opens them only from
    // input): an arrow key or Enter on a property line must keep moving the
    // caret or splitting the block. An open value list follows the caret.
    if (open?.kind === "property-value") {
      if (props.selection()[0] !== props.selection()[1] || here?.kind !== "property-value") closeAc();
      else if (!sameAcTrigger(open, here)) refresh();
      return;
    }
    if (!open || !["page", "tag", "block"].includes(open.kind)) return;
    if (!here || here.kind !== open.kind || here.start !== open.start) closeAc();
  };
  createEffect(on(pinyinMatchingEnabled, () => {
    const trigger = untrack(ac);
    if (trigger?.kind === "page" || trigger?.kind === "tag") {
      if (!pinyinMatchingEnabled()) releaseCompletionLease();
      void updateAutocomplete();
    }
  }, { defer: true }));
  // The page/tag read is owned by `graphOwner`, which retires with the display
  // epoch (graph open, config or title-format change). An open list whose
  // answer the epoch outran would stay empty until the next keystroke: ask again.
  createEffect(on(graphEpoch, () => {
    const trigger = untrack(ac);
    if (trigger?.kind === "page" || trigger?.kind === "tag") void updateAutocomplete();
  }, { defer: true }));
  const openLanguagePicker = (raw: string, position: number, write = true) => {
    props.languagePicker(true);
    setAc({ kind: "code-language", query: "", start: position, end: position });
    setAcIndex(0);
    setAcBlockState(null);
    setAcItems(codeLanguageItems("").map(language => ({
      get label() { return language.label; }, sub: [language.id, ...language.aliases].join(" · "),
      insert: language.id, caret: language.id.length + 1,
    })));
    if (write) apply(raw, position);
  };
  onCleanup(() => {
    pluginInvocation++; clearTimeout(timer); latestOwner(autocompleteScope, "suggestions"); pageReads.cancel();
    releaseCompletionLease(); propertyCompletion.end();
  });
  return { ac, acItems, acBlockState, acIndex, setAcIndex, acVisible, closeAc, selectAc,
    updateAutocomplete, refresh, selectionChanged, openLanguagePicker,
    pause: () => { clearTimeout(timer); latestOwner(autocompleteScope, "suggestions"); pageReads.cancel(); } };
}
