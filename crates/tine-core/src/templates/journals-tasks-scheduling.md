icon:: 📅

- # Journals, tasks, and scheduling
  - Everything dated lives here: one journal page per day, tasks as plain-text markers at the start of a block, `[#A]`–`[#C]` priorities, and `SCHEDULED:`/`DEADLINE:` planning lines with calendar pickers. This page maps the shipped behavior; [[Workflows/Capture and plan your day]] walks through using it, and [[Feature showcase]] shows each construct rendered.
- ## Journals
  - The **Journals** entry in the left sidebar (or **g j**) opens a multi-day feed — one continuous, editable list, newest day first. Today's journal is always shown at the top of the feed. With no daily template, its file is created lazily on your first edit: the feed shows the day, but no file exists until you type.
  - "Today" follows the calendar of the machine's local time zone as Tine's backend reads it, so today's journal rolls over at local midnight — including when Tine stays open overnight or resumes from sleep — and the Agenda, carry-over and the **Go to date** calendar all use that same day.
  - **g n** and **g p** step to the next or previous calendar day from the journal you are viewing (from another page they start at today, and they open that day even if it has no file yet). The calendar button in the top bar (**Go to date**) opens a month grid — days that already have an entry are marked — to jump to any day's journal.
  - **Linked References** appears beneath each day in the Journals feed when other blocks link to that journal, with a count and the same filter, expand, and edit controls as an individually opened page. References load as you scroll to the day. Empty sections stay hidden; the graph’s linked-reference collapse threshold controls whether they start folded.
  - Settings → **Journals** → **Journal date format** controls the display titles and how new `[[date]]` titles are written (saved to `:journal/page-title-format`, traveling with the graph). The choices match Logseq’s built-in formats, including weekday and `yyyy年MM月dd日` titles. `E`, `EE`, and `EEE` show abbreviated weekdays (such as `Fri`); `EEEE` shows the full name (`Friday`). Graphs using the older `:date-formatter` setting are also supported: `:journal/page-title-format` takes precedence when both are present, and opening a graph keeps the legacy setting untouched. It renames no files: journal *file* names are untouched and existing journals keep working. If some journal files are named after their title instead of their date, their days look empty; Settings → **Backups & recovery** lists them under **Journal files named by title** with a **Rename to date names** button, which never runs on its own. For files, duplicate days, and the on-disk layout in general, see [[Reference/Files, external edits, and backups]].
  - Settings → **Journals** → **New-journal template** inserts a template into each new day (`:default-templates {:journals …}` in `config.edn`); “(none)” means blank days, the default. Make a template from a block's right-click menu (**Make a template…**).
- ## Task markers
  - The marker set is `TODO`, `DOING`, `NOW`, `LATER`, `WAITING`, `WAIT`, `STARTED`, `IN-PROGRESS`, `DONE`, `CANCELED`, `CANCELLED` — stored as the plain first word of the block's first line, like Logseq.
  - Settings → **Journals** → **Task workflow** chooses between two cycling lanes, TODO / DOING and NOW / LATER (`:preferred-workflow`, traveling with the graph); NOW / LATER is the default.
  - Click the marker chip for Logseq's two-state toggle: TODO ↔ DOING or LATER ↔ NOW (a chip click never clears `DONE`). **Ctrl+Enter** (Cmd on Mac) uses the full keyboard cycle TODO → DOING → DONE → none, or LATER → NOW → DONE → none, applied to every selected task when several blocks are selected.
  - Every open task also shows a checkbox in front of it: click to mark DONE, click again to reopen to the workflow's open marker (TODO or LATER). `DONE` shows a checked box; `CANCELED`/`CANCELLED` show none. The same live checkbox edits the source block inside query results, linked references, and embeds.
  - Queries filter on markers with a task clause, e.g. `(task TODO DOING)` in `{{query }}`, or as a **Task** condition in the visual query builder.
- ## Priorities
  - `[#A]`, `[#B]`, `[#C]` sit directly after the task marker; a block without a task marker can carry one too. Type slash, choose **Priority A** (or B, C), and the chip updates in place.
  - Queries can filter on priority with a `(priority A)` clause; the [[Feature showcase]] shows the constructs rendered.
