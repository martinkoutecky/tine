icon:: ▦

- # Sheets
	- Sheets turn ordinary outline branches into 2-D views. The same file still opens in Logseq as nested bullets with harmless `tine.*` properties.
- ## Positional grid
  tine.view:: grid
  tine.header:: true
  tine.col-widths:: 0=140;1=130;2=220
	-
		- Area
		- Owner
		- Notes
	-
		- Spec
		  background-color:: yellow
		- Martin
		- Nested sub-grid
		  tine.view:: grid
			-
				- Risk
				- Mitigation
			-
				- Scope drift
				- Keep v1 narrow
	-
		- Build
		- Codex
		- Ragged rows are fine
	-
		- Empty middle cell
		-
		- Still round-trips
- ## Task table
  tine.view:: table
  tine.col-aggregates:: prop:estimate=sum
	- TODO [#A] Draft the sheet docs #sheets-demo
	  SCHEDULED: <2026-07-08 Wed>
	  owner:: Martin
	  estimate:: 2
	- DOING Polish cell menus #sheets-demo
	  owner:: Codex
	  estimate:: 5
	- DONE Add aggregate footer #sheets-demo
	  DEADLINE: <2026-07-10 Fri>
	  owner:: Codex
	  estimate:: 1
- ## Typed reading list
  tine.view:: table
  tine.fields:: status=enum:todo,reading,done;rating=number;done=checkbox;owner=ref
	- Bases study
	  status:: reading
	  rating:: 5
	  done:: false
	  owner:: [[Martin]]
	- CSV import notes
	  status:: todo
	  rating:: 3
	  done:: false
	  owner:: [[Codex]]
- ## Task board
	- {{query (and (task TODO DOING DONE) #sheets-demo)}}
	  tine.view:: board
	  tine.group-by:: state
- ## Topic board
  tine.view:: board
  tine.group-by:: tags
	- Schema menu polish #schema #sheets-demo
	- CSV drop walkthrough #interop #sheets-demo
