icon:: ✅

- # Capture and plan your day
  - The shortest path: open **Journals** in the sidebar and start typing. Everything below adds one tool at a time — a task marker, a priority, a date — and ends with the two things that keep a plan current: the Agenda and carry-over.
- ## Describe your day in bullets
  - 1. Click **Journals** in the sidebar — the feed opens with today first (today is the current day on this machine's local calendar, and it rolls over at local midnight). Click into today's first bullet and type what happened or what you mean to do.
  - 2. Press **Enter** for the next bullet, **Tab** / **Shift+Tab** to nest or flatten it.
  - 3. To put a bullet *above* an existing one, press **Enter** with the caret at its very start. Some bullets keep Enter for themselves — inside a code block it adds a line of code — so for those, right-click the bullet's dot and choose **Insert block above**.
  - 4. What you should see: the text saves to today's journal file on its own — there is no Save button. For where writes go and what happens if a file changes on disk, see [[Reference/Files, external edits, and backups]].
- ## Not at your desk? Quick capture
  - When a thought arrives while you are in another app, press your desktop quick-capture shortcut, type, and the note is appended to today's journal without switching windows. One-time setup, the optional page-title field, and the Enter-key tuning are all in [[Features/Quick capture]].
- ## Turn a bullet into a task
  - 1. With the caret anywhere in a bullet, press **Ctrl+Enter** (Cmd on Mac). The bullet becomes `TODO …`; press again for `DOING`, again for `DONE`, once more to drop the marker. (Under the NOW / LATER workflow — the default — the cycle is LATER → NOW → DONE.)
  - 2. Prefer the mouse? Click the marker chip to toggle between the workflow's two open states (TODO ↔ DOING or LATER ↔ NOW), or click the checkbox in front of the task to finish it in one step.
  - 3. Select several blocks first and the same keypress cycles every selected task at once.
  - 4. What you should see: a colored marker chip at the start of the block, plus an empty checkbox until the task is done.
- ## Rank it
  - Type **/** and choose **Priority A** (or B, C) — the marker line becomes `TODO [#A] Call the bank`.
  - What you should see: a colored `[#A]` chip right after the marker; swap it by running the command again.
- ## Give it a date
  - 1. Type **/** and choose **Scheduled**, then pick the day in the calendar popup — use **+ Add time** if the hour matters.
  - 2. Add **Deadline** the same way when the task has a hard due date.
  - 3. What you should see: a small calendar chip next to the block for each date. Click a chip any time to change or remove it.
- ## Try it: from bullets to an open-task query
	- Copy this Guide into your graph and look at these example tasks:
	- TODO [#A] Collect the audit receipts
		SCHEDULED: <2026-08-12 Wed>
	- TODO Book the flights
		DEADLINE: <2026-08-15 Sat>
	- LATER Read the harvest notes
	- Water the office plants
		SCHEDULED: <2026-08-12 Wed +1w>
	- Then this query lists them:
	- {{query (task TODO DOING NOW LATER)}}
	- What you should see: the marked tasks above, plus tasks elsewhere in the graph that use `TODO`, `DOING`, `NOW`, or `LATER` (the [[Feature showcase]] has a few; a copied Guide adds your own). The query covers the whole graph — a task written on any page turns up here, not just the ones on this page; DONE tasks and tasks using other markers do not appear.
- ## See the plan: the Agenda
  - 1. Look at the bottom of today's block in the Journals feed (the first day): the **Scheduled & Deadline** list shows open items whose date is near today, wherever in the graph they were written.
  - 2. What you should see: your dated tasks, each with its date chip; click one to jump to it. Widen or narrow the range in Settings → **Journals** → **Agenda window** (default: a week back and a week ahead). The Agenda sits under today in the feed, not on a journal page you opened by itself, and the list vanishes when nothing is dated in range.
- ## Carry-over: let unfinished tasks follow you
  - 1. Scroll to a past day in the feed and click **Carry unfinished tasks → today** under its title. Blocks with open markers move to today; DONE and CANCELED stay put.
  - 2. What you should see: a "Carried N items to today" toast — or "No unfinished tasks to carry" if none were open.
  - 3. Variants: on today, **Carry from previous day** pulls the latest earlier day in; **Carry last N days** goes further back (N is set in Settings, default 7). The past-day action is also on that day's right-click menu, and the command palette (Ctrl+Shift+P) has 7 / 30 / 365-day presets.
  - Settings → **Journals** also tunes it: hide the buttons (the right-click menu keeps working), move the whole block with its context or just the task line, add a “Carried over” heading.
- ## Where everything lives
  - All of this remains plain text. Journal entries use the graph's configured journal directory; tasks on ordinary pages stay in their own Markdown or Org files. Within a task, the marker comes first, then `[#A]`, then the `SCHEDULED:`/`DEADLINE:` lines — readable by Logseq and by any editor. The companion reference page [[Reference/Journals, tasks, and scheduling]] maps the marker set, repeaters, time tracking, and the stored query behind the Agenda. If something looks wrong — a banner, a conflict copy, a duplicate day — [[Reference/Troubleshooting and recovery]] has the recovery steps.
