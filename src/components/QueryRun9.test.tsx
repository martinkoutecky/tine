// Queries run-8 review (2026-10-14): result provenance for List planning pickers and
// Search workspace task checkboxes; D-18 Convert with surrounding whitespace.
// Fail-before: 6 of 9 failed on f3265afa2 (evidence/r2-batch/queries/review-queries-run8.md).
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { initParser } from '../render/parse';
import { applyGraphChange, resetStore } from '../document';
import { doc, setDoc } from '../document/model';
import { backend } from '../backend';
import { adoptResults, beginResultsChange, captureResultsTicket } from '../queryResultsEpoch';
import { LiveRefGroup } from './LiveRefGroup';
import { SheetTable } from './SheetTable';
import { QueryResultTask } from './QueryResultTask';
import { DatePicker } from './DatePicker';
import { QueryWorkspace, type QueryWorkspaceDependencies } from './QueryWorkspace';
import { closeDatePicker, datePicker } from '../ui';
import { setToasts } from '../toasts';
import type { PaneRouter, QueryRoute } from '../router';
import type { BlockDto, QueryExecution } from '../types';
import type { Query } from '../editor/queryIr';

beforeAll(initParser);
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); dispose = undefined; closeDatePicker(); resetStore(); setToasts([]); vi.restoreAllMocks(); document.body.replaceChildren(); });
const hit = (raw: string): BlockDto => ({ id: 'same', raw, marker: 'TODO', scheduled: '2026-10-14 Wed', collapsed: false, children: [] });
async function fixture(raw: string) {
  setDoc({ byId: { same: { id: 'same', raw, collapsed: false, parent: null, page: 'Source', children: [] } }, pages: [{ id: 'pages/Source.md', name: 'Source', kind: 'page', title: 'Source', preBlock: null, roots: ['same'], format: 'md', readOnly: false, guide: false }], feed: ['Source'], loaded: true });
  await Promise.resolve();
}
function mount(view: () => import('solid-js').JSX.Element) { const root = document.createElement('div'); document.body.append(root); dispose = render(view, root); return root; }

it('List query planning popup refuses replacement after the live result unmounts', async () => {
  await fixture('TODO Selected\nSCHEDULED: <2026-10-14 Wed>');
  const blocks = [hit(doc.byId.same.raw)]; adoptResults(blocks, captureResultsTicket());
  const root = mount(() => <><LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks} surface="query" eager /><DatePicker /></>);
  await vi.waitFor(() => expect(root.querySelector('.date-chip.scheduled')).not.toBeNull());
  (root.querySelector('.date-chip.scheduled') as HTMLElement).click();
  expect(datePicker()).not.toBeNull();
  (root.querySelector('[data-day="20"]') as HTMLButtonElement).click();
  const finish = beginResultsChange();
  setDoc('byId', 'same', { ...doc.byId.same, raw: 'TODO Replacement\nSCHEDULED: <2026-10-14 Wed>' });
  finish(); await Promise.resolve();
  const before = doc.byId.same.raw;
  [...root.querySelectorAll<HTMLButtonElement>('.dp-btn')].find(b => b.textContent === 'Done')!.click();
  expect(doc.byId.same.raw).toBe(before);
  expect(datePicker()).not.toBeNull();
});

it.each(['search', 'list', 'table', 'board'] as const)('Search workspace %s task refuses replacement arriving during hydration', async presentation => {
  const selected = hit('TODO Selected');
  const execution: QueryExecution = { hits: [{ entity: 'block', page: 'Source', kind: 'page', path: 'pages/Source.md', block: selected, display_text: 'Selected', evidence: [], match_class: 'exact' }], diagnostics: [], explanation: { branches: [] }, cancelled: false };
  const deps = {
    runGraphSearch: vi.fn(async () => structuredClone(execution)), serializeDsl: vi.fn(), resolvePage: vi.fn(), createPage: vi.fn(), parseQuery: vi.fn(), queryRun: vi.fn(), queryExplainEmpty: vi.fn()
  } as unknown as QueryWorkspaceDependencies;
  const route: QueryRoute = { kind: 'query', id: 'review-task', sourceKind: 'search', source: 'Selected', presentation };
  const router = { route: () => route, updateActiveQuery: vi.fn(), openInNewTab: vi.fn(), replaceActiveRoute: vi.fn() } as unknown as PaneRouter;
  vi.spyOn(backend(), 'getPageByPath').mockImplementation(async () => {
    await applyGraphChange({ name: 'Unloaded', kind: 'page', created: false, removed: false });
    return { id: 'pages/Source.md', name: 'Source', title: 'Source', kind: 'page', format: 'md', pre_block: null, blocks: [hit('TODO Replacement')] };
  });
  const root = mount(() => <QueryWorkspace route={route} router={router} deps={deps} />);
  await vi.waitFor(() => expect(root.querySelector('[role="checkbox"]')).not.toBeNull());
  root.querySelector('[role="checkbox"]')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(doc.byId.same).toBeDefined());
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(doc.byId.same.raw).toBe('TODO Replacement');
});

