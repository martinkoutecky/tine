// Queries run-9 re-review (2026-10-14): planning pickers on a query result's live
// descendants answer to the result row; ordinary editing elsewhere is unchanged.
// Fail-before: 2 of 16 failed on cd24f74b3 (evidence/r2-batch/queries/review-queries-run9.md).
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { initParser } from '../render/parse';
import { resetStore } from '../document';
import { doc, setDoc } from '../document/model';
import { adoptResults, beginResultsChange, captureResultsTicket } from '../queryResultsEpoch';
import { LiveRefGroup } from './LiveRefGroup';
import { Block, SurfaceContext } from './Block';
import { DatePicker } from './DatePicker';
import { QueryWorkspace, type QueryWorkspaceDependencies } from './QueryWorkspace';
import { backend } from '../backend';
import { closeDatePicker, datePicker } from '../ui';
import { setToasts } from '../toasts';
import type { BlockDto, QueryExecution } from '../types';
import type { PaneRouter, QueryRoute } from '../router';

beforeAll(initParser);
let dispose: (() => void) | undefined;
afterEach(() => { dispose?.(); dispose = undefined; closeDatePicker(); resetStore(); setToasts([]); vi.restoreAllMocks(); document.body.replaceChildren(); });
const raw = 'TODO Selected\nSCHEDULED: <2026-10-14 Wed>\nDEADLINE: <2026-10-14 Wed>';
const hit = (id: string, text = raw, children: BlockDto[] = []): BlockDto => ({ id, raw: text, marker: 'TODO', scheduled: '2026-10-14 Wed', deadline: '2026-10-14 Wed', collapsed: false, children });
async function fixture(nested = false, kind: 'page' | 'journal' = 'page') {
  setDoc({ byId: {
    same: { id: 'same', raw, collapsed: false, parent: null, page: 'Source', children: nested ? ['child'] : [] },
    ...(nested ? { child: { id: 'child', raw, collapsed: false, parent: 'same', page: 'Source', children: [] } } : {}),
  }, pages: [{ id: 'pages/Source.md', name: 'Source', kind, title: 'Source', preBlock: null, roots: ['same'], format: 'md', readOnly: false, guide: false }], feed: ['Source'], loaded: true });
  await Promise.resolve();
}
function mount(view: () => import('solid-js').JSX.Element) { const root = document.createElement('div'); document.body.append(root); dispose = render(view, root); return root; }
async function open(root: HTMLElement, id: string, which: string) {
  await vi.waitFor(() => expect(root.querySelector(`[data-block-id="${id}"] .date-chip.${which}`)).not.toBeNull());
  (root.querySelector(`[data-block-id="${id}"] .date-chip.${which}`) as HTMLElement).click();
  expect(datePicker()?.blockId).toBe(id);
  expect(datePicker()?.which).toBe(which);
  (root.querySelector('[data-day="20"]') as HTMLButtonElement).click();
}
function done(root: HTMLElement) { [...root.querySelectorAll<HTMLButtonElement>('.dp-btn')].find(b => b.textContent === 'Done')!.click(); }

it.each(['scheduled', 'deadline'])('nested query descendant %s refuses replacement', async which => {
  await fixture(true);
  const blocks = [hit('same', raw, [hit('child')])]; adoptResults(blocks, captureResultsTicket());
  const root = mount(() => <><LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks} surface="query" eager /><DatePicker /></>);
  await open(root, 'child', which);
  const finish = beginResultsChange();
  setDoc('byId', 'child', { ...doc.byId.child, raw: raw.replace('Selected', 'Replacement') });
  finish(); await Promise.resolve();
  const before = doc.byId.child.raw;
  done(root);
  console.info(JSON.stringify({ journey: `nested ${which} replacement`, expected: before, observed: doc.byId.child.raw, pickerOpen: datePicker() !== null }));
  expect(doc.byId.child.raw, `${which} replacement must remain unchanged`).toBe(before);
  expect(datePicker()).not.toBeNull();
});

