icon:: ▦

- # Sheets
	- Sheets turn ordinary outline branches into 2-D views. The same file still opens in Logseq as nested bullets with harmless `tine.*` properties.
	- Formula columns derive values live (the *Typed reading list* below computes `effort` from `rating`); derived values are shown, never written onto the rows.
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
- ### Create one yourself — grid
	- 1. On an empty block, type `/Grid` and pick **Grid**. Tine seeds one editable cell — just start typing.
	- 2. Grow it by its **edges**: hover the grid and click the **`+`** on the right edge for a new column, or the **`+`** on the bottom edge for a new row. Drag row or cell bullets to reshape it; the grid follows the tree.
	- 3. Already have an outline? Right-click its parent bullet and choose **Show children as → Grid** (or run `/Grid` on that block) to turn the existing bullets into a grid in place.
	- 4. What you should see: a live grid whose cells are still ordinary Logseq bullets.
	- 5. Under the hood: the block just carries `tine.view:: grid` (plus optional `tine.header:: true` for a label row and `tine.col-widths::`). You never have to type those by hand.
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
- ### Create one yourself — table
	- 1. On a block, type `/Table` and pick **Table** (or right-click an existing outline and choose **Show children as → Table**).
	- 2. Use the ghost **+ Add row** and **+ Add column** buttons to build it out. Each row is a child bullet; each column is a property such as `owner::` or `estimate::`, edited right in the cells.
	- 3. Right-click a children-backed column header or double-click its name to rename the field and its dependent filter, group, aggregate, and formula references together. Ambiguous or colliding names are refused.
	- 4. Press **Tab** after editing a cell to commit its value and move to the next cell.
	- 5. What you should see: each child bullet becomes a row, and properties become editable table columns.
	- 6. Under the hood: the block carries `tine.view:: table` (with optional `tine.fields::` to type columns and `tine.col-aggregates:: prop:estimate=sum` to total a numeric column in the footer). The ghost buttons write these for you.
- ## Typed reading list
  tine.view:: table
  tine.fields:: status=enum:todo,reading,done;rating=number;done=checkbox;owner=ref
  tine.formula.effort:: rating * 2
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
- ### Create one yourself — formula
	- 1. Start with a table that has a numeric or enum field, for example `rating=number`.
	- 2. Right-click a column header and choose **Add/Edit formula**, then build the value from the visual faces (or the `</> raw` box).
	- 3. What you should see: a read-only computed column whose values evaluate live and are not written onto the rows.
	- 4. For the full walkthrough — the value picker, IF / THEN / ELSE, transforms, and when to reach for `</> raw` — see [[Features/Formulas]].
- ## Task board
	- {{query (and (task TODO DOING DONE) #sheets-demo)}}
	  tine.view:: board
	  tine.group-by:: state
- ### Create one yourself — query
	- 1. Type `/Query` and create a query for the blocks you want, such as tasks tagged `#sheets-demo`.
	- 2. Keep the query in one block, then run `/Table` or `/Board` on that same block to view the results.
	- 3. For a board, use the **Group by** dropdown (see below) to pick the column axis.
	- 4. What you should see: query results render as a live sheet view without copying the source blocks.
- ### Create one yourself — board
	- 1. On a block, type `/Board` and pick **Board** (`/Kanban` finds it too).
	- 2. Choose the column axis with the **Group by** dropdown in the board header, or right-click and use **Group by →** to pick State, Tags, Priority, or any field.
	- 3. Add child bullets, or run `/Board` on a query block to board over query results.
	- 4. What you should see: cards group into columns while staying normal outline blocks underneath.
	- 5. Under the hood: the dropdown just writes `tine.view:: board` and `tine.group-by:: state` (or your chosen axis) for you.
- ## Topic board
  tine.view:: board
  tine.group-by:: tags
	- Schema menu polish #schema #sheets-demo
	- CSV drop walkthrough #interop #sheets-demo
