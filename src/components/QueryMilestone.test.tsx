import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { createSignal } from 'solid-js';
import { initParser } from '../render/parse';
import { resetStore, blockProperty } from '../document';
import { setDoc, doc } from '../document/model';
import { toggleTaskAtSource } from '../components/SourceTaskCheckbox';
import { collapseQueryResults } from '../components/QueryCollapse';
import { QueryResultsSelection } from '../components/QueryResultsSelection';
import { closeExportModal, exportModal } from '../ui';
import { backend } from '../backend';
import { QueryBuilder } from '../components/QueryBuilder';
import { computeSheetExport } from '../sheet/staticExport';
import type { QueryHit } from '../types';
import { toasts, setToasts } from '../toasts';
import { openQueryResultsCopy } from './queryResultsCopy';
import { samePagePropertyResult, resolveResultRow } from './queryResultIdentity';
beforeAll(async () => { await initParser(); });
afterEach(() => { resetStore(); closeExportModal(); setToasts([]); vi.restoreAllMocks(); document.body.replaceChildren(); });
function load(raw: string) {
  setDoc({ byId: { same: { id: 'same', raw, collapsed: false, parent: null, page: 'Source', children: ['child'] }, child: { id: 'child', raw: 'Child', collapsed: false, parent: 'same', page: 'Source', children: [] } },
    pages: [{ id: 'pages/Source.md', name: 'Source', kind: 'page', title: 'Source', preBlock: null, roots: ['same'], format: 'md', readOnly: false, guide: false }], feed: ['Source'], loaded: true });
}
it('does not toggle the matching locator of a different graph after the load await', async () => {
  load('TODO Original graph');
  const pending = toggleTaskAtSource({ uuid: 'same', page: 'Source', pageKind: 'page', path: 'pages/Source.md' }, false);
  resetStore(); load('TODO New graph');
  await pending;
  expect(doc.byId.same.raw).toBe('TODO New graph');
});
it('does not bulk-fold a replacement at a stale result runtime locator', async () => {
  const old = { id: 'same', raw: 'Original query match', collapsed: false, children: [] };
  load('Unrelated replacement after external insertion');
  await collapseQueryResults([{ page: 'Source', kind: 'page', path: 'pages/Source.md', blocks: [old] }], true);
  expect(blockProperty('same', 'collapsed')).toBeNull();
  expect(toasts().map(toast => toast.message)).toEqual(['1 results changed since the query ran; run it again']);
});
it('refuses the whole selection when positional provenance is unavailable', async () => {
  vi.spyOn(backend(), 'getPageByPath').mockResolvedValue({
    id: 'pages/Source.md', name: 'Source', title: 'Source', kind: 'page', pre_block: null,
    blocks: [
      { id: 'same', raw: 'Replacement', collapsed: false, children: [] },
      { id: 'kept', raw: 'Selected', collapsed: false, children: [{ id: 'c', raw: 'Child', collapsed: false, children: [] }] },
    ],
  });
  await openQueryResultsCopy([{ page: 'Source', kind: 'page', path: 'pages/Source.md', blocks: [
    { id: 'same', raw: 'Old match', collapsed: false, children: [] },
    { id: 'kept', raw: 'Selected', collapsed: false, children: [] },
  ] }]);
  expect(exportModal()).toBeNull();
  expect(JSON.stringify(exportModal())).not.toContain('Replacement');
  expect(toasts().map(toast => toast.message)).toEqual(['2 results changed since the query ran; run it again']);
});
it('retains authored identity across content edits and locator changes, in Markdown and Org', () => {
  const uuid = '4e719cb2-ec4b-4ff7-9fd9-8f3ae6dfffea';
  for (const format of ['md', 'org'] as const) {
    const property = format === 'md' ? `\nid:: ${uuid}` : `\n:PROPERTIES:\n:id: ${uuid}\n:END:`;
    const dto = { id: 'old', raw: 'Original' + property, collapsed: false, children: [] };
    const source = { name: 'Source', kind: 'page' as const, title: 'Source', pre_block: null, format, blocks: [{ ...dto, id: 'new', raw: 'Edited' + property }] };
    const row = { id: 'old', dto, page: 'Source', kind: 'page' as const };
    expect(resolveResultRow(row, 'action', source)?.id).toBe('new');
    expect(resolveResultRow(row, 'action', { ...source, blocks: [{ ...dto, raw: 'Original' }] })).toBeNull();
    expect(resolveResultRow({ ...row, dto: { ...dto, raw: 'Original' } }, 'action', source)).toBeNull();
  }
});
it('validates read-only page-property selections against the hydrated header', () => {
  const hit = { id: 'header', raw: 'related:: [[Target]]', collapsed: false, page_property: true, children: [] };
  expect(samePagePropertyResult(hit, 'related:: [[Target]]\n', 'md')).toBe(true);
  expect(samePagePropertyResult(hit, 'related:: [[Replacement]]', 'md')).toBe(false);
});
it('does not copy a replacement at a frozen Search selection runtime locator', async () => {
  const old: QueryHit[] = [{ entity: 'block', page: 'Source', kind: 'page', path: 'pages/Source.md', block: { id: 'same', raw: 'Original graph selection', collapsed: false, children: [] }, display_text: 'Original graph selection', evidence: [] }];
  const [hits, setHits] = createSignal(old);
  const root=document.createElement('div'); document.body.append(root);
  const dispose = render(() => <QueryResultsSelection hits={hits()} />, root);
  try {
    root.querySelector<HTMLButtonElement>('button')!.click();
    load('Unselected replacement after external insertion'); setHits([]);
    [...root.querySelectorAll<HTMLButtonElement>('button')].find(b => b.textContent === 'Copy results')!.click();
    await vi.waitFor(() => expect(toasts().map(toast => toast.message)).toEqual(['1 results changed since the query ran; run it again']));
    expect(exportModal()).toBeNull();
    expect(JSON.stringify(exportModal())).not.toContain('Unselected replacement after external insertion');
  } finally { dispose(); }
});
it.each(['scheduled', 'deadline'] as const)('reopens a %s comparison as its saved operator and value', async (attr) => {
  vi.spyOn(backend(), 'printQuery').mockResolvedValue('@block and scheduled > +5d');
  const root = document.createElement('div'); document.body.append(root);
  const query = { anchor: 'block' as const, filter: { kind: 'leaf' as const, leaf: { kind: 'attr' as const, attr, op: 'gt' as const, value: { kind: 'date' as const, literal: '+5d' } } }, source: { kind: 'builder' as const } };
  const dispose = render(() => <QueryBuilder sheetAlwaysOpen session={() => ({ query, view: {} })} onChange={() => {}} />, root);
  try {
    expect(root.querySelector('.qs-op')?.textContent).toContain('>');
    root.querySelector<HTMLButtonElement>('.qs-value')!.click();
    const picker = await vi.waitFor(() => { const s = document.querySelector<HTMLSelectElement>('[aria-label="Date comparison"]'); expect(s).not.toBeNull(); return s!; });
    expect({ editorOperator: picker.value, editorDate: document.querySelector<HTMLInputElement>('[aria-label="Date"]')?.value }).toEqual({ editorOperator: 'gt', editorDate: '+5d' });
  } finally { dispose(); }
});
it('keeps a case-insensitive explicit empty column choice in static export', () => {
  const row={id:'r',raw:'TODO Row',marker:'TODO',properties:[['owner','Someone']] as [string,string][],collapsed:false,children:[]};
  const owner={id:'q',raw:'{{query (task TODO)}}\nTINE.COLUMNS::',properties:[['tine.view','table'],['TINE.COLUMNS','']] as [string,string][],collapsed:false,children:[]};
  const out=computeSheetExport({page:'Source',path:[0],fp:'p',owner,rows:[],omitted:0,query:{fp:'q',presentation:'table',view:{columns:[]},pages:['Source'],rows:[row]}},{now:new Date('2026-10-14T12:00:00Z'),workflow:'todo'});
  expect(out?.view).toBe('table');
  expect(out && 'columns' in out ? out.columns.length : null).toBe(1);
});
