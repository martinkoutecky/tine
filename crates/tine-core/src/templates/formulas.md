icon:: 🧮

- # Formulas
	- A **formula column** is an extra column whose value Tine works out for you from the other columns in the row. You never type the result — Tine recomputes it live, shows it on screen, and never writes it back onto your bullets. Open the same file in Logseq and you just see ordinary bullets with a harmless `tine.formula.*` property.
- ## A formula in action
  tine.view:: table
  tine.fields:: task=text;hours=number;done=checkbox
  tine.formula.plan:: if(hours > 3, "focus block", "quick task")
	- Sketch the outline
	  hours:: 2
	  done:: true
	- Write the first draft
	  hours:: 5
	  done:: false
	- The **plan** column is a formula. Tine reads `hours` from each row and shows *focus block* or *quick task* — you never type it, and it updates the moment `hours` changes.
- ## Add one yourself
	- 1. Make a table (see [[Features/Sheets]]) with at least one field to compute from, for example a `hours=number` column.
	- 2. **Right-click a table column header** — or right-click the table body / open its **⋮** menu — and choose **Add formula…**. Give it a short lowercase name like `plan`.
	- 3. The **visual builder** opens. You build the value by clicking *faces* — no syntax to memorize. Press **Save** and a read-only column appears, filling in live.
	- 4. To change it later, right-click the formula column and choose **Edit formula…**.
- ## The building blocks
	- **Value face** — click it to open a picker with these sections:
		- **Field** — one of your own columns (or type a custom field name).
		- **Formula** — another formula column on the same table, shown as `formula.<name>`.
		- **Literal** — a fixed number, a bit of text, or **True** / **False** / **Empty**.
		- **Function** — `if condition`, `is empty`, `now`, `today`.
		- **Transform** — tweaks like *lowercase*, *trim*, *round*, *year*, *format date*, *length*.
	- **IF / THEN / ELSE** — pick the **if condition** function on any value to branch: *IF* a condition *THEN* one value *ELSE* another value.
	- **Condition** — compare two values with an operator (`=`, `≠`, `<`, `≤`, `>`, `≥`), and join several with **AND** / **OR**.
- ## `</> raw` — type it directly
	- The **`</> raw`** button at the top-right of the editor switches from clickable faces to a plain text box, where you can type the expression yourself — for example `hours * 2` or `if(hours > 3, "focus block", "quick task")`. The chips below insert your fields, other formulas, and built-in functions at the cursor. Flip **Builder** back on whenever the text is something the faces can show again.
- ## Honest limits
	- The visual builder handles **one level**: a single IF/THEN/ELSE, simple comparisons, and a value that is at most one arithmetic step of two simple values (like `hours * 2`).
	- Anything more nested — a formula inside a THEN branch, or arithmetic of arithmetic like `(hours + extra) * 2` — shows up as a small raw box instead of pretty faces. Use **`</> raw`** to type those directly; they still evaluate exactly the same, they just do not get a visual face.
	- Formula values are **read-only** and are never saved onto your rows, so they always reflect the current data and round-trip cleanly back to Logseq.
- ### Create one yourself — formula
	- 1. Start from a table with a numeric or enum field.
	- 2. Right-click the column header and choose **Add/Edit formula**.
	- 3. Build the value from the faces — pick a field, add a comparison, wrap it in IF / THEN / ELSE.
	- 4. Use the `</> raw` toggle when you would rather type the expression, such as `hours * 2`.
	- 5. What you should see: a read-only computed column whose values evaluate live and are never written onto the rows.