- ## Scheduled and deadline dates
  - A block's plan is a `SCHEDULED:` and/or `DEADLINE:` line with an Org-style timestamp `<2026-07-07 Tue>`, optionally with a clock time `<2026-07-07 Tue 14:30>`. Convention: use SCHEDULED for the day you intend to work on something, DEADLINE for the day it must be done.
  - While editing, type / and choose **Scheduled** or **Deadline**. Click a day to select it, then optionally use **+ Add time** for `HH:mm` and the repeat menu (No repeat / Daily / Weekly / Monthly / Yearly, with **from completion**). **Done** or clicking outside applies the chosen date, time, and repeat together. **Escape** cancels all pending changes and returns to the editor at the same caret. The calendar takes keyboard focus: **Left/Right** move one day, **Up/Down** move one week, and **Enter** applies the focused day (or the chosen date when entering a time). Its first weekday follows Settings → **Journals** → **First day of week**. **Today** selects today; **Clear** removes an existing date. The block gains a calendar **date chip** — click it to edit the date, time, or repeat with the same Done/outside/Escape rules.
  - **/Date picker** opens the same calendar to insert a `[[date]]` link at the cursor in your graph's journal title format. Choose a day, then use **Done**, **Enter**, or click outside to insert it; **Escape** cancels. **/Today**, **/Tomorrow**, and **/Yesterday** insert the corresponding journal date directly; **/That day** inserts the containing journal's date.
  - You may also type a planning line anywhere in the block: on leaving the editor it moves to its canonical spot — right after the first line, before any properties, SCHEDULED before DEADLINE — like Logseq. A `SCHEDULED:` inside inline code or a code fence stays literal content and is never moved.
  - The same chips and pickers appear wherever the block does — in linked references, query results, and embeds.
- ## Agenda
  - Today's journal ends with a **Scheduled & Deadline** list of *open* items whose date falls inside the Agenda window — by default 7 days back and 7 ahead. Settings → **Journals** → **Agenda window** adjusts both on this device; `:scheduled/future-days` in `logseq/config.edn`, when set, controls days ahead for the whole graph instead. The list disappears when nothing matches. Set `:feature/disable-scheduled-and-deadline-query? true` to hide it.
  - `:default-queries {:journals [...]}` adds queries beneath today's blocks, before the agenda, in the order written. Each uses the same query renderer as a query block; a failing or unsupported query shows its own error while its neighbours remain usable. String titles and static text in hiccup vectors appear as plain text. `:collapsed? true` starts a query folded. Config edits apply live; malformed journal settings show a config diagnostic. Graph code is never executed; supported advanced query shapes match [[Reference/Queries]].
  - Add these entries inside the existing config map (use `false` to keep the built-in agenda):
    ```clojure
    :feature/disable-scheduled-and-deadline-query? false
    :scheduled/future-days 7
    :default-queries
    {:journals
     [{:title "Priority A tasks"
       :query (and (task TODO DOING) (priority A))
       :collapsed? false}
      {:title [:span "Open tasks"]
       :query [:find (pull ?b [*])
               :where [?b :block/marker "TODO"]]
       :inputs []
       :collapsed? true}]}
    ```
  - Keyed off the scheduled/deadline date itself, not the page's journal day, over the whole graph: an unfinished item on an old page still shows while its date is in range. The underlying query is `(and (or (between scheduled -7d +7d) (between deadline -7d +7d)) (not (task DONE CANCELED CANCELLED)))`.
  - Finished tasks (`DONE`, `CANCELED`, `CANCELLED`) never appear, matching Logseq.
- ## Time tracking and the logbook
  - With Settings → **Journals** → **Time tracking** on (the default), moving into DOING / NOW records a clock-in and moving back to TODO / LATER or into DONE records a clock-out, as OG-compatible `:LOGBOOK:` `CLOCK:` rows (seconds on by default via `:logbook/settings`). Saved to `:feature/enable-timetracking?`.
  - Blocks with clock rows show a small elapsed-time badge with a Type / Start / End / Span tooltip; the `:LOGBOOK:` drawer itself stays hidden by default, like Logseq.
  - Toggled off, markers change without writing log rows.
- ## Repeating tasks
  - Put a repeater inside the timestamp, e.g. `<2026-07-07 Tue +1w>` (or pick one in the calendar popup); the units are `d`, `w`, `m`, `y` for days, weeks, months, years.
  - Marking a repeating task done does not close it: the date advances to the next occurrence and the marker resets to the workflow's open state (TODO, or LATER under NOW / LATER). This works from the checkbox, the marker chip, and Ctrl+Enter.
  - A timed repeater such as `<2026-09-30 Wed 09:00 +1w>` advances to the next date while keeping `09:00`; the date picker keeps that time and repeater when you re-pick its date.
  - `+1w` advances from the stored date; `.+1w` counts from the day you complete it; `++1w` skips missed occurrences, landing past today. The repeater survives date re-picks in the calendar.
- ## Carry-over
  - On a past day, **Carry unfinished tasks → today** moves its open tasks (every marker except DONE / CANCELED / CANCELLED) to today. On today, **Carry from previous day** pulls from the most recent day that has content and **Carry last N days** goes further back. The same action is on a past journal day's right-click menu, and the command palette (**Ctrl+Shift+P**) has last-7 / 30 / 365-day presets.
  - Settings → **Journals** tunes it: **Show carry-over buttons**, **Carry-over keeps context** (on: move whole blocks that contain an open task; off: pull out just the task), **Carry-over header** (add a “Carried over” heading), and **Carry “last N days”**.
- ## Settings quick map
  - Settings → **Journals** collects every control above: **Journal date format**, **First day of week**, the carry-over controls, **Task workflow**, **Time tracking**, **New-journal template**, and **Agenda window**. For the ones in use, see [[Workflows/Capture and plan your day]].