it.each(['scheduled', 'deadline'])('top-level query result %s refuses replacement', async which => {
  await fixture();
  const blocks = [hit('same')]; adoptResults(blocks, captureResultsTicket());
  const root = mount(() => <><LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks} surface="query" eager /><DatePicker /></>);
  await open(root, 'same', which);
  const finish = beginResultsChange();
  setDoc('byId', 'same', { ...doc.byId.same, raw: raw.replace('Selected', 'Replacement') });
  finish(); await Promise.resolve();
  const before = doc.byId.same.raw;
  done(root);
  expect(doc.byId.same.raw).toBe(before);
  expect(datePicker()).not.toBeNull();
});

it.each(['scheduled', 'deadline'])('nested query descendant %s still edits its live source', async which => {
  await fixture(true);
  const blocks = [hit('same', raw, [hit('child')])]; adoptResults(blocks, captureResultsTicket());
  const root = mount(() => <><LiveRefGroup page="Source" kind="page" blocks={blocks} surface="query" eager /><DatePicker /></>);
  await open(root, 'child', which); done(root);
  expect(doc.byId.child.raw).toContain(`${which.toUpperCase()}: <2026-10-20 Tue>`);
  expect(doc.byId.same.raw).toBe(raw);
  expect(datePicker()).toBeNull();
});

it.each(['query', 'ref', 'embed'] as const)('%s group keeps ordinary planning editing', async surface => {
  await fixture();
  const blocks = [hit('same')]; adoptResults(blocks, captureResultsTicket());
  const root = mount(() => <><LiveRefGroup page="Source" kind="page" blocks={blocks} surface={surface} eager /><DatePicker /></>);
  await open(root, 'same', 'scheduled'); done(root);
  expect(doc.byId.same.raw).toContain('SCHEDULED: <2026-10-20 Tue>');
  expect(datePicker()).toBeNull();
});

it.each(['main', 'sidebar', 'journal'])('%s ordinary block keeps planning editing', async surface => {
  await fixture(false, surface === 'journal' ? 'journal' : 'page');
  const root = mount(() => <><SurfaceContext.Provider value={surface}><Block id="same" /></SurfaceContext.Provider><DatePicker /></>);
  await open(root, 'same', 'deadline'); done(root);
  expect(doc.byId.same.raw).toContain('DEADLINE: <2026-10-20 Tue>');
  expect(datePicker()).toBeNull();
});

it.each(['search', 'list', 'table', 'board'] as const)('Search workspace %s task still edits selected source after hydration', async presentation => {
  const execution: QueryExecution = { hits: [{ entity: 'block', page: 'Source', kind: 'page', path: 'pages/Source.md', block: hit('same'), display_text: 'Selected', evidence: [], match_class: 'exact' }], diagnostics: [], explanation: { branches: [] }, cancelled: false };
  const deps = { runGraphSearch: vi.fn(async () => structuredClone(execution)), serializeDsl: vi.fn(), resolvePage: vi.fn(), createPage: vi.fn(), parseQuery: vi.fn(), queryRun: vi.fn(), queryExplainEmpty: vi.fn() } as unknown as QueryWorkspaceDependencies;
  const route: QueryRoute = { kind: 'query', id: 'review-task', sourceKind: 'search', source: 'Selected', presentation };
  const router = { route: () => route, updateActiveQuery: vi.fn(), openInNewTab: vi.fn(), replaceActiveRoute: vi.fn() } as unknown as PaneRouter;
  vi.spyOn(backend(), 'getPageByPath').mockResolvedValue({ id: 'pages/Source.md', name: 'Source', title: 'Source', kind: 'page', format: 'md', pre_block: null, blocks: [hit('same')] });
  const root = mount(() => <QueryWorkspace route={route} router={router} deps={deps} />);
  await vi.waitFor(() => expect(root.querySelector('[role="checkbox"]')).not.toBeNull());
  root.querySelector('[role="checkbox"]')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(doc.byId.same?.raw).toContain('DONE Selected'));
});
