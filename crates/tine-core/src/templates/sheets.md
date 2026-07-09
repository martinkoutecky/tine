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
	- 1. Create a heading block for the thing you want to compare, for example `## Reading plan`.
	- 2. Add `tine.view:: grid` under that heading. Add `tine.header:: true` if the first row should be column labels.
	- 3. Add child bullets for rows. Inside each row, add one child bullet per cell.
	- 4. Drag row or cell bullets to reshape the outline; the grid follows the tree.
	- 5. What you should see: a live grid whose cells are still ordinary Logseq bullets.
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
	- 1. Create a parent block and add `tine.view:: table` under it.
	- 2. Add one child bullet per row. Put properties such as `owner:: Martin` or `estimate:: 2` on each row.
	- 3. Optional: add `tine.col-aggregates:: prop:estimate=sum` to sum a numeric property in the footer.
	- 4. What you should see: each child bullet becomes a row, and properties become editable table columns.
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
	- 1. Start with a table that has a numeric or enum field, for example `rating=number` in `tine.fields::`.
	- 2. Right-click a column header and choose **Add/Edit formula**.
	- 3. Use the visual formula builder faces: pick a field, add a comparison, and combine it with IF / THEN / ELSE.
	- 4. Use the `</> raw` toggle when you want to type the expression directly, such as `rating * 2`.
	- 5. What you should see: a read-only computed column whose values evaluate live and are not written onto the rows.
- ## Task board
	- {{query (and (task TODO DOING DONE) #sheets-demo)}}
	  tine.view:: board
	  tine.group-by:: state
- ### Create one yourself — query
	- 1. Type `/Query` and create a query for the blocks you want, such as tasks tagged `#sheets-demo`.
	- 2. Keep the query in one block, then add `tine.view:: table` or `tine.view:: board` under that same block.
	- 3. For a board, add `tine.group-by:: state` or another property/tag axis.
	- 4. What you should see: query results render as a live sheet view without copying the source blocks.
- ### Create one yourself — board
	- 1. Create a parent block and add `tine.view:: board`.
	- 2. Add `tine.group-by:: state`, `tine.group-by:: tags`, or a property name to choose the columns.
	- 3. Add child bullets, or put the board properties under a query block to board over query results.
	- 4. What you should see: cards group into columns while staying normal outline blocks underneath.
- ## Topic board
  tine.view:: board
  tine.group-by:: tags
	- Schema menu polish #schema #sheets-demo
	- CSV drop walkthrough #interop #sheets-demo
