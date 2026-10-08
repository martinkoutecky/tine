icon:: 🧱

- # Structure repeated information
	- Start with the bullets you already write. Add a few fields when they become useful, then choose a table or board to see the same blocks from a different angle.
- ## A small project tracker
  tine.view:: table
  tine.fields:: status=enum:planned,active,done;owner=text;estimate=number
	- Refresh the Guide examples
	  status:: active
	  owner:: Avery
	  estimate:: 2
	- Decide the next topic
	  status:: planned
	  owner:: Jules
	  estimate:: 1
	- Publish the updated demo
	  status:: done
	  owner:: Avery
	  estimate:: 3
- ## Turn child bullets into a view
	- 1. Make a parent bullet like the tracker above, then add ordinary child bullets for each item.
	- 2. Right-click the parent and choose **Show children as → Table** (or type `/Table`). Use **+ Add column** to add fields such as Status, Owner, and Estimate; edit their values in the cells.
	- 3. When progress matters more than rows, type `/Board` on that parent. Choose **Group by** and select Status to make one column per status.
	- 4. What you should see: the same child bullets appear as editable table rows or status columns. Move a board card to change that block's grouping field; return to the outline and the change is there too.
- ## Find and reuse the same kind of block
	- 1. Press **Ctrl+K**, then search for `Refresh Guide examples`. The friendly search surface finds that tracker block without needing a query expression.
	- 2. Choose **Open search tab**. There, choose **Filters / Advanced**, then **Edit as visual query**. The query reads as one sentence — *Blocks where …* — over a sheet of conditions; use **+ Add condition** → **Property** to choose Owner and the value Avery.
	- 3. Choose **Table** or **Board** in the view switcher (**Search**, **List**, **Table**, **Board**) above the results. The selection answers *which blocks* to show; the view answers *how* to show them, so you can change the presentation without remaking the selection.
	- 4. Open **Display** beside the conditions to set the rest of the presentation in one place: the view, what the results are grouped by, the sort order, which columns show, what the footer summarizes, and a sample size that limits the rows. Sorts, columns, and summaries are ordered lists you can add to, reorder with the arrows, and remove from.
	- 5. What you should see: a reusable selection of matching tracker blocks, first as search results and then as the table or board you chose.
- ### Query-backed board of the same tracker
	- {{query (property owner Avery)}}
	  tine.view:: board
	  tine.group-field:: prop:status
	- This board finds the matching tracker bullets above; it does not copy them. Inspect it here, in a copied Guide, or in the public demo, then change its view with the visual controls.
- ## What is stored, only when you need to inspect it
	- The view-owning parent carries `tine.view:: table` or `tine.view:: board`; the optional `tine.fields::` line declares the columns and their types. The child bullets remain the source blocks with their ordinary properties.
	- The query-backed example stores `{{query (property owner Avery)}}` to choose its blocks, while its `tine.view:: board` and `tine.group-field:: prop:status` lines choose the presentation. A query groups by a field name — `prop:status` for the property named Status, or `state`, `priority`, `tags`, `page` — so the same word cannot mean two things on two different views. An older graph that says `tine.group-by:: status` keeps working and is rewritten the first time you change the grouping from Display. The visual builder and friendly search lead here without requiring you to type the query syntax.
- ## Optional: add a derived field
	- If Estimate needs a computed companion, add a formula from the table header. For the visual formula editor, its read-only results, and the raw-expression escape hatch, use [[Features/Formulas]]. For table, board, and query-view details, return to [[Features/Sheets]].
