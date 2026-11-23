- Readonly grid demo
  tine.view:: grid
  tine.header:: true
  tine.col-widths:: 0=140;1=180;2=220
	-
		- Project
		- Status
		- Notes
	-
		- TODO Ship [[Tine]] sheet
		- Nested sub-grid
		  tine.view:: grid
			-
				- Inner A
				- Inner B
			-
				- Inner C
		- Uses tree geometry only
	-
		- Ragged row
		- missing note cell
	-
		- Done
		- Read-only
		- Phase 2 adds interaction
- Field table demo
  tine.view:: table
  tine.col-aggregates:: prop:estimate=sum
  tine.fields:: state=state;owner=text;topic=enum:infra,ui,docs;points=number;shipped=checkbox;due=date;estimate=text
  tine.formula.effort:: points * 2
  tine.formula.due-soon:: if(isEmpty(due), false, due < today() + "14d")
  tine.formula.broken:: points +
	- TODO [#A] Draft spec #sheets
	  SCHEDULED: <2026-07-08 Wed>
	  owner:: Martin
	  estimate:: 2h
	  topic:: docs
	  points:: 3
	  shipped:: false
	  due:: 2026-07-09
	- DOING Build table renderer #sheets
	  owner:: Codex
	  estimate:: 5h
	  topic:: ui
	  points:: 8
	  shipped:: false
	  note:: stray column
	- DONE Verify screenshots
	  DEADLINE: <2026-07-10 Fri>
	  owner:: Codex
	  topic:: infra
	  points:: 1
	  shipped:: true
	  due:: 2026-07-07
- {{query (todo TODO DOING DONE)}}
  tine.view:: board
  tine.group-by:: state
- Reading list by topic
  tine.view:: board
  tine.group-by:: tags
	- Aaronson survey #reading
	- n-fold draft #reading #writing
	- ChoCo rebuttal #writing