it('control: Table planning popup refuses the same replacement', async () => {
  await fixture('TODO Selected\nSCHEDULED: <2026-10-14 Wed>');
  setDoc('byId', 'query', { id: 'query', raw: '{{query (task TODO)}}', page: 'Source', parent: null, collapsed: false, children: [] });
  await Promise.resolve();
  const groups = [{ page: 'Source', kind: 'page' as const, path: 'pages/Source.md', blocks: [hit(doc.byId.same.raw)] }];
  adoptResults(groups, captureResultsTicket());
  const root = mount(() => <><SheetTable ownerId="query" rowSource="query" groups={groups} queryDisplay={{ view: { columns: ['scheduled'] }, apply: () => {} }} /><DatePicker /></>);
  (root.querySelector('.date-chip.scheduled') as HTMLElement).click();
  (root.querySelector('[data-day="20"]') as HTMLButtonElement).click();
  const finish = beginResultsChange();
  setDoc('byId', 'same', { ...doc.byId.same, raw: 'TODO Replacement\nSCHEDULED: <2026-10-14 Wed>' });
  finish(); await Promise.resolve();
  const before = doc.byId.same.raw;
  [...root.querySelectorAll<HTMLButtonElement>('.dp-btn')].find(b => b.textContent === 'Done')!.click();
  expect(doc.byId.same.raw).toBe(before);
  expect(datePicker()).not.toBeNull();
});

it('control: a task given result provenance refuses the same hydration replacement', async () => {
  const selected = hit('TODO Selected'); adoptResults(selected, captureResultsTicket());
  vi.spyOn(backend(), 'getPageByPath').mockImplementation(async () => {
    await applyGraphChange({ name: 'Unloaded', kind: 'page', created: false, removed: false });
    return { id: 'pages/Source.md', name: 'Source', title: 'Source', kind: 'page', format: 'md', pre_block: null, blocks: [hit('TODO Replacement')] };
  });
  const root = mount(() => <QueryResultTask row={{ id: 'same', page: 'Source', kind: 'page', path: 'pages/Source.md', dto: selected }} />);
  root.querySelector('[role="checkbox"]')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(doc.byId.same).toBeDefined());
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(doc.byId.same.raw).toBe('TODO Replacement');
});

it.each(['', ' '])('D-18 Convert and Undo are available with surrounding whitespace %j', async padding => {
  const source = `${padding}(between scheduled today +7d)${padding}`;
  const typed = "@block scheduled between today and '+7d'";
  const query: Query = { anchor: 'block', filter: { kind: 'true' }, source: { kind: 'builder' }, diagnostics: [] };
  const deps = {
    parseQuery: vi.fn(async (_text, dialect) => dialect === 'macro_tql' ? { query, view: {} } : {
      query: { ...query, diagnostics: [{ kind: 'syntax', message: 'date window refusal' }] }, view: {},
      og_hint: { bare: [], no_conditions: false, date_windows: true, rewrites: [{ rewrite: 'convert', query }] }
    }),
    queryRun: vi.fn(async q => ({ anchor: 'block', groups: [], diagnostics: q.diagnostics, total: 0, exceeded: false, report: { ran: [], ignored: [], supported: true } })),
    runGraphSearch: vi.fn(), serializeDsl: vi.fn(), resolvePage: vi.fn(), createPage: vi.fn(), queryExplainEmpty: vi.fn()
  } as unknown as QueryWorkspaceDependencies;
  vi.spyOn(backend(), 'printQuery').mockResolvedValue(typed);
  const route: QueryRoute = { kind: 'query', id: 'review-convert', sourceKind: 'dsl', source, presentation: 'list' };
  const router = { route: () => route, updateActiveQuery: vi.fn(), openInNewTab: vi.fn(), replaceActiveRoute: vi.fn() } as unknown as PaneRouter;
  const root = mount(() => <QueryWorkspace route={route} router={router} deps={deps} />);
  await vi.waitFor(() => expect(deps.queryRun).toHaveBeenCalled());
  await new Promise(resolve => setTimeout(resolve, 0));
  expect(root.querySelector('[data-rewrite="convert"]')).not.toBeNull();
  expect(root.querySelector('.query-workspace-diagnostics')).toBeNull();
  expect(root.querySelector('.query-workspace-status')?.textContent).not.toContain('0 results');
  (root.querySelector('[data-rewrite="convert"]') as HTMLButtonElement).click();
  await vi.waitFor(() => expect(deps.parseQuery).toHaveBeenCalledWith(typed, 'macro_tql'));
  (root.querySelector('.query-conversion-undo') as HTMLButtonElement).click();
  await vi.waitFor(() => expect(root.querySelector('[data-rewrite="convert"]')).not.toBeNull());
  expect((root.querySelector('.query-workspace-source') as HTMLInputElement).value).toBe(source);
  expect(deps.createPage).not.toHaveBeenCalled();
});
