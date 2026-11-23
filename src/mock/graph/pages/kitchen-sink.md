- Blockquote:
  > a quoted line
  > a second quoted line
- Callout NOTE:
  > [!NOTE] Heads up
  > body of the note
- Callout WARNING:
  > [!WARNING] Be careful here
- Callout TIP:
  > [!TIP] a helpful tip
- Horizontal rule below:
  ---
- Table with alignment:
  | Left | Center | Right |
  |:---|:---:|---:|
  | a | b | c |
  | 1 | 2 | 3 |
- Autolink (bare): visit https://logseq.com/docs for details
- Autolink (angle): <https://example.com>
- Inline math: $E = mc^2$ and chemistry: $\ce{H2O + CO2}$
- Display math:
  $$\int_0^1 x^2 \, dx = \tfrac13$$
- DONE finished task with a logbook drawer
  :LOGBOOK:
  CLOCK: [2026-06-16 Tue 09:00:00]--[2026-06-16 Tue 09:30:45] =>  00:30:45
  CLOCK: [2026-06-17 Wed 10:00:00]--[2026-06-17 Wed 10:20:00] =>  00:20:00
  :END:
- Task markers: TODO a, DOING b, NOW c, LATER d, WAIT e, DONE f
- in-block checklist (+ list inside one bullet — ticks in OG/mobile):
  + [ ] pack toothbrush
  + [x] pack charger
  + [ ] pack passport
- in-block nested list:
  + groceries
    + milk
    + eggs
  + hardware
- Numbered list (logseq.order-list-type — the block itself is numbered):
	- Bump the version
	  logseq.order-list-type:: number
	- Tag and push
	  logseq.order-list-type:: number
		- run the test suite
		  logseq.order-list-type:: number
		- build the installers
		  logseq.order-list-type:: number
	- Announce on Discord
	  logseq.order-list-type:: number
- TODO [#A] high-priority task
- Inline styles: **bold**, *italic*, ~~strike~~, ==highlight==, `code`
- Video asset (plays inline where the codec is supported; otherwise a click-to-open chip):
  ![](../assets/demo_clip.mp4)
- Audio asset (⇔ Widen stretches the seek bar for precise scrubbing):
  ![](../assets/voice_memo.wav)
- Footnote reference[^1] in a sentence.
  [^1]: the footnote definition.
- Video embed: {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}
- Tweet embed: {{tweet https://twitter.com/logseq/status/123}}
- More embeds: {{twitter https://twitter.com/logseq/status/9}} · {{vimeo 76979871}} · {{bilibili BV1xx411c7mD}}
- youtube-timestamp {{youtube-timestamp 125}} · cloze {{cloze the answer\\the cue}} · zotero {{zotero-imported-file abc, paper.pdf}}
- User macro (config.edn :macros): {{poem red, blue}} and {{hi Martin, kitchen-sink}}
- Block user macro:
  {{card Topic, Body text}}
- Block reference (bare): see ((64b9c0e2-0000-0000-0000-000000000000)) inline
- Labeled block reference: see [Related Work](((64b9c0e2-0000-0000-0000-000000000000))) inline
- Project notes
	- Methods
		- a nested reference: see ((64b9c0e2-0000-0000-0000-000000000000)) here
- Block-ref target: the **Related Work** section
  id:: 64b9c0e2-0000-0000-0000-000000000000
