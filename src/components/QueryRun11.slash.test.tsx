// GH #433 review run 11: the query editor's /Scheduled and /Deadline pickers
// are deferred controls like the result date chips; they write only while the
// edited row is still the result it was opened on.
import { afterEach, beforeAll, expect, it, vi } from 'vitest';
import { render } from 'solid-js/web';
import { initParser } from '../render/parse';
import { resetStore } from '../document';
import { doc, setDoc } from '../document/model';
import { adoptResults, beginResultsChange, captureResultsTicket } from '../queryResultsEpoch';
import { endEdit } from '../editorController';
import { LiveRefGroup } from './LiveRefGroup';
import { DatePicker } from './DatePicker';
import { closeDatePicker, datePicker } from '../ui';
import { setToasts } from '../toasts';
import type { BlockDto } from '../types';

beforeAll(initParser);
let dispose: (() => void) | undefined;
afterEach(() => { endEdit('external-reload'); dispose?.(); closeDatePicker(); resetStore(); setToasts([]); document.body.replaceChildren(); });
const raw = 'TODO Selected\nSCHEDULED: <2026-10-14 Wed>\nDEADLINE: <2026-10-14 Wed>';
async function mount() {
  setDoc({ byId: {
    ancestor: { id: 'ancestor', raw, collapsed: false, parent: null, page: 'Source', children: ['hit'] },
    hit: { id: 'hit', raw, collapsed: false, parent: 'ancestor', page: 'Source', children: [] },
  }, pages: [{ id: 'pages/Source.md', name: 'Source', kind: 'page', title: 'Source', preBlock: null, roots: ['ancestor'], format: 'md', readOnly: false, guide: false }], feed: ['Source'], loaded: true });
  const blocks: BlockDto[] = [{ id: 'hit', raw, collapsed: false, children: [] }];
  adoptResults(blocks, captureResultsTicket());
  const root = document.createElement('div'); document.body.append(root);
  dispose = render(() => <><LiveRefGroup page="Source" kind="page" path="pages/Source.md" blocks={blocks} surface="query" showBreadcrumb eager /><DatePicker /></>, root);
  await Promise.resolve();
  return root;
}
function done(root: HTMLElement) { [...root.querySelectorAll<HTMLButtonElement>('.dp-btn')].find(b => b.textContent === 'Done')!.click(); }
async function slashPicker(which: string) {
  const root = await mount();
  root.querySelector('[data-block-id="hit"] .block-content-wrapper')!.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
  document.dispatchEvent(new MouseEvent('mouseup', { bubbles: true, button: 0 }));
  await vi.waitFor(() => expect(root.querySelector('textarea')).not.toBeNull());
  const editor = root.querySelector<HTMLTextAreaElement>('textarea')!;
  editor.value = raw + '\n/' + which; editor.setSelectionRange(editor.value.length, editor.value.length);
  editor.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertText', data: which }));
  await vi.waitFor(() => expect([...document.querySelectorAll('.ac-label')].some(e => e.textContent === which)).toBe(true));
  const item = [...document.querySelectorAll('.ac-item')].find(e => e.querySelector('.ac-label')?.textContent === which)!;
  item.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0 }));
  expect(datePicker()?.blockId).toBe('hit');
  const targetCaptured = !!datePicker()?.resolveWriteTarget;
  root.querySelector<HTMLButtonElement>('[data-day="20"]')!.click();
  return { root, targetCaptured };
}
it.each(['Scheduled', 'Deadline'])('query editor slash %s refuses a same-ID result replacement', async which => {
  const { root, targetCaptured } = await slashPicker(which);
  const finish = beginResultsChange(); setDoc('byId', 'hit', { ...doc.byId.hit, raw: raw.replace('Selected', 'Replacement') }); finish(); await Promise.resolve();
  const before = doc.byId.hit.raw; done(root);
  expect(targetCaptured).toBe(true);
  expect(doc.byId.hit.raw).toBe(before); expect(datePicker()).not.toBeNull();
});
it.each(['Scheduled', 'Deadline'])('query editor slash %s still writes when the results did not change', async which => {
  const { root } = await slashPicker(which);
  done(root);
  expect(doc.byId.hit.raw).toContain(`${which.toUpperCase()}: <2026-10-20 Tue>`); expect(datePicker()).toBeNull();
});
