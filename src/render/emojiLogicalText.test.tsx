import { afterEach, describe, expect, it } from 'vitest';
import { render } from 'solid-js/web';
import { EmojiText } from '../render/emoji';
import { renderedTextCaret } from '../render/spans';
let dispose=()=>{};
afterEach(()=>{dispose();document.body.innerHTML='';delete document.documentElement.dataset.editableEmoji});
describe('independent emoji logical text and caret',()=>{
 for(const platform of ['windows','apple','safe-monochrome','android']){
  it(platform,()=>{
   document.documentElement.dataset.editableEmoji=platform;
   const root=document.createElement('div');document.body.append(root);
   const source='A 👩🏽‍💻 ❤️ © Z';
   dispose=render(()=><EmojiText text={source}/>,root);
   const result=renderedTextCaret(root,root,root.childNodes.length);
   expect(result.text).toBe(source);expect(result.caret).toBe(source.length);
  });
 }
});
