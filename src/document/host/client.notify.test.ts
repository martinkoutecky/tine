// UI-QUERY-STUCK-LOADING-AFTER-EDIT (review r1 of the fix): the user outcome (a query answer
// dropped during a close is replaced) and the notification invariant on unpin and re-entry.
import { afterEach, expect, it, vi } from 'vitest';
import { createMemo, createRoot, createSignal } from 'solid-js';
import { backend } from '../../backend';
import { endEdit, startEditing } from '../../editorController';
import { resetStore } from '..';
import { loadRoutedPage } from '../workingSet';
import { bindTestHost } from './wiring.test.support';
import { createMembershipResource } from '../../resultMembership';
import { resultsSettled } from '../../queryResultsEpoch';
import { applied, mail, mailPage, opened, page, setup, tick } from './fakes.test.support';

afterEach(() => { endEdit('blur'); resetStore(); vi.restoreAllMocks(); });

it('the dropped query answer is replaced after the close is admitted', async () => {
  loadRoutedPage({ ...page('P', 'original', 'r1'), id: 'pages/P.md' });
  await bindTestHost();
  let admitClose!: () => void;
  const realClose = backend().pageClose.bind(backend());
  const close = vi.spyOn(backend(), 'pageClose').mockImplementation((...args) => new Promise(resolve => {
    admitClose = () => { void realClose(...args).then(resolve); };
  }));
  startEditing('P-1');
  await vi.waitFor(() => expect(resultsSettled()).toBe(true));
  let finishRead!: (value: string[]) => void;
  const load = vi.fn().mockImplementationOnce(() => new Promise<string[]>(resolve => { finishRead = resolve; }))
    .mockResolvedValue(['matching result']);
  await createRoot(async dispose => {
    try {
      const [value] = createMembershipResource(() => 'query', () => 'query', load, () => false, () => true, true);
      expect(load).toHaveBeenCalledOnce();
      endEdit('blur');
      await vi.waitFor(() => expect(close).toHaveBeenCalledOnce());
      finishRead(['obsolete result']);
      await Promise.resolve(); await Promise.resolve();
      admitClose();
      await vi.waitFor(() => expect(value()).toEqual(['matching result']));
      expect(load).toHaveBeenCalledTimes(2);
    } finally { dispose(); }
  });
});

it('unpinning held content while another acquisition remains notifies readers', async () => {
  const ctx = await setup();
  const release = await opened(ctx, 'P', 3);
  const unpin = ctx.client.acquire('P', { pin: true });
  createRoot(dispose => {
    try {
      const [revision, setRevision] = createSignal(0);
      ctx.client.onChange = () => setRevision(n => n + 1);
      const observed = createMemo(() => { revision(); return ctx.client.holdsContent('P'); });
      ctx.client.receive(mail(7, 'pages/P.md', mailPage(5, { kind: 'page', dto: page('P', 'external', 'r5') })));
      expect(observed()).toBe(true);
      unpin();
      expect(ctx.doc.text('P')).toBe('external');
      expect(ctx.client.holdsContent('P')).toBe(false);
      expect(observed()).toBe(false);
    } finally { dispose(); ctx.client.onChange = () => {}; release(); }
  });
});

it('sending callbacks tolerate acquire/release, sendNow, discard, and move re-entry', async () => {
  const ctx = await setup();
  ctx.doc.load(page('P', 'a', 'r3'));
  ctx.doc.load(page('Q', 'b', 'r3'));
  const { client, host } = ctx;
  let callbacks = 0;
  const nested: Promise<unknown>[] = [];
  client.onChange = () => {
    if (client['pages'].get('P')?.phase.kind !== 'sending') return;
    expect(++callbacks).toBeLessThan(10);
    expect(client.busy('P')).toBe(true);
    const temporary = client.acquire('P');
    temporary();
    client.sendNow('P');
    nested.push(client.move('P', 'Q', page('P'), page('Q'), ['move-blocks']));
    nested.push(client.discard('P'));
  };
  const releaseP = await opened(ctx, 'P', 3);
  const releaseQ = await opened(ctx, 'Q', 3);
  // Discards queued by callbacks await idle; keep this probe bounded to synchronous re-entry.
  client.onChange = () => {
    if (client['pages'].get('P')?.phase.kind !== 'sending') return;
    expect(++callbacks).toBeLessThan(10);
    const temporary = client.acquire('P'); temporary();
    client.sendNow('P');
    nested.push(client.move('P', 'Q', page('P'), page('Q'), ['move-blocks']));
  };
  await tick();
  // Complete the one discard queued once the Open had answered.
  if ((client['pages'].get('P')?.phase as { request?: string } | undefined)?.request === 'discard') {
    client.receive(mail(7, 'pages/P.md', mailPage(3, { kind: 'unchanged' }), applied(host.last('discard').id, 3)));
  }
  await tick();
  host.autoAdmit = false;
  void client.move('P', 'Q', page('P', 'moved'), page('Q', 'received'), ['move-blocks']);
  expect(host.count('move')).toBe(1);
  expect(client.busy('P')).toBe(true);
  expect(client.busy('Q')).toBe(true);
  client.onChange = () => {};
  client.retire();
  await Promise.all(nested);
  releaseP(); releaseQ();
});
