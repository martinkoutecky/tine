- # KITCHEN SINK — rendering parity net (every construct, labeled)
- Each bullet is one construct. The label before the colon names it; what follows is the live example. Screenshot this page in Tine vs Logseq OG and gaps jump out.
- ## HEADINGS
- # h1: Heading level 1
- ## h2: Heading level 2
- ### h3: Heading level 3
- #### h4: Heading level 4
- ##### h5: Heading level 5
- ###### h6: Heading level 6
- ## INLINE TEXT FORMATTING
- bold (asterisks): **bold text**
- bold (underscores): __bold text__
- italic (asterisk): *italic text*
- italic (underscore): _italic text_
- strikethrough: ~~struck text~~
- highlight: ==highlighted text==
- inline code: `inline code`
- nested emphasis: **bold with *italic* and `code` inside**
- ## LINKS & REFERENCES
- markdown link: [Logseq site](https://logseq.com)
- raw URL (bare): see https://example.com/raw-url for the bare-URL case
- autolink (angle brackets): <https://example.com/autolink>
- markdown link to PDF asset: [open the pdf](../assets/sample.pdf)
- page reference: link to [[Some Page]]
- tag (word): a #tagword here
- tag (bracketed): a #[[Tag With Spaces]] here
- block reference: see ((64b9c0e2-0000-0000-0000-000000000000))
- ## IMAGES
- image (external): ![alt text](https://www.gstatic.com/webp/gallery/1.png)
- image (asset): ![local asset](../assets/sample.png)
- sized image (Logseq width): ![sized](../assets/sample.png){:width 200}
- sized image (height+width): ![sized2](../assets/sample.png){:height 100 :width 150}
- ## BLOCKQUOTES & RULES
- blockquote (single line): > This is a blockquote.
- multi-line blockquote:
  > First quoted line.
  > Second quoted line.
  > Third quoted line.
- horizontal rule:
  ---
- ## CALLOUTS (markdown admonitions)
- callout NOTE:
  > [!NOTE]
  > This is a note callout.
- callout TIP:
  > [!TIP]
  > This is a tip callout.
- callout WARNING:
  > [!WARNING]
  > This is a warning callout.
- callout IMPORTANT:
  > [!IMPORTANT]
  > This is an important callout.
- callout CAUTION:
  > [!CAUTION]
  > This is a caution callout.
- ## CODE
- fenced code (with language):
  ```javascript
  function greet(name) {
    return `hello ${name}`;
  }
  ```
- fenced code (no language):
  ```
  plain preformatted text
    indented line
  ```
- ## MATH
- inline math: Euler's identity $e^{i\pi} + 1 = 0$ inline.
- display math:
  $$\int_0^\infty e^{-x^2}\,dx = \frac{\sqrt{\pi}}{2}$$
- mhchem (chemistry): water forms via $\ce{2H2 + O2 -> 2H2O}$
- ## LISTS
- ordered/numbered list (logseq.order-list-type): the children below should render 1. 2. 3.
  logseq.order-list-type:: number
	- first numbered item
	- second numbered item
	- third numbered item
- ## TASKS & PRIORITIES
- TODO task marker: TODO finish the parity audit
- DOING task marker: DOING writing the fixture
- DONE task marker: DONE read the renderer code
- checklist (GFM checkbox, NOT a task — tick it, no agenda):
	- [ ] pack toothbrush
	- [x] pack charger
	- [ ] pack passport
- NOW task marker: NOW focus block
- LATER task marker: LATER revisit
- WAITING task marker: WAITING on review
- CANCELED task marker: CANCELED dropped idea
- priority A: TODO [#A] high priority task
- priority B: TODO [#B] medium priority task
- priority C: TODO [#C] low priority task
- ## SCHEDULING (org timestamps)
- scheduled task: TODO call the bank
  SCHEDULED: <2026-06-20 Sat>
- deadline task: TODO submit the form
  DEADLINE: <2026-06-25 Thu 14:00>
- both scheduled and deadline: TODO big task
  SCHEDULED: <2026-06-20 Sat>
  DEADLINE: <2026-06-25 Thu>
- ## PROPERTIES
- single property block: this block has one property
  type:: example
- multiple properties: this block has several
  status:: draft
  author:: Martin
- multi-value property (comma list): tagged with two pages
  tags:: [[Alpha]], [[Beta]]
- ## DRAWERS
- LOGBOOK drawer (clock history): a DONE task carries a logbook
  :LOGBOOK:
  CLOCK: [2026-06-14 Sun 10:00:00]--[2026-06-14 Sun 10:30:00] =>  0:30
  :END:
- ## FOOTNOTES
- footnote reference and definition: here is a claim with a footnote[^1]
- footnote definition:
  [^1]: This is the footnote text.
- ## TABLES
- table with alignment (left / center / right):
  | Name  | Center | Right |
  | :---  | :----: |  ---: |
  | alpha |   1    |   100 |
  | beta  |   22   |     5 |
- ## LINE BREAKS
- hard line break (two trailing spaces): first line  
  second line after a hard break
- ## EMOJI
- emoji shortcode: party time :smile: and :rocket: and :+1:
- emoji literal (unicode): direct emoji 🎉 renders fine
- ## QUERIES & EMBEDS
- query macro (simple): {{query (task TODO)}}
- query macro (page): {{query [[Some Page]]}}
- query macro (advanced datalog): {{query (and (task TODO) (page "Some Page"))}}
- embed block: {{embed ((64b9c0e2-0000-0000-0000-000000000000))}}
- embed page: {{embed [[Some Page]]}}
- ## OTHER MACROS
- video macro: {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}
- tweet macro: {{tweet https://twitter.com/jack/status/20}}
- namespace macro: {{namespace Some Page}}
- renderer macro (generic): {{renderer :something arg1 arg2}}
- cloze macro: {{cloze hidden answer}}
- ## RAW HTML / EMBEDS
- raw inline HTML: a span <span style="color:red">in red</span> here
- raw HTML block: <div class="custom">raw div content</div>
- iframe embed: <iframe src="https://example.com" width="400" height="200"></iframe>
