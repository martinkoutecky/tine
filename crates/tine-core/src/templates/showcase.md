title:: Feature showcase
type:: reference
tags:: demo, showcase
alias:: Kitchen sink (features)
icon:: 🧪

- This page exercises every Logseq page-level feature Tine knows about, so you can see **exactly how each one renders** — and where we're still rough. It's ordinary Logseq Markdown; the same file opens unchanged in Logseq.
- # 1 — Inline text formatting
- **Bold**, *italic*, _also italic_, ~~strikethrough~~, `inline code`, ==highlight==, and ^^also highlight^^.
- A mix in one line: a **bold `code` span**, an *italic [[link]]*, and a ~~struck ==highlight==~~.
- Raw inline HTML renders (sanitized to a safe allowlist): <ins>inserted</ins>, <del>deleted</del>, <kbd>Ctrl</kbd> + <kbd>C</kbd>, <mark>marked</mark>, and <sup>superscript</sup> / <sub>subscript</sub>. Scripts and event handlers (`onerror=`, `<script>`) are stripped.
- *Parity note:* a tag glued to a word — `H<sub>2</sub>O`, `mc<sup>2</sup>` — stays **literal in Logseq too** (mldoc only starts inline HTML at a word boundary), and so do the Markdown carets `x^2^` / `H~2~O`. For a glued subscript, use math: $E = mc^2$, $H_2O$.
- # 2 — Links & references
- Page link: [[Welcome to Tine]] · tag: #demo · namespaced page: [[Features/Quick capture]].
- External link: [the Logseq docs](https://docs.logseq.com) · bare URL: https://logseq.com · autolinked email: <hello@example.com>.
- Labelled page link: [read the welcome]([[Welcome to Tine]]).
- This block is a reference target.
  id:: 00000000-0000-4000-8000-00000000feed
- Block reference to it: ((00000000-0000-4000-8000-00000000feed)).
- # 3 — Headings
- # Heading 1
- ## Heading 2
- ### Heading 3
- #### Heading 4
- ##### Heading 5
- ###### Heading 6
- # 4 — Lists & nesting
- Parent bullet
  - Child bullet
    - Grandchild bullet
  - Second child with a **formatted** tail
- Numbered list (logseq order):
  logseq.order-list-type:: number
  - First
  - Second
  - Third
- # 5 — Block content types
- > A blockquote. Logseq renders this with a left rule and muted text.
- Fenced code block with language highlighting:
- ```rust
  fn main() {
      let greeting = "hello from Tine";
      println!("{greeting}");
  }
  ```
- A Markdown table:
- | Feature | Logseq | Tine |
  | --- | --- | --- |
  | Tables | yes | ? |
  | Math | yes | ? |
- Horizontal rule follows:
- ---
- Inline math $E = mc^2$ and display math:
- $$\int_{-\infty}^{\infty} e^{-x^2}\,dx = \sqrt{\pi}$$
- An inline image (sized): ![capture window](../assets/quick-capture.png){:width 320}
- # 6 — Tasks, priorities, scheduling
- TODO A plain task marker
- DOING A task in progress
- DONE A finished task
- LATER Deferred (now-workflow marker)
- NOW Active (now-workflow marker)
- WAITING Blocked on someone else
- CANCELED Abandoned task
- TODO [#A] High-priority task
- TODO [#B] Medium-priority task
- TODO [#C] Low-priority task
- TODO A scheduled task
  SCHEDULED: <2026-07-10 Fri>
- TODO A task with a deadline
  DEADLINE: <2026-07-12 Sun>
- TODO A repeating task
  SCHEDULED: <2026-07-10 Fri .+1w>
- DONE A task with a logbook (time tracking)
  :LOGBOOK:
  CLOCK: [2026-07-01 Wed 10:00:00]--[2026-07-01 Wed 10:30:00] =>  00:30:00
  :END:
- # 7 — Block properties
- A block carrying its own properties.
  status:: in-progress
  owner:: [[Martin]]
  estimate:: 3
- # 8 — Embeds
- Page embed of another demo page:
- {{embed [[Features/Tips & shortcuts]]}}
- Block embed of the reference target above:
- {{embed ((00000000-0000-4000-8000-00000000feed))}}
- # 9 — Queries
- Simple query — every TODO task in this graph:
- {{query (task TODO)}}
- Simple query — blocks referencing a page:
- {{query [[Welcome to Tine]]}}
- Query by priority:
- {{query (priority A)}}
- Compound query (and / or / not) — TODO tasks that are also high priority:
- {{query (and (task TODO) (priority A))}}
- # 10 — Macros, renderers & callouts
- Video embed: {{video https://www.youtube.com/watch?v=dQw4w9WgXcQ}}
- Namespace macro: {{namespace Features}}
- A NOTE callout:
- #+BEGIN_NOTE
  Callouts (admonitions) group a highlighted aside. This is a NOTE.
  #+END_NOTE
- A footnote reference[^1] in a sentence.
- [^1]: And here is the footnote body.
- # Create one yourself
- 1. Make a scratch page and add one bullet per construct you want to test: links, tables, code, tasks, embeds, queries, and callouts.
- 2. Keep each example small and label it with a heading so it is easy to scan.
- 3. Add page properties such as `title::` or `icon::` at the top when the whole page needs metadata.
- 4. What you should see: each example renders live while staying ordinary Logseq Markdown.
