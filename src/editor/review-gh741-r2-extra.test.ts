import { beforeAll, expect, it } from 'vitest';
import { blockRegions, initParser } from '../render/parse';
import { BUILTIN_HIDE, HIDE_ALL, SHEET_CELL_HIDE, splitProps, joinProps, splitLogbook } from './properties';
import { propertyEditorSession } from './propertySession';
beforeAll(() => initParser());
const oldDrawer = ':LOGBOOK:\nCLOCK: [2026-10-20 Tue 08:00:00]\n:END:';
const newDrawer = ':LOGBOOK:\nCLOCK: [2026-10-21 Wed 09:00:00]\n:END:';
const id = '11111111-1111-4111-8111-111111111111';
it.each([BUILTIN_HIDE,HIDE_ALL,SHEET_CELL_HIDE])('Org transport with id and previous history', hide => {
 const raw = `plain\n:PROPERTIES:\n:id: ${id}\n:owner: Ada\n:END:\n${oldDrawer}`;
 const session = propertyEditorSession();
 const parts = session.split(raw,hide,'org');
 console.log('transport',JSON.stringify(parts));
 expect(parts.hidden).not.toContain(':PROPERTIES:');
 expect(splitLogbook(parts.hidden,'md').drawers).toBe(oldDrawer);
 let previous=raw;
 let visible=parts.visible;
 for(const ch of `\n${newDrawer} suffix`) {
  visible+=ch;
  const current=session.split(previous,hide,'org');
  previous=session.join(visible,current.hidden,previous,hide,'org');
  if(visible.includes(newDrawer)) expect(session.split(previous,hide,'org').visible).toBe(visible);
  expect(blockRegions(previous,'org').id?.value).toBe(id);
 }
 expect(previous).toContain(newDrawer+' suffix');
 expect(previous).not.toContain('2026-10-20');
});
it('replacement matrix checks accepted history, ownership and visible bytes', () => {
 const texts=['', 'new', '> quoted', '> quoted\n> more', '- list', '1. list','| a | b |','```\nunclosed','#+BEGIN_QUOTE\nunclosed',':PROPERTIES:',':PROPERTIES:\n:owner: Ada', ':PROPERTIES:\n:owner: Ada\n:END:',':OTHER:\ntext\n:END:', ':END:', '#+TITLE: thing','foo:: bar','foo:: bar\n> quoted', 'first\n\n:PROPERTIES:\n:owner: Ada\n:END:', 'first\n\nfoo:: bar', 'first\n\n> quote\nbody', 'first\n\n```\nunclosed', '\n', '\n\n', ' ', '  > quoted', '\ttext', '- item\n  foo:: bar', '#+BEGIN_SRC\ncode\n#+END_SRC', ':LOGBOOK:', ':LOGBOOK:\nCLOCK: [2026-10-21 Wed 09:00:00]', 'SCHEDULED: <2026-10-21 Wed>', 'SCHEDULED: <2026-10-21 Wed>\n:PROPERTIES:\n:owner: Ada\n:END:'];
 const failures: unknown[]=[];
 for(const format of ['md','org'] as const) {
  const props = format==='md'? `id:: ${id}\ncollapsed:: true` : `:PROPERTIES:\n:id: ${id}\n:collapsed: true\n:END:`;
  for(const raw of [`DOING x\n${props}\n${oldDrawer}\nbody`, `DOING x\n${oldDrawer}`,`${props}\n${oldDrawer}`, `DOING x\n${oldDrawer}\n${props}`]) {
   const parts=splitProps(raw,BUILTIN_HIDE,format);
   for(const text of texts) {
    try {
     const out=joinProps(text,parts.hidden,format,raw);
     const next=splitProps(out,BUILTIN_HIDE,format);
     if(!next.hidden.includes(oldDrawer)) failures.push({format,text,raw,out,problem:'missing history'});
     // Visible hidden properties intentionally typed by user can project away.
    } catch(error) { failures.push({format,text,raw,error:String(error)}); }
   }
  }
 }
 console.log('matrix failures',JSON.stringify(failures,null,2));
 expect(failures).toEqual([]);
});
it('hidden-property authoring matrix', () => {
 const failures:unknown[]=[];
 for(const format of ['md','org'] as const) {
  const wrap=(s:string)=>format==='md'?s:`:PROPERTIES:\n${s}\n:END:`;
  const line=(k:string,v:string)=>format==='md'?`${k}:: ${v}`:`:${k}: ${v}`;
  for(const key of ['id','collapsed','logseq.order-list-type','quote','author']) {
   const raw=`plain\n${wrap(line(key,key==='id'?id:'old'))}\n${oldDrawer}`;
   const parts=splitProps(raw,HIDE_ALL,format);
   for(const otherKey of [key,key.toUpperCase()]) {
    for(const title of ['plain\n','']) {
     const visible=title+wrap(line(otherKey,key==='id'?id:'new'));
     try { joinProps(visible,parts.hidden,format,raw); }
     catch(error) {failures.push({format,key,otherKey,visible,error:String(error)});}
    }
   }
  }
 }
 console.log('authoring failures',JSON.stringify(failures,null,2));
 expect(failures).toEqual([]);
});
