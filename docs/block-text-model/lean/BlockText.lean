import Init

namespace BlockText

/- Physical bytes, line lexer, and outline-directed reading -/

abbrev Bytes := List UInt8
def lf : UInt8 := 10
def cr : UInt8 := 13
def space : UInt8 := 32

inductive Ending where
  | lf | crlf | cr | eof
  deriving DecidableEq, Repr

def Ending.bytes : Ending → Bytes
  | .lf => [BlockText.lf]
  | .crlf => [BlockText.cr, BlockText.lf]
  | .cr => [BlockText.cr]
  | .eof => []

structure Line where
  body : Bytes
  ending : Ending
  deriving DecidableEq, Repr

def Line.bytes (line : Line) : Bytes := line.body ++ line.ending.bytes

def prependByte (b : UInt8) (lines : List Line) : List Line :=
  if b = lf then ⟨[], .lf⟩ :: lines
  else if b = cr then
    match lines with
    | ⟨[], .lf⟩ :: rest => ⟨[], .crlf⟩ :: rest
    | _ => ⟨[], .cr⟩ :: lines
  else
    match lines with
    | [] => [⟨[b], .eof⟩]
    | line :: rest => { line with body := b :: line.body } :: rest

def lex : Bytes → List Line
  | [] => []
  | b :: rest => prependByte b (lex rest)

theorem prependByte_lossless (b : UInt8) (lines : List Line) :
    (prependByte b lines).flatMap Line.bytes = b :: lines.flatMap Line.bytes := by
  unfold prependByte
  split <;> rename_i h
  · simp [h, Line.bytes, Ending.bytes]
  split <;> rename_i h'
  · split <;> simp [h', Line.bytes, Ending.bytes]
  · split <;> simp [Line.bytes, Ending.bytes]

theorem lex_lossless (bytes : Bytes) : (lex bytes).flatMap Line.bytes = bytes := by
  induction bytes with
  | nil => rfl
  | cons b rest ih => rw [lex, prependByte_lossless, ih]

structure Piece where
  layout : Bytes
  text : Bytes
  ending : Ending
  deriving DecidableEq, Repr

def Piece.line (p : Piece) : Line := ⟨p.layout ++ p.text, p.ending⟩
def Piece.bytes (p : Piece) : Bytes := p.line.bytes

def splitLine (width : Nat) (line : Line) : Piece :=
  ⟨line.body.take width, line.body.drop width, line.ending⟩

@[simp] theorem splitLine_bytes (width : Nat) (line : Line) :
    (splitLine width line).bytes = line.bytes := by
  simp [splitLine, Piece.bytes, Piece.line, Line.bytes]

structure SlotSpec where
  width : Nat
  level : Nat
  /-- One removed width per content row, starting with the header. Remaining rows are trailer. -/
  trims : List Nat
  /-- Parser reports a bare header. Its content can begin immediately after the marker. -/
  bare : Bool := false
  continuation : Bytes := []
  deriving DecidableEq, Repr

structure Outline where
  preamble : Nat
  slots : List SlotSpec
  deriving DecidableEq, Repr

structure Slot where
  level : Nat
  rows : List Piece
  trailer : List Line
  bare : Bool
  continuation : Bytes
  deriving DecidableEq, Repr

def Slot.bytes (slot : Slot) : Bytes :=
  slot.rows.flatMap Piece.bytes ++ slot.trailer.flatMap Line.bytes

def readSlot (spec : SlotSpec) (bytes : Bytes) : Slot :=
  let lines := lex bytes
  ⟨spec.level, List.zipWith splitLine spec.trims lines, lines.drop spec.trims.length,
    spec.bare, spec.continuation⟩

structure Page where
  preamble : List Line
  slots : List Slot
  /-- Unassigned tail, empty for an admissible outline. -/
  unassigned : List Line
  deriving DecidableEq, Repr

def readSlots : List SlotSpec → Bytes → List Slot × List Line
  | [], bytes => ([], lex bytes)
  | spec :: specs, bytes =>
    let (slots, tail) := readSlots specs (bytes.drop spec.width)
    (readSlot spec (bytes.take spec.width) :: slots, tail)

def read (outline : Outline) (bytes : Bytes) : Page :=
  let (slots, tail) := readSlots outline.slots (bytes.drop outline.preamble)
  ⟨lex (bytes.take outline.preamble), slots, tail⟩

def Page.bytes (page : Page) : Bytes :=
  page.preamble.flatMap Line.bytes ++ page.slots.flatMap Slot.bytes ++
    page.unassigned.flatMap Line.bytes

theorem splitLines_lossless (trims : List Nat) (lines : List Line) :
    (List.zipWith splitLine trims lines).flatMap Piece.bytes ++
      (lines.drop trims.length).flatMap Line.bytes = lines.flatMap Line.bytes := by
  induction trims generalizing lines with
  | nil => simp
  | cons w ws ih =>
    cases lines with
    | nil => simp
    | cons line lines => simp [List.append_assoc, ih]

@[simp] theorem readSlot_lossless (spec : SlotSpec) (bytes : Bytes) :
    (readSlot spec bytes).bytes = bytes := by
  simp [readSlot, Slot.bytes, splitLines_lossless, lex_lossless]

theorem readSlots_lossless (specs : List SlotSpec) (bytes : Bytes) :
    (readSlots specs bytes).1.flatMap Slot.bytes ++
      (readSlots specs bytes).2.flatMap Line.bytes = bytes := by
  induction specs generalizing bytes with
  | nil => simp [readSlots, lex_lossless]
  | cons spec specs ih => simp [readSlots, List.append_assoc, ih]

theorem R0 (outline : Outline) (bytes : Bytes) : (read outline bytes).bytes = bytes := by
  simp [read, Page.bytes, lex_lossless, List.append_assoc, readSlots_lossless]

/- Content cells and byte coordinates -/

inductive Cell where
  | text (byte : UInt8)
  | newline (ending : Ending) (indent : Bytes)
  deriving DecidableEq, Repr

def Cell.raw : Cell → Bytes
  | .text b => [b]
  | .newline e indent => e.bytes ++ indent

def Cell.logical : Cell → UInt8
  | .text b => b
  | .newline .. => lf

def cells (row : Piece) : List Piece → List Cell
  | [] => row.text.map Cell.text
  | next :: tail =>
    row.text.map Cell.text ++ .newline row.ending next.layout :: cells next tail

def rowCells : List Piece → List Cell
  | [] => []
  | head :: tail => cells head tail

def Slot.content (slot : Slot) : Bytes := (rowCells slot.rows).map Cell.logical

def front (rows : List Piece) : Bytes := (rows.head?.map Piece.layout).getD []
def finalEnding (rows : List Piece) : Ending := (rows.getLast?.map Piece.ending).getD .eof

def offset : List Cell → Nat → Nat
  | cell :: rest, i + 1 => cell.raw.length + offset rest i
  | _, _ => 0

/-- A cursor maps to a byte gap, including the end of the final content row. -/
def Slot.gap (slot : Slot) (i : Nat) : Nat :=
  (front slot.rows).length + offset (rowCells slot.rows) i

theorem cells_text_raw (bytes : Bytes) : (bytes.map Cell.text).flatMap Cell.raw = bytes := by
  induction bytes <;> simp_all [Cell.raw]

theorem cells_render (row : Piece) (tail : List Piece) :
    row.layout ++ (cells row tail).flatMap Cell.raw ++ (finalEnding (row :: tail)).bytes =
      (row :: tail).flatMap Piece.bytes := by
  induction tail generalizing row with
  | nil => simp [cells, finalEnding, cells_text_raw, Piece.bytes, Piece.line, Line.bytes]
  | cons next tail ih =>
    simp [cells, cells_text_raw, Cell.raw, Piece.bytes, Piece.line, Line.bytes, List.append_assoc]
    have h := ih next
    simp [Piece.bytes, Piece.line, Line.bytes, List.append_assoc] at h
    rw [← h]
    simp [finalEnding]

theorem slot_render (slot : Slot) :
    slot.bytes = front slot.rows ++ (rowCells slot.rows).flatMap Cell.raw ++
      (finalEnding slot.rows).bytes ++ slot.trailer.flatMap Line.bytes := by
  cases h : slot.rows with
  | nil => simp [Slot.bytes, h, front, rowCells, finalEnding, Ending.bytes]
  | cons head tail =>
    change (slot.rows.flatMap Piece.bytes) ++ _ = _
    rw [h]
    simp only [front, List.head?_cons, Option.map_some, Option.getD_some, rowCells]
    rw [← cells_render]

theorem offset_eq (stream : List Cell) (i : Nat) :
    offset stream i = ((stream.take i).flatMap Cell.raw).length := by
  induction stream generalizing i with
  | nil => cases i <;> rfl
  | cons cell rest ih => cases i <;> simp [offset, ih]

theorem offset_monotone (stream : List Cell) {i j : Nat} (h : i ≤ j) :
    offset stream i ≤ offset stream j := by
  induction stream generalizing i j with
  | nil => cases i <;> cases j <;> simp [offset]
  | cons cell rest ih =>
    cases i <;> cases j <;> simp_all [offset]

theorem gap_monotone (slot : Slot) {i j : Nat} (h : i ≤ j) :
    slot.gap i ≤ slot.gap j :=
  Nat.add_le_add_left (offset_monotone _ h) _

theorem gap_total (slot : Slot) (i : Nat) : slot.gap i ≤ slot.bytes.length := by
  let stream := rowCells slot.rows
  have h := gap_monotone slot (i := i) (j := max i stream.length) (Nat.le_max_left ..)
  have takeAll : stream.take (max i stream.length) = stream :=
    List.take_of_length_le (Nat.le_max_right ..)
  have next : slot.gap (max i stream.length) ≤ slot.bytes.length := by
    rw [Slot.gap, offset_eq, takeAll, slot_render]
    simp only [List.length_append]
    dsimp [stream]
    omega
  exact Nat.le_trans h next

theorem take_at_cell {stream : List Cell} {i : Nat} {cell : Cell} (suffix : Bytes)
    (h : stream[i]? = some cell) :
    (((stream.flatMap Cell.raw) ++ suffix).drop (offset stream i)).take cell.raw.length =
      cell.raw := by
  induction stream generalizing i with
  | nil => simp at h
  | cons head tail ih =>
    cases i with
    | zero =>
      simp only [List.getElem?_cons_zero, Option.some.injEq] at h
      simp [h, offset, List.append_assoc]
    | succ i =>
      simp only [List.getElem?_cons_succ] at h
      rw [offset, List.flatMap_cons, List.append_assoc, List.drop_length_add_append]
      exact ih h

theorem mapped_cell (slot : Slot) {i : Nat} {cell : Cell}
    (h : (rowCells slot.rows)[i]? = some cell) :
    (slot.bytes.drop (slot.gap i)).take cell.raw.length = cell.raw := by
  rw [slot_render, Slot.gap]
  simp only [List.append_assoc]
  rw [List.drop_length_add_append]
  exact take_at_cell ((finalEnding slot.rows).bytes ++ slot.trailer.flatMap Line.bytes) h

theorem content_text_inverse (slot : Slot) {i : Nat} {b : UInt8}
    (h : (rowCells slot.rows)[i]? = some (.text b)) :
    slot.content[i]? = some b ∧ (slot.bytes.drop (slot.gap i)).take 1 = [b] := by
  exact ⟨by simp [Slot.content, List.getElem?_map, h, Cell.logical], mapped_cell slot h⟩

theorem content_newline (slot : Slot) {i : Nat} {e : Ending} {indent : Bytes}
    (h : (rowCells slot.rows)[i]? = some (.newline e indent)) :
    slot.content[i]? = some lf ∧
      (slot.bytes.drop (slot.gap i)).take (e.bytes ++ indent).length = e.bytes ++ indent := by
  exact ⟨by simp [Slot.content, List.getElem?_map, h, Cell.logical], mapped_cell slot h⟩

/-- An assembly always has a first row, including for empty content. -/
def assemble (ending : Ending) : List Cell → Piece × List Piece
  | [] => (⟨[], [], ending⟩, [])
  | .text b :: rest =>
    let (head, tail) := assemble ending rest
    ({ head with text := b :: head.text }, tail)
  | .newline e indent :: rest =>
    let (head, tail) := assemble ending rest
    (⟨[], [], e⟩, { head with layout := indent } :: tail)

def assembleRows (layout : Bytes) (ending : Ending) (stream : List Cell) : List Piece :=
  let (head, tail) := assemble ending stream
  { head with layout := layout } :: tail

@[simp] theorem assemble_cells (layout : Bytes) (ending : Ending) (stream : List Cell) :
    rowCells (assembleRows layout ending stream) = stream := by
  induction stream generalizing layout with
  | nil => rfl
  | cons cell rest ih =>
    cases cell with
    | text b =>
      cases h : (assemble ending rest).2 <;>
        simpa [cells, assembleRows, assemble, rowCells, h] using
          congrArg (List.cons (.text b)) (ih layout)
    | newline e indent =>
      simpa [cells, assembleRows, assemble, rowCells] using
        congrArg (List.cons (.newline e indent)) (ih indent)

@[simp] theorem assemble_front (layout : Bytes) (ending : Ending) (stream : List Cell) :
    front (assembleRows layout ending stream) = layout := rfl

@[simp] theorem assemble_ending (layout : Bytes) (ending : Ending) (stream : List Cell) :
    finalEnding (assembleRows layout ending stream) = ending := by
  induction stream generalizing layout with
  | nil => rfl
  | cons cell rest ih =>
    cases cell
    · cases h : (assemble ending rest).2 <;>
        simpa [assembleRows, assemble, finalEnding, h] using ih layout
    · simpa [assembleRows, assemble, finalEnding] using ih _


/- Snapshot splice order and the writer -/

structure Splice where
  start : Nat
  stop : Nat
  replacement : Bytes
  deriving DecidableEq, Repr

def keyLE (a b : Splice) : Bool :=
  decide (a.start < b.start ∨ a.start = b.start ∧ a.stop ≤ b.stop)

def separate (a b : Splice) : Prop :=
  (a.start, a.stop) ≠ (b.start, b.stop) ∧
    (a.stop ≤ b.start ∨ b.stop ≤ a.start)

def ordered (edits : List Splice) : List Splice := edits.mergeSort keyLE

theorem ordered_pairwise (edits : List Splice) :
    (ordered edits).Pairwise (fun a b => keyLE a b = true) := by
  apply List.pairwise_mergeSort
  · intro a b c
    simp [keyLE]
    omega
  · intro a b
    simp [keyLE]
    omega

theorem ordered_perm {edits other : List Splice} (h : edits.Perm other)
    (disjoint : edits.Pairwise separate) : ordered edits = ordered other := by
  have perm : (ordered edits).Perm (ordered other) :=
    (List.mergeSort_perm edits _).trans (h.trans (List.mergeSort_perm other _).symm)
  have distinct : (ordered edits).Pairwise separate :=
    (List.mergeSort_perm edits _).symm.pairwise disjoint (by
      intro a b hab
      exact ⟨Ne.symm hab.1, hab.2.symm⟩)
  apply List.Perm.eq_of_pairwise ?_ (ordered_pairwise _) (ordered_pairwise _) perm
  intro a b hab hba hleft hright
  simp [keyLE] at hleft hright
  have starts : a.start = b.start := by omega
  have stops : a.stop = b.stop := by omega
  by_cases ne : a = b
  · exact ne
  exfalso
  have pair := List.pairwise_iff_getElem.mp distinct
  have hb : b ∈ ordered edits := perm.mem_iff.mpr hba
  obtain ⟨i, hi, rfl⟩ := List.getElem_of_mem hab
  obtain ⟨j, hj, rfl⟩ := List.getElem_of_mem hb
  have ij : i ≠ j := by
    intro eq
    subst j
    exact ne rfl
  rcases Nat.lt_or_gt_of_ne ij with lt | gt
  · exact (pair i j hi hj lt).1 (Prod.ext starts stops)
  · exact (pair j i hj hi gt).1 (Prod.ext starts.symm stops.symm)

/-- Every chunk records a kept run, the next deleted run, and the replacement. -/
structure Chunk (α : Type) where
  keep : List α
  cut : List α
  put : List α

def Chunk.before (c : Chunk α) : List α := c.keep ++ c.cut
def Chunk.after (c : Chunk α) : List α := c.keep ++ c.put

def partition (encode : Splice → List α) (source : List α) (cursor : Nat) :
    List Splice → List (Chunk α)
  | [] => [⟨source, [], []⟩]
  | edit :: rest =>
    let kept := edit.start - cursor
    let removed := edit.stop - edit.start
    let remaining := source.drop kept
    ⟨source.take kept, remaining.take removed, encode edit⟩ ::
      partition encode (remaining.drop removed) edit.stop rest

theorem partition_before (encode : Splice → List α) (source : List α) (cursor : Nat)
    (edits : List Splice) :
    (partition encode source cursor edits).flatMap Chunk.before = source := by
  induction edits generalizing source cursor with
  | nil => simp [partition, Chunk.before]
  | cons edit rest ih => simp [partition, Chunk.before, List.append_assoc, ih, -List.drop_drop]

def OrderedWithin (limit : Nat) (cursor : Nat) : List Splice → Prop
  | [] => cursor ≤ limit
  | edit :: rest =>
    cursor ≤ edit.start ∧ edit.start ≤ edit.stop ∧ OrderedWithin limit edit.stop rest

theorem OrderedWithin.cursor_le {limit cursor : Nat} {edits : List Splice}
    (h : OrderedWithin limit cursor edits) : cursor ≤ limit := by
  induction edits generalizing cursor with
  | nil => exact h
  | cons edit rest ih =>
    rcases h with ⟨hc, hs, ht⟩
    have := ih ht
    omega

theorem partition_cut (encode : Splice → List α) (source : List α) (cursor : Nat)
    (edits : List Splice) (valid : OrderedWithin source.length cursor edits) :
    ((partition encode (source.drop cursor) cursor edits).map Chunk.cut).take edits.length =
      edits.map (fun e => (source.drop e.start).take (e.stop - e.start)) := by
  induction edits generalizing cursor with
  | nil => simp
  | cons edit rest ih =>
    rcases valid with ⟨hc, hs, ht⟩
    have start : cursor + (edit.start - cursor) = edit.start := by omega
    have stop : edit.start + (edit.stop - edit.start) = edit.stop := by omega
    simp only [partition, List.drop_drop, start,
      List.map_cons, List.length_cons, List.take_succ_cons, List.cons.injEq, true_and]
    rw [stop]
    exact ih _ ht

def followingEnding (fallback : Ending) : List Cell → Ending
  | [] => fallback
  | .text _ :: rest => followingEnding fallback rest
  | .newline e _ :: _ => e

def insertedEnding : Ending → Ending
  | .crlf => .crlf
  | _ => .lf

def encode (slot : Slot) (edit : Splice) : List Cell :=
  let style := followingEnding (finalEnding slot.rows) ((rowCells slot.rows).drop edit.start)
  edit.replacement.map fun b =>
    if b = lf then .newline (insertedEnding style) slot.continuation else .text b

def chunks (slot : Slot) (edits : List Splice) : List (Chunk Cell) :=
  partition (encode slot) (rowCells slot.rows) 0 (ordered edits)

def editedCells (slot : Slot) (edits : List Splice) : List Cell :=
  (chunks slot edits).flatMap Chunk.after

private def startsWithText : List Cell → Bool
  | .text _ :: _ => true
  | _ => false

private def oldSeparator (slot : Slot) (stream : List Cell) : Bytes :=
  if slot.bare && !startsWithText (rowCells slot.rows) && startsWithText stream then [space]
  else []

private def oldPlannedSlot (slot : Slot) (edits : List Splice) : Slot :=
  if edits.isEmpty || slot.rows.isEmpty then slot
  else
    let stream := editedCells slot edits
    { slot with
      rows := assembleRows (front slot.rows ++ oldSeparator slot stream) (finalEnding slot.rows) stream }

private def oldWriteSlot (slot : Slot) (edits : List Splice) : Bytes := (oldPlannedSlot slot edits).bytes

def repairEnding (afterCR : Bool) (e : Ending) : Ending :=
  if afterCR && e == .lf then .crlf else e

def Cell.afterCR : Cell → Bool
  | .newline .cr [] => true
  | _ => false

def Cell.repair (afterCR : Bool) : Cell → Cell
  | .text b => .text b
  | .newline e indent => .newline (repairEnding afterCR e) indent

def repairCells (afterCR : Bool) : List Cell → List Cell
  | [] => []
  | cell :: rest => cell.repair afterCR :: repairCells cell.afterCR rest

def exitCR (afterCR : Bool) : List Cell → Bool
  | [] => afterCR
  | cell :: rest => exitCR cell.afterCR rest

/-- Text of the first row, excluding its layout, break, and later rows. -/
def firstRowText : List Cell → Bytes
  | .text b :: rest => b :: firstRowText rest
  | _ => []

-- True when this layout followed by this text has the same local header
-- boundary and the parser's structural trim ends exactly at the layout's end.
variable (headReads : Bytes → Bytes → Bool)

/-- A changed, nonempty bare first row can earn a space at the initial content
    gap. The first sorted splice must own that gap. -/
def separator (slot : Slot) (first : Option Splice) (stream : List Cell) : Bytes :=
  match first with
  | none => []
  | some edit =>
    let text := firstRowText stream
    if slot.bare && edit.start == 0 && !text.isEmpty &&
        text != firstRowText (rowCells slot.rows) && !headReads (front slot.rows) text then [space]
    else []

@[simp] theorem separator_same (slot : Slot) (first : Option Splice) :
    separator headReads slot first (rowCells slot.rows) = [] := by
  cases first <;> simp [separator]

/-- The raw boundary or the boundary with one extra layout space must work.
    Only the former is queried by the writer. -/
theorem changedHead_separatorCheck (slot : Slot) (stream : List Cell) (edit : Splice)
    (bare : slot.bare = true) (start : edit.start = 0) (nonempty : firstRowText stream ≠ [])
    (changed : firstRowText (rowCells slot.rows) ≠ firstRowText stream) :
    headReads (front slot.rows ++ separator headReads slot (some edit) stream) (firstRowText stream) =
      (headReads (front slot.rows) (firstRowText stream) ||
        headReads (front slot.rows ++ [space]) (firstRowText stream)) := by
  cases accepted : headReads (front slot.rows) (firstRowText stream) <;>
    simp [separator, bare, start, nonempty, Ne.symm changed, accepted]

def plannedSlot (slot : Slot) (edits : List Splice) (priorCR : Bool := false) : Slot :=
  if edits.isEmpty || slot.rows.isEmpty then slot
  else
    let order := ordered edits
    let stream := (partition (encode slot) (rowCells slot.rows) 0 order).flatMap Chunk.after
    let layout := front slot.rows ++ separator headReads slot order.head? stream
    let adjacent := priorCR && layout.isEmpty
    { slot with
      rows := assembleRows layout
        (repairEnding (exitCR adjacent stream) (finalEnding slot.rows)) (repairCells adjacent stream) }

@[simp] theorem Cell.repair_logical (p : Bool) (cell : Cell) :
    (cell.repair p).logical = cell.logical := by
  cases cell <;> rfl

@[simp] theorem repairCells_logical (p : Bool) (stream : List Cell) :
    (repairCells p stream).map Cell.logical = stream.map Cell.logical := by
  induction stream generalizing p with
  | nil => rfl
  | cons cell rest ih => simp [repairCells, ih]

@[simp] theorem plannedSlot_nil (slot : Slot) (priorCR : Bool := false) :
    plannedSlot headReads slot [] priorCR = slot := rfl

@[simp] theorem plannedSlot_level (slot : Slot) (edits : List Splice) (priorCR : Bool := false) :
    (plannedSlot headReads slot edits priorCR).level = slot.level := by
  unfold plannedSlot
  split <;> rfl

def writeSlot (slot : Slot) (edits : List Splice) (priorCR : Bool := false) : Bytes :=
  (plannedSlot headReads slot edits priorCR).bytes

@[simp] theorem encode_logical (slot : Slot) (edit : Splice) :
    (encode slot edit).map Cell.logical = edit.replacement := by
  unfold encode
  simp only [List.map_map]
  simp only [Function.comp_def]
  conv => rhs; rw [← List.map_id edit.replacement]
  apply List.map_congr_left
  intro b _
  split <;> simp_all [Cell.logical]

theorem partition_after_map (f : α → β) (encode : Splice → List α)
    (source : List α) (cursor : Nat) (edits : List Splice) :
    ((partition encode source cursor edits).flatMap Chunk.after).map f =
      (partition (fun e => (encode e).map f) (source.map f) cursor edits).flatMap Chunk.after := by
  induction edits generalizing source cursor with
  | nil => simp [partition, Chunk.after]
  | cons edit rest ih =>
    simp [partition, Chunk.after, List.map_append, ← List.map_take, ← List.map_drop, ih]

def applyContent (source : Bytes) (edits : List Splice) : Bytes :=
  (partition Splice.replacement source 0 (ordered edits)).flatMap Chunk.after

@[simp] theorem applyContent_nil (bytes : Bytes) : applyContent bytes [] = bytes := by
  simp [applyContent, ordered, partition, Chunk.after]

theorem plan_content (slot : Slot) (edits : List Splice) (h : slot.rows ≠ []) (priorCR : Bool := false) :
    (plannedSlot headReads slot edits priorCR).content = applyContent slot.content edits := by
  cases edits with
  | nil => simp
  | cons edit rest =>
    simp [plannedSlot, h, Slot.content, partition_after_map, applyContent]

theorem plan_render (slot : Slot) (edits : List Splice) (head : slot.rows ≠ [])
    (edited : edits ≠ []) (priorCR : Bool := false) :
    let stream := editedCells slot edits
    let layout := front slot.rows ++ separator headReads slot (ordered edits).head? stream
    let adjacent := priorCR && layout.isEmpty
    writeSlot headReads slot edits priorCR =
      layout ++ (repairCells adjacent stream).flatMap Cell.raw ++
        (repairEnding (exitCR adjacent stream) (finalEnding slot.rows)).bytes ++
          slot.trailer.flatMap Line.bytes := by
  dsimp only
  rw [writeSlot, slot_render]
  cases edits with
  | nil => contradiction
  | cons edit rest => simp [plannedSlot, head, editedCells, chunks, List.append_assoc]

theorem R1_slot (slot : Slot) (priorCR : Bool := false) :
    writeSlot headReads slot [] priorCR = slot.bytes := by
  simp [writeSlot]

theorem plannedSlot_commute (slot : Slot) {edits other : List Splice} (h : edits.Perm other)
    (disjoint : edits.Pairwise separate) (priorCR : Bool) :
    plannedSlot headReads slot edits priorCR = plannedSlot headReads slot other priorCR := by
  have len := h.length_eq
  have empty : edits.isEmpty = other.isEmpty := by
    cases edits <;> cases other <;> simp_all
  simp only [plannedSlot, empty, ordered_perm h disjoint]

theorem commute (slot : Slot) {edits other : List Splice} (h : edits.Perm other)
    (disjoint : edits.Pairwise separate) (priorCR : Bool := false) :
    writeSlot headReads slot edits priorCR = writeSlot headReads slot other priorCR :=
  congrArg Slot.bytes (plannedSlot_commute headReads slot h disjoint priorCR)

/- Byte locality and the separator patch -/

theorem offset_add (stream : List Cell) (i k : Nat) :
    offset stream (i + k) = offset stream i + offset (stream.drop i) k := by
  induction stream generalizing i with
  | nil => simp [offset_eq]
  | cons head tail ih =>
    cases i with
    | zero => simp [offset]
    | succ i =>
      simp only [Nat.succ_add, offset, List.drop_succ_cons, ih, Nat.add_assoc]

theorem drop_raw (stream : List Cell) (i : Nat) (suffix : Bytes) :
    (stream.flatMap Cell.raw ++ suffix).drop (offset stream i) =
      (stream.drop i).flatMap Cell.raw ++ suffix := by
  induction stream generalizing i with
  | nil => simp [offset_eq]
  | cons head tail ih =>
    cases i with
    | zero => rfl
    | succ i =>
      simp only [offset, List.flatMap_cons, List.append_assoc, List.drop_length_add_append,
        List.drop_succ_cons, ih]

theorem take_raw (stream : List Cell) (i : Nat) (suffix : Bytes) :
    (stream.flatMap Cell.raw ++ suffix).take (offset stream i) =
      (stream.take i).flatMap Cell.raw := by
  rw [offset_eq]
  have h := List.take_append_drop i stream
  conv => lhs; arg 2; arg 1; rw [← h]
  simp only [List.flatMap_append, List.append_assoc, List.take_append_length]

theorem cut_bytes (slot : Slot) (start stop : Nat) (inOrder : start ≤ stop) :
    ((rowCells slot.rows).drop start |>.take (stop - start)).flatMap Cell.raw =
      (slot.bytes.drop (slot.gap start)).take (slot.gap stop - slot.gap start) := by
  let stream := rowCells slot.rows
  have h : offset stream stop = offset stream start + offset (stream.drop start) (stop - start) := by
    rw [← offset_add, Nat.add_sub_of_le inOrder]
  have gap : slot.gap stop - slot.gap start = offset (stream.drop start) (stop - start) := by
    unfold Slot.gap
    change (front slot.rows).length + offset stream stop -
      ((front slot.rows).length + offset stream start) = _
    omega
  rw [gap, slot_render, Slot.gap]
  simp only [List.append_assoc, List.drop_length_add_append, drop_raw]
  exact (take_raw (stream.drop start) (stop - start) _).symm

def Slot.suffix (slot : Slot) : Bytes :=
  (finalEnding slot.rows).bytes ++ slot.trailer.flatMap Line.bytes

theorem OrderedWithin.ranges {limit cursor : Nat} {edits : List Splice}
    (h : OrderedWithin limit cursor edits) : ∀ e ∈ edits, e.start ≤ e.stop := by
  induction edits generalizing cursor with
  | nil => simp
  | cons e rest ih =>
    rcases h with ⟨_, hs, ht⟩
    simp only [List.mem_cons]
    intro edit he
    rcases he with rfl | he
    · exact hs
    · exact ih ht _ he


theorem OrderedWithin.stop_le {limit cursor : Nat} {edits : List Splice}
    (h : OrderedWithin limit cursor edits) (e : Splice) (mem : e ∈ edits) : e.stop ≤ limit := by
  induction edits generalizing cursor with
  | nil => simp at mem
  | cons d rest ih =>
    rcases List.mem_cons.mp mem with rfl | mem
    · exact h.2.2.cursor_le
    · exact ih h.2.2 mem

/-- The leading space belongs to the first sorted patch, whose kept prefix is
    empty and whose snapshot start is the initial content gap. -/
theorem separator_firstPatch (slot : Slot) (edits : List Splice)
    (_valid : OrderedWithin slot.content.length 0 (ordered edits))
    (inserted : separator headReads slot (ordered edits).head? (editedCells slot edits) ≠ []) :
    ∃ edit, (ordered edits).head? = some edit ∧ edit.start = 0 := by
  cases first : (ordered edits).head? with
  | none => simp [separator, first] at inserted
  | some edit =>
    refine ⟨edit, rfl, ?_⟩
    cases start : edit.start with
    | zero => rfl
    | succ n => simp [separator, first, start] at inserted

theorem separator_origin (slot : Slot) (edits : List Splice)
    (valid : OrderedWithin slot.content.length 0 (ordered edits))
    (inserted : separator headReads slot (ordered edits).head? (editedCells slot edits) ≠ []) :
    ∃ edit ∈ edits, edit.start = 0 := by
  obtain ⟨edit, first, start⟩ := separator_firstPatch headReads slot edits valid inserted
  exact ⟨edit, List.mem_mergeSort.mp (List.mem_of_head? first), start⟩

/- Independent read-back and parser placement -/

def Slot.lines (slot : Slot) : List Line := slot.rows.map Piece.line ++ slot.trailer

theorem lines_bytes (slot : Slot) : slot.lines.flatMap Line.bytes = slot.bytes := by
  simp [Slot.lines, Slot.bytes, List.flatMap_map]
  rfl

/-- The plan's physical runs must be the runs the file lexer actually returns. -/
def Lexical (slot : Slot) : Prop := lex slot.bytes = slot.lines

def SlotSpec.placement (spec : SlotSpec) : Nat × Nat × List Nat :=
  (spec.width, spec.level, spec.trims)

def Slot.placement (slot : Slot) : Nat × Nat × List Nat :=
  (slot.bytes.length, slot.level, slot.rows.map (fun row => row.layout.length))

theorem splitLine_restore (row : Piece) : splitLine row.layout.length row.line = row := by
  cases row
  simp [splitLine, Piece.line]

theorem splitLines_restore (rows : List Piece) (trailer : List Line) :
    List.zipWith splitLine (rows.map (fun row => row.layout.length))
      (rows.map Piece.line ++ trailer) = rows := by
  induction rows <;> simp_all [splitLine_restore]

theorem readSlot_placement (spec : SlotSpec) (slot : Slot)
    (placement : spec.placement = slot.placement) (lexical : Lexical slot) :
    (readSlot spec slot.bytes).rows = slot.rows ∧
    (readSlot spec slot.bytes).trailer = slot.trailer ∧
    (readSlot spec slot.bytes).level = slot.level := by
  simp only [SlotSpec.placement, Slot.placement, Prod.mk.injEq] at placement
  rcases placement with ⟨_, level, trims⟩
  unfold Lexical at lexical
  have len : (slot.rows.map (fun row => row.layout.length)).length =
      (slot.rows.map Piece.line).length := by simp
  simp only [readSlot, lexical, Slot.lines, trims, level, splitLines_restore,
    len, List.drop_append_length, and_self]

theorem readSlot_readBack (spec : SlotSpec) (slot : Slot) (edits : List Splice)
    (head : slot.rows ≠ [])
    (placement : spec.placement = (plannedSlot headReads slot edits).placement)
    (lexical : Lexical (plannedSlot headReads slot edits)) :
    (readSlot spec (writeSlot headReads slot edits)).content = applyContent slot.content edits := by
  have h := (readSlot_placement spec (plannedSlot headReads slot edits) placement lexical).1
  unfold writeSlot Slot.content at *
  rw [h]
  exact plan_content headReads slot edits head

def linesEndInCR (lines : List Line) : Bool :=
  lines.getLast?.any (fun line => line.ending == .cr)

def Slot.endsInCR (slot : Slot) : Bool := linesEndInCR slot.lines

def planSlots (slots : List Slot) (edits : Nat → List Splice) (i : Nat) (priorCR : Bool := false) :
    List Slot :=
  match slots with
  | [] => []
  | slot :: rest =>
    plannedSlot headReads slot (edits i) priorCR :: planSlots rest edits (i + 1) slot.endsInCR

def planPage (page : Page) (edits : Nat → List Splice) : Page :=
  { page with slots := planSlots headReads page.slots edits 0 (linesEndInCR page.preamble) }

def write (page : Page) (edits : Nat → List Splice) : Bytes := (planPage headReads page edits).bytes

theorem renderPlan_nil (slots : List Slot) (i : Nat) (priorCR : Bool) :
    (planSlots headReads slots (fun _ => []) i priorCR).flatMap Slot.bytes = slots.flatMap Slot.bytes := by
  induction slots generalizing i priorCR with
  | nil => rfl
  | cons slot rest ih =>
    simp only [planSlots, List.flatMap_cons, ih]
    rw [← writeSlot, R1_slot headReads slot priorCR]

theorem R1 (outline : Outline) (bytes : Bytes) :
    write headReads (read outline bytes) (fun _ => []) = bytes := by
  simpa [write, planPage, Page.bytes, renderPlan_nil] using R0 outline bytes

/-- The new parser reports the same slot order, moved byte extents, levels, and removed widths.
    Matching removed widths includes which rows are trailer. Style for future writes is free. -/
def Outline.moves (outline : Outline) (plan : Page) : Prop :=
  outline.preamble = (plan.preamble.flatMap Line.bytes).length ∧
  outline.slots.map SlotSpec.placement = plan.slots.map Slot.placement

def layoutContent (slots : List Slot) : List (Nat × Bytes) :=
  slots.map fun slot => (slot.level, slot.content)

theorem readSlots_placement (specs : List SlotSpec) (slots : List Slot) (tail : Bytes)
    (moves : specs.map SlotSpec.placement = slots.map Slot.placement)
    (lexical : ∀ slot ∈ slots, Lexical slot) :
    layoutContent (readSlots specs (slots.flatMap Slot.bytes ++ tail)).1 = layoutContent slots := by
  induction specs generalizing slots with
  | nil =>
    cases slots <;> simp_all [readSlots, layoutContent]
  | cons spec specs ih =>
    cases slots with
    | nil => simp at moves
    | cons slot slots =>
      simp only [List.map_cons, List.cons.injEq] at moves
      have width := congrArg Prod.fst moves.1
      change spec.width = slot.bytes.length at width
      have h := readSlot_placement spec slot moves.1 (lexical slot (by simp))
      have rest := ih slots moves.2 (fun s hs => lexical s (by simp [hs]))
      simp only [List.flatMap_cons, List.append_assoc, readSlots, width, List.take_append_length,
        List.drop_append_length, layoutContent, List.map_cons, List.cons.injEq]
      exact ⟨by simp [h.2.2, Slot.content, h.1], rest⟩

theorem planSlots_content (slots : List Slot) (edits : Nat → List Splice) (i : Nat)
    (heads : ∀ slot ∈ slots, slot.rows ≠ []) (priorCR : Bool := false) :
    layoutContent (planSlots headReads slots edits i priorCR) =
      (slots.zipIdx i).map (fun (slot, index) => (slot.level, applyContent slot.content (edits index))) := by
  induction slots generalizing i priorCR with
  | nil => rfl
  | cons head tail ih =>
    have h := heads head (by simp)
    have hc := plan_content headReads head (edits i) h priorCR
    simp only [planSlots, layoutContent, List.zipIdx_cons, List.map_cons, List.cons.injEq,
      Prod.mk.injEq, plannedSlot_level, true_and]
    exact ⟨hc, ih _ (fun slot hs => heads slot (by simp [hs])) _⟩

/-- Levels and content in outline order after the physical write and an independent re-lex. -/
theorem readBack (before after : Outline) (bytes : Bytes) (edits : Nat → List Splice)
    (heads : ∀ slot ∈ (read before bytes).slots, slot.rows ≠ [])
    (moves : after.moves (planPage headReads (read before bytes) edits))
    (lexical : ∀ slot ∈ (planPage headReads (read before bytes) edits).slots, Lexical slot) :
    layoutContent (read after (write headReads (read before bytes) edits)).slots =
      ((read before bytes).slots.zipIdx).map
        (fun (slot, i) => (slot.level, applyContent slot.content (edits i))) := by
  rw [← planSlots_content headReads _ _ _ heads (linesEndInCR (read before bytes).preamble)]
  let plan := planPage headReads (read before bytes) edits
  change layoutContent (read after plan.bytes).slots = layoutContent plan.slots
  rcases moves with ⟨preamble, slots⟩
  change after.preamble = (plan.preamble.flatMap Line.bytes).length at preamble
  rw [read, Page.bytes, preamble]
  simpa only [List.append_assoc, List.drop_append_length]
    using readSlots_placement after.slots plan.slots (plan.unassigned.flatMap Line.bytes) slots lexical

/- Exact lexical criterion -/

def cleanBody (body : Bytes) : Prop := ∀ b ∈ body, b ≠ lf ∧ b ≠ cr

def endingOK (line : Line) (tail : List Line) : Prop :=
  match line.ending with
  | .eof => line.body ≠ [] ∧ tail = []
  | .cr => ∀ rest, tail ≠ ⟨[], .lf⟩ :: rest
  | .lf | .crlf => True

def lexShape : List Line → Prop
  | [] => True
  | line :: tail => cleanBody line.body ∧ endingOK line tail ∧ lexShape tail

theorem lexShape_prepend (b : UInt8) (lines : List Line) (shape : lexShape lines) :
    lexShape (prependByte b lines) := by
  unfold prependByte
  split <;> rename_i hLF
  · exact ⟨by simp [cleanBody], trivial, shape⟩
  split <;> rename_i hCR
  · split
    · exact ⟨by simp [cleanBody], trivial, shape.2.2⟩
    · rename_i _ avoid
      exact ⟨by simp [cleanBody], avoid, shape⟩
  · cases lines with
    | nil => simp [lexShape, cleanBody, endingOK, hLF, hCR]
    | cons line tail =>
      rcases shape with ⟨body, ending, rest⟩
      refine ⟨?_, ?_, rest⟩
      · simpa [cleanBody] using And.intro (And.intro hLF hCR) body
      · cases he : line.ending <;> simp_all [endingOK]

theorem lex_shape (bytes : Bytes) : lexShape (lex bytes) := by
  induction bytes with
  | nil => trivial
  | cons b rest ih => exact lexShape_prepend b (lex rest) ih

theorem lex_clean_prefix (body bytes : Bytes) (head : Line) (tail : List Line)
    (clean : cleanBody body) (h : lex bytes = head :: tail) :
    lex (body ++ bytes) = { head with body := body ++ head.body } :: tail := by
  induction body with
  | nil => exact h
  | cons b rest ih =>
    have hb := clean b (by simp)
    have ht : cleanBody rest := fun v hv => clean v (by simp [hv])
    simp [lex, ih ht, prependByte, hb.1, hb.2]

theorem lex_clean_eof (body : Bytes) (nonempty : body ≠ []) (clean : cleanBody body) :
    lex body = [⟨body, .eof⟩] := by
  induction body with
  | nil => contradiction
  | cons b rest ih =>
    have hb := clean b (by simp)
    have ht : cleanBody rest := fun v hv => clean v (by simp [hv])
    cases rest with
    | nil => simp [lex, prependByte, hb.1, hb.2]
    | cons c rest =>
      rw [lex, ih (by simp) ht]
      simp [prependByte, hb.1, hb.2]

theorem lex_at_break (e : Ending) (lines : List Line) (h : e ≠ .eof)
    (ok : endingOK ⟨[], e⟩ lines) (bytes : Bytes) (known : lex bytes = lines) :
    lex (e.bytes ++ bytes) = ⟨[], e⟩ :: lines := by
  cases e with
  | eof => contradiction
  | lf => simp [Ending.bytes, lex, prependByte, known]
  | crlf => simp [Ending.bytes, lex, prependByte, known, cr, lf]
  | cr =>
    change prependByte BlockText.cr (lex bytes) = _
    rw [known]
    change (match lines with
      | ⟨[], .lf⟩ :: rest => Line.mk [] .crlf :: rest
      | _ => Line.mk [] .cr :: lines) = _
    split
    · rename_i rest
      exact False.elim (ok rest rfl)
    · rfl

theorem lex_render (lines : List Line) (shape : lexShape lines) :
    lex (lines.flatMap Line.bytes) = lines := by
  induction lines with
  | nil => rfl
  | cons line tail ih =>
    rcases shape with ⟨clean, ending, rest⟩
    have eq := ih rest
    by_cases e : line.ending = .eof
    · simp [endingOK, e] at ending
      rcases ending with ⟨nonempty, rfl⟩
      cases line with
      | mk body ending =>
        change ending = .eof at e
        subst ending
        simpa [Line.bytes, Ending.bytes] using lex_clean_eof body nonempty clean
    · have h : endingOK ⟨[], line.ending⟩ tail := by
        cases he : line.ending <;> simp_all [endingOK]
      have step := lex_at_break line.ending tail e h (tail.flatMap Line.bytes) eq
      have pref := lex_clean_prefix line.body _ _ tail clean step
      cases line
      simpa [Line.bytes, List.append_assoc] using pref

theorem lexical_iff (slot : Slot) : Lexical slot ↔ lexShape slot.lines := by
  constructor
  · intro h
    rw [← h]
    exact lex_shape slot.bytes
  · intro h
    rw [Lexical, ← lines_bytes]
    exact lex_render _ h


theorem splitLine_line (width : Nat) (line : Line) : (splitLine width line).line = line := by
  cases line
  simp [splitLine, Piece.line]

theorem splitLines_lines (trims : List Nat) (lines : List Line) :
    (List.zipWith splitLine trims lines).map Piece.line ++ lines.drop trims.length = lines := by
  induction trims generalizing lines with
  | nil => simp
  | cons w ws ih =>
    cases lines with
    | nil => simp
    | cons line rest =>
      simp only [List.zipWith_cons_cons, List.map_cons, List.length_cons,
        List.drop_succ_cons, List.cons_append, splitLine_line, ih]

theorem readSlot_lines (spec : SlotSpec) (bytes : Bytes) :
    (readSlot spec bytes).lines = lex bytes :=
  splitLines_lines spec.trims (lex bytes)

theorem readSlot_lexical (spec : SlotSpec) (bytes : Bytes) : Lexical (readSlot spec bytes) := by
  rw [Lexical, readSlot_lossless, readSlot_lines]

theorem readSlot_exact_iff (spec : SlotSpec) (slot : Slot)
    (placement : spec.placement = slot.placement) :
    ((readSlot spec slot.bytes).rows = slot.rows ∧
      (readSlot spec slot.bytes).trailer = slot.trailer) ↔ lexShape slot.lines := by
  rw [← lexical_iff]
  constructor
  · rintro ⟨rows, trailer⟩
    rw [Lexical, ← readSlot_lines spec slot.bytes, Slot.lines, rows, trailer]
    rfl
  · intro h
    have eq := readSlot_placement spec slot placement h
    exact ⟨eq.1, eq.2.1⟩

/- Input characterization for the repaired plan -/

theorem cleanBody_append (a b : Bytes) :
    cleanBody (a ++ b) ↔ cleanBody a ∧ cleanBody b := by
  simp only [cleanBody, List.mem_append]
  constructor
  · intro h
    exact ⟨fun c hc => h c (Or.inl hc), fun c hc => h c (Or.inr hc)⟩
  · rintro ⟨ha, hb⟩ c (hc | hc)
    · exact ha c hc
    · exact hb c hc

@[simp] theorem cleanBody_single (b : UInt8) :
    cleanBody [b] ↔ b ≠ lf ∧ b ≠ cr := by simp [cleanBody]

def Cell.clean : Cell → Prop
  | .text b => b ≠ lf ∧ b ≠ cr
  | .newline e indent => e ≠ .eof ∧ cleanBody indent

/-- Physical bodies of a cell stream, independent of how each body is divided into layout and text. -/
def streamLines (body : Bytes) (ending : Ending) : List Cell → List Line
  | [] => [⟨body, ending⟩]
  | .text b :: rest => streamLines (body ++ [b]) ending rest
  | .newline e indent :: rest => ⟨body, e⟩ :: streamLines indent ending rest

theorem assemble_lines (layout : Bytes) (ending : Ending) (stream : List Cell) :
    (assembleRows layout ending stream).map Piece.line = streamLines layout ending stream := by
  induction stream generalizing layout with
  | nil => simp [assembleRows, assemble, Piece.line, streamLines]
  | cons cell rest ih =>
    cases cell with
    | text b =>
      simp only [streamLines, ← ih, assembleRows, assemble, List.map_cons, Piece.line,
        List.append_assoc, List.singleton_append]
    | newline e indent =>
      simp [streamLines, ← ih, assembleRows, assemble, Piece.line]

theorem streamLines_nonempty (body : Bytes) (e : Ending) (stream : List Cell) :
    streamLines body e stream ≠ [] := by
  induction stream generalizing body with
  | nil => simp [streamLines]
  | cons cell rest ih =>
    cases cell with
    | text b => exact ih (body ++ [b])
    | newline => simp [streamLines]

theorem streamLines_text (layout text : Bytes) (ending : Ending) (stream : List Cell) :
    streamLines layout ending (text.map Cell.text ++ stream) =
      streamLines (layout ++ text) ending stream := by
  induction text generalizing layout with
  | nil => simp
  | cons b rest ih =>
    simp [streamLines, ih, List.append_assoc]

theorem streamLines_cells (row : Piece) (rows : List Piece) :
    streamLines row.layout (finalEnding (row :: rows)) (cells row rows) =
      (row :: rows).map Piece.line := by
  induction rows generalizing row with
  | nil =>
    simp [cells, finalEnding]
    have h := streamLines_text row.layout row.text row.ending []
    simpa [streamLines, Piece.line] using h
  | cons next rest ih =>
    simp only [cells, streamLines_text, streamLines, List.map_cons, Piece.line]
    have last : finalEnding (row :: next :: rest) = finalEnding (next :: rest) := by
      simp [finalEnding]
    rw [last, ih next]
    rfl

/-- Whether a prior bare CR may be followed by these rows. -/
def headOK (prevCR : Bool) (lines : List Line) : Prop :=
  prevCR = true → ∀ rest, lines ≠ ⟨[], .lf⟩ :: rest

theorem headOK_cons (p : Bool) (body : Bytes) (e : Ending) (rest : List Line) :
    headOK p (⟨body, e⟩ :: rest) ↔ (p = true → body = [] → e ≠ .lf) := by
  constructor
  · intro h hp hb he
    exact h hp rest (by simp [hb, he])
  · intro h hp tail eq
    have sides := List.cons.inj eq
    exact h hp (congrArg Line.body sides.1) (congrArg Line.ending sides.1)

theorem headOK_body (p : Bool) (body : Bytes) (ending : Ending)
    (stream : List Cell) (trailer : List Line) (nonempty : body ≠ []) :
    headOK p (streamLines body ending stream ++ trailer) := by
  induction stream generalizing body with
  | nil => simp [streamLines, headOK_cons, nonempty]
  | cons cell rest ih =>
    cases cell with
    | text b => exact ih _ (by simp)
    | newline e indent => simp [streamLines, headOK_cons, nonempty]

theorem endingOK_headOK (body : Bytes) (e : Ending) (rest : List Line) (he : e ≠ .eof) :
    endingOK ⟨body, e⟩ rest ↔ headOK (e == .cr) rest := by
  cases e <;> simp_all [endingOK, headOK]

/-- A final ending can be followed by this unchanged trailer before and after editing. -/
def trailerFits (e : Ending) (trailer : List Line) : Prop :=
  (e = .eof → trailer = []) ∧ headOK (e == .cr) trailer

theorem repairEnding_facts (p : Bool) (e : Ending) :
    (repairEnding p e == .cr) = (e == .cr) ∧
    (repairEnding p e = .eof ↔ e = .eof) ∧
    (p = true → repairEnding p e ≠ .lf) := by
  cases p <;> cases e <;> decide

/-- Whether the input splices leave the final body without any byte, including layout. -/
def lastBodyEmpty (bodyEmpty : Bool) : List Cell → Bool
  | [] => bodyEmpty
  | .text _ :: rest => lastBodyEmpty false rest
  | .newline _ indent :: rest => lastBodyEmpty indent.isEmpty rest

/-- The previous-row guard is included so induction can enter immediately after any break. -/
theorem repair_shape (p : Bool) (body : Bytes) (e : Ending) (stream : List Cell)
    (trailer : List Line) (bodyClean : cleanBody body)
    (streamClean : ∀ cell ∈ stream, Cell.clean cell) (fits : trailerFits e trailer)
    (tailShape : lexShape trailer) :
    let lines := streamLines body
      (repairEnding (exitCR (p && body.isEmpty) stream) e)
      (repairCells (p && body.isEmpty) stream) ++ trailer
    headOK p lines ∧ (lexShape lines ↔ ¬ (e = .eof ∧ lastBodyEmpty body.isEmpty stream = true)) := by
  induction stream generalizing p body with
  | nil =>
    dsimp [repairCells, exitCR, streamLines, lastBodyEmpty]
    let final := repairEnding (p && body.isEmpty) e
    have facts := repairEnding_facts (p && body.isEmpty) e
    change headOK p (⟨body, final⟩ :: trailer) ∧ _
    refine ⟨(headOK_cons ..).mpr (fun hp hb => facts.2.2 (by simp [hp, hb])), ?_⟩
    change (cleanBody body ∧ endingOK ⟨body, final⟩ trailer ∧ lexShape trailer) ↔ _
    rw [and_iff_right bodyClean, and_iff_left tailShape]
    by_cases he : e = .eof
    · have hf : final = .eof := facts.2.1.mpr he
      simp [hf, he, endingOK, fits.1 he]
    · have hf : final ≠ .eof := fun h => he (facts.2.1.mp h)
      rw [endingOK_headOK _ _ _ hf, facts.1]
      simp [he, fits.2]
  | cons cell rest ih =>
    have hc := streamClean cell (by simp)
    have hr : ∀ c ∈ rest, c.clean := fun c mem => streamClean c (by simp [mem])
    cases cell with
    | text b =>
      have h := ih false (body ++ [b]) ((cleanBody_append ..).mpr ⟨bodyClean, (cleanBody_single _).mpr hc⟩) hr
      have nonempty : (body ++ [b]).isEmpty = false := by simp
      simp only [repairCells, Cell.repair, Cell.afterCR, exitCR, streamLines, lastBodyEmpty,
        Bool.false_and, nonempty] at h ⊢
      exact ⟨headOK_body p _ _ _ _ (by simp), h.2⟩
    | newline br indent =>
      rcases hc with ⟨he, hi⟩
      have h := ih (br == .cr) indent hi hr
      have state : (Cell.newline br indent).afterCR = ((br == .cr) && indent.isEmpty) := by
        cases br <;> cases indent <;> rfl
      simp only [repairCells, Cell.repair, exitCR, streamLines, lastBodyEmpty, state, List.cons_append]
      let first := repairEnding (p && body.isEmpty) br
      have facts := repairEnding_facts (p && body.isEmpty) br
      have nonEOF : first ≠ .eof := fun hf => he (facts.2.1.mp hf)
      refine ⟨(headOK_cons ..).mpr (fun hp hb => facts.2.2 (by simp [hp, hb])), ?_⟩
      rw [lexShape, and_iff_right bodyClean, endingOK_headOK _ _ _ nonEOF, facts.1,
        and_iff_right h.1]
      exact h.2

theorem repairEnding_eq (p : Bool) (body : Bytes) (e : Ending) (rest : List Line)
    (guard : headOK p (⟨body, e⟩ :: rest)) :
    repairEnding (p && body.isEmpty) e = e := by
  unfold repairEnding
  split
  · rename_i h
    simp only [Bool.and_eq_true, List.isEmpty_iff, beq_iff_eq] at h
    exact ((headOK_cons ..).mp guard h.1.1 h.1.2 h.2).elim
  · rfl

/-- Source lexical shape supplies clean cells, original framing, and the absence of any
    would-be repair when the snapshot is serialized without edits. -/
theorem stream_source (p : Bool) (body : Bytes) (e : Ending) (stream : List Cell)
    (trailer : List Line)
    (shape : lexShape (streamLines body e stream ++ trailer))
    (guard : headOK p (streamLines body e stream ++ trailer)) :
    cleanBody body ∧ (∀ c ∈ stream, c.clean) ∧ trailerFits e trailer ∧ lexShape trailer ∧
      repairCells (p && body.isEmpty) (stream ++ [.newline e []]) =
        stream ++ [.newline e []] := by
  induction stream generalizing p body with
  | nil =>
    rcases shape with ⟨clean, endOK, tail⟩
    have fits : trailerFits e trailer := by
      cases e <;> simp_all [trailerFits, endingOK, headOK]
    exact ⟨clean, by simp, fits, tail, by
      simp [repairCells, Cell.repair, repairEnding_eq p body e trailer guard]⟩
  | cons cell rest ih =>
    cases cell with
    | text b =>
      have h := ih false (body ++ [b]) shape (by simp [headOK])
      obtain ⟨clean, all, fits, tail, eq⟩ := h
      rw [cleanBody_append, cleanBody_single] at clean
      exact ⟨clean.1, by simpa [Cell.clean] using And.intro clean.2 all, fits, tail, by
        simpa [repairCells, Cell.repair, Cell.afterCR] using
          congrArg (List.cons (.text b)) eq⟩
    | newline br indent =>
      rcases shape with ⟨clean, endOK, tailShape⟩
      have nonEOF : br ≠ .eof := by
        intro h
        have impossible : streamLines indent e rest ++ trailer = [] := by
          have ok := endOK
          rw [h] at ok
          exact ok.2
        exact streamLines_nonempty indent e rest (List.append_eq_nil_iff.mp impossible).1
      have tailGuard := (endingOK_headOK _ _ _ nonEOF).mp endOK
      obtain ⟨indentClean, all, fits, tail, eq⟩ := ih (br == .cr) indent tailShape tailGuard
      have state : (Cell.newline br indent).afterCR = ((br == .cr) && indent.isEmpty) := by
        cases br <;> cases indent <;> rfl
      have first := repairEnding_eq p body br _ guard
      exact ⟨clean, by simpa [Cell.clean] using And.intro (And.intro nonEOF indentClean) all,
        fits, tail, by simp [repairCells, Cell.repair, first, state, eq]⟩

theorem slot_source (slot : Slot) (head : slot.rows ≠ []) (lexical : Lexical slot)
    (p : Bool := false) (guard : headOK p slot.lines := by simp [headOK]) :
    cleanBody (front slot.rows) ∧ (∀ c ∈ rowCells slot.rows, c.clean) ∧
      trailerFits (finalEnding slot.rows) slot.trailer ∧ lexShape slot.trailer ∧
      repairCells (p && (front slot.rows).isEmpty)
          (rowCells slot.rows ++ [.newline (finalEnding slot.rows) []]) =
        rowCells slot.rows ++ [.newline (finalEnding slot.rows) []] := by
  have shape := (lexical_iff slot).mp lexical
  cases rows : slot.rows with
  | nil => exact (head rows).elim
  | cons row rest =>
    apply stream_source p row.layout _ _ slot.trailer
    · simpa only [rowCells, streamLines_cells, Slot.lines, rows] using shape
    · simpa only [rowCells, streamLines_cells, Slot.lines, rows] using guard

theorem partition_all_after (good : α → Prop) (encode : Splice → List α)
    (source : List α) (cursor : Nat) (edits : List Splice)
    (old : ∀ c ∈ source, good c) (new : ∀ e ∈ edits, ∀ c ∈ encode e, good c) :
    ∀ c ∈ (partition encode source cursor edits).flatMap Chunk.after, good c := by
  induction edits generalizing source cursor with
  | nil => simpa [partition, Chunk.after] using old
  | cons e rest ih =>
    simp only [partition, Chunk.after, List.flatMap_cons, List.mem_append]
    rintro c ((kept | put) | tail)
    · exact old c (List.mem_of_mem_take kept)
    · exact new e (by simp) c put
    · exact ih _ _ (fun d hd => old d (List.mem_of_mem_drop (List.mem_of_mem_drop hd)))
        (fun edit he => new edit (by simp [he])) c tail

def CRFree (edits : List Splice) : Prop := ∀ edit ∈ edits, cr ∉ edit.replacement

theorem encode_clean (slot : Slot) (edit : Splice) (indent : cleanBody slot.continuation)
    (free : cr ∉ edit.replacement) : ∀ c ∈ encode slot edit, c.clean := by
  simp only [encode, List.mem_map]
  rintro _ ⟨b, hb, rfl⟩
  split <;> rename_i h
  · simp only [Cell.clean, and_iff_left indent]
    cases followingEnding (finalEnding slot.rows) (List.drop edit.start (rowCells slot.rows)) <;>
      decide
  · exact ⟨h, fun eq => free (eq ▸ hb)⟩

theorem lastBodyEmpty_separator (slot : Slot) (first : Option Splice) (stream : List Cell) :
    lastBodyEmpty (front slot.rows ++ separator headReads slot first stream).isEmpty stream =
      lastBodyEmpty (front slot.rows).isEmpty stream := by
  cases stream with
  | nil => cases first <;> simp [separator, firstRowText, lastBodyEmpty]
  | cons cell rest => cases cell <;> rfl

/-- All quantities come from the snapshot and the requested splices. EOF here is end of slice. -/
def EmptySliceEnd (slot : Slot) (edits : List Splice) : Prop :=
  let stream := editedCells slot edits
  finalEnding slot.rows = .eof ∧
    lastBodyEmpty (front slot.rows).isEmpty stream = true

theorem editedSlot_shape (slot : Slot) (edits : List Splice)
    (head : slot.rows ≠ []) (lexical : Lexical slot) (indent : cleanBody slot.continuation)
    (free : CRFree edits) (priorCR : Bool) (edited : edits ≠ []) :
    headOK priorCR (plannedSlot headReads slot edits priorCR).lines ∧
      (lexShape (plannedSlot headReads slot edits priorCR).lines ↔ ¬ EmptySliceEnd slot edits) := by
  cases edits with
  | nil => contradiction
  | cons edit rest =>
    obtain ⟨layout, cells, fits, trailer, _⟩ := slot_source slot head lexical
    let edits := edit :: rest
    let stream := editedCells slot edits
    have streamClean : ∀ c ∈ stream, c.clean :=
      partition_all_after Cell.clean (encode slot) (rowCells slot.rows) 0 (ordered edits) cells
        (fun e he => encode_clean slot e indent (free e (List.mem_mergeSort.mp he)))
    have sepClean : cleanBody (separator headReads slot (ordered edits).head? stream) := by
      unfold separator
      split
      · simp [cleanBody]
      · dsimp only
        split
        · exact (cleanBody_single _).mpr (by decide)
        · simp [cleanBody]
    have layoutClean := (cleanBody_append ..).mpr ⟨layout, sepClean⟩
    have shape := (repair_shape priorCR _ (finalEnding slot.rows) stream slot.trailer
      layoutClean streamClean fits trailer)
    simpa [Slot.lines, plannedSlot, head, assemble_lines, EmptySliceEnd, lastBodyEmpty_separator,
      edits, stream, editedCells, chunks] using shape

theorem repairedShape_iff (slot : Slot) (edits : List Splice)
    (head : slot.rows ≠ []) (lexical : Lexical slot) (indent : cleanBody slot.continuation)
    (free : CRFree edits) (priorCR : Bool := false) :
    lexShape (plannedSlot headReads slot edits priorCR).lines ↔ edits = [] ∨ ¬ EmptySliceEnd slot edits := by
  by_cases edited : edits = []
  · simpa [edited] using (lexical_iff slot).mp lexical
  · simpa only [edited, false_or] using
      (editedSlot_shape headReads slot edits head lexical indent free priorCR edited).2

theorem partition_put_mem (encode : Splice → List α) (source : List α) (cursor : Nat)
    (edits : List Splice) {edit : Splice} (mem : edit ∈ edits) {c : α} (put : c ∈ encode edit) :
    c ∈ (partition encode source cursor edits).flatMap Chunk.after := by
  induction edits generalizing source cursor with
  | nil => simp at mem
  | cons e rest ih =>
    simp only [partition, Chunk.after, List.flatMap_cons, List.mem_append]
    rcases List.mem_cons.mp mem with rfl | tail
    · exact Or.inl (Or.inr put)
    · exact Or.inr (ih _ _ tail)

theorem repairCells_text_mem (p : Bool) (stream : List Cell) (b : UInt8)
    (mem : Cell.text b ∈ stream) : Cell.text b ∈ repairCells p stream := by
  induction stream generalizing p with
  | nil => simp at mem
  | cons c rest ih =>
    simp only [repairCells, List.mem_cons]
    rcases List.mem_cons.mp mem with rfl | tail
    · exact Or.inl rfl
    · exact Or.inr (ih _ tail)

theorem repairedShape_CRFree (slot : Slot) (edits : List Splice) (head : slot.rows ≠ [])
    (priorCR : Bool) (shape : lexShape (plannedSlot headReads slot edits priorCR).lines) : CRFree edits := by
  cases edits with
  | nil => simp [CRFree]
  | cons edit rest =>
    let edits := edit :: rest
    let stream := editedCells slot edits
    let layout := front slot.rows ++ separator headReads slot (ordered edits).head? stream
    let adjacent := priorCR && layout.isEmpty
    have physical : lexShape (streamLines layout
        (repairEnding (exitCR adjacent stream) (finalEnding slot.rows)) (repairCells adjacent stream) ++
          slot.trailer) := by
      simpa [Slot.lines, plannedSlot, head, assemble_lines, edits, stream, layout, adjacent,
        editedCells, chunks] using shape
    have good := (stream_source false _ _ _ _ physical (by simp [headOK])).2.1
    intro e mem bad
    have literal : Cell.text cr ∈ encode slot e := by
      unfold encode
      exact List.mem_map.mpr ⟨cr, bad, rfl⟩
    have retained : Cell.text cr ∈ stream :=
      partition_put_mem (encode slot) _ 0 _ (List.mem_mergeSort.mpr mem) literal
    exact (good (.text cr) (repairCells_text_mem adjacent stream cr retained)).2 rfl

/- Assign each new CR to a replacement or to the right endpoint of a splice. -/

theorem repairCells_append (p : Bool) (a b : List Cell) :
    repairCells p (a ++ b) = repairCells p a ++ repairCells (exitCR p a) b := by
  induction a generalizing p with
  | nil => rfl
  | cons c a ih => simp [repairCells, exitCR, ih]

theorem exitCR_append (p : Bool) (a b : List Cell) :
    exitCR p (a ++ b) = exitCR (exitCR p a) b := by
  induction a generalizing p with
  | nil => rfl
  | cons c a ih => exact ih c.afterCR

def entryRepair (p : Bool) : List Cell → Bytes
  | .newline .lf _ :: _ => if p then [cr] else []
  | _ => []

theorem repairCells_entry (p : Bool) (run : List Cell) :
    (repairCells p run).flatMap Cell.raw =
      entryRepair p run ++ (repairCells false run).flatMap Cell.raw := by
  cases p <;> cases run with
  | nil => rfl
  | cons c rest =>
    cases c with
    | text => rfl
    | newline e indent => cases e <;> rfl

theorem repairCells_false (p : Bool) (run : List Cell)
    (same : repairCells p run = run) : repairCells false run = run := by
  cases run with
  | nil => rfl
  | cons c rest =>
    have tail := (List.cons.inj same).2
    have first : c.repair false = c := by
      cases c <;> rfl
    simp only [repairCells, first, tail]

theorem repairCells_length (p : Bool) (run : List Cell) : (repairCells p run).length = run.length := by
  induction run generalizing p with
  | nil => rfl
  | cons c rest ih => simp [repairCells, ih]

theorem repairCells_split (p : Bool) (a b : List Cell)
    (same : repairCells p (a ++ b) = a ++ b) :
    repairCells p a = a ∧ repairCells (exitCR p a) b = b := by
  rw [repairCells_append] at same
  exact List.append_inj same (repairCells_length p a)

theorem keeps_safe (cs : List (Chunk Cell)) (p : Bool)
    (safe : repairCells p (cs.flatMap Chunk.before) = cs.flatMap Chunk.before) :
    ∀ c ∈ cs, repairCells false c.keep = c.keep := by
  induction cs generalizing p with
  | nil => simp
  | cons c rest ih =>
    simp only [List.flatMap_cons, Chunk.before, List.append_assoc] at safe
    obtain ⟨keep, tail⟩ := repairCells_split p c.keep _ safe
    obtain ⟨_, remaining⟩ := repairCells_split _ c.cut _ tail
    simp only [List.mem_cons]
    rintro d (rfl | mem)
    · exact repairCells_false p d.keep keep
    · exact ih _ remaining d mem

def firstKeep (cs : List (Chunk Cell)) : List Cell :=
  (cs.head?.map Chunk.keep).getD []

theorem firstKeep_safe (p : Bool) (cs : List (Chunk Cell))
    (safe : repairCells p (cs.flatMap Chunk.before) = cs.flatMap Chunk.before) :
    entryRepair p (firstKeep cs) = [] := by
  cases cs with
  | nil => rfl
  | cons c rest =>
    simp only [List.flatMap_cons, Chunk.before, List.append_assoc] at safe
    have same := (repairCells_split p c.keep _ safe).1
    change entryRepair p c.keep = []
    have identity := repairCells_entry p c.keep
    rw [same, repairCells_false p c.keep same] at identity
    have lengths := congrArg List.length identity
    simp at lengths ⊢
    exact lengths

/-- The final chunk has no splice. Every other chunk puts the next kept run's leading
    repair at the end of its own replacement, before the first retained byte. -/
def bytePatches (p : Bool) : List (Chunk Cell) → List (Chunk UInt8)
  | [] => []
  | c :: rest =>
    let afterKeep := exitCR p c.keep
    let afterPut := exitCR afterKeep c.put
    ⟨c.keep.flatMap Cell.raw, c.cut.flatMap Cell.raw,
      (repairCells afterKeep c.put).flatMap Cell.raw ++ entryRepair afterPut (firstKeep rest)⟩ ::
      bytePatches afterPut rest

theorem bytePatches_before (p : Bool) (cs : List (Chunk Cell)) :
    (bytePatches p cs).flatMap Chunk.before = (cs.flatMap Chunk.before).flatMap Cell.raw := by
  induction cs generalizing p with
  | nil => rfl
  | cons c cs ih => simp [bytePatches, Chunk.before, ih, List.append_assoc]

theorem bytePatches_cuts (p : Bool) (cs : List (Chunk Cell)) :
    (bytePatches p cs).map Chunk.cut = cs.map (fun c => c.cut.flatMap Cell.raw) := by
  induction cs generalizing p with
  | nil => rfl
  | cons c cs ih => simp [bytePatches, ih]

theorem bytePatches_after (p : Bool) (cs : List (Chunk Cell))
    (safe : ∀ c ∈ cs, repairCells false c.keep = c.keep) :
    (repairCells p (cs.flatMap Chunk.after)).flatMap Cell.raw =
      entryRepair p (firstKeep cs) ++ (bytePatches p cs).flatMap Chunk.after := by
  induction cs generalizing p with
  | nil => rfl
  | cons c rest ih =>
    simp only [List.flatMap_cons, Chunk.after, repairCells_append, exitCR_append, List.flatMap_append]
    rw [repairCells_entry p c.keep, safe c (by simp),
      ih _ (fun d hd => safe d (by simp [hd]))]
    simp [bytePatches, firstKeep, Chunk.after, List.append_assoc]

theorem OrderedWithin.limit_mono {a b cursor : Nat} {edits : List Splice}
    (valid : OrderedWithin a cursor edits) (le : a ≤ b) : OrderedWithin b cursor edits := by
  induction edits generalizing cursor with
  | nil => exact Nat.le_trans valid le
  | cons e rest ih => exact ⟨valid.1, valid.2.1, ih valid.2.2⟩

theorem partition_after_suffix (encode : Splice → List Cell) (source suffix : List Cell)
    (cursor : Nat) (edits : List Splice) (valid : OrderedWithin (source.length + cursor) cursor edits) :
    (partition encode (source ++ suffix) cursor edits).flatMap Chunk.after =
      (partition encode source cursor edits).flatMap Chunk.after ++ suffix := by
  induction edits generalizing source cursor with
  | nil => simp [partition, Chunk.after]
  | cons e rest ih =>
    obtain ⟨start, stop, tail⟩ := valid
    have eBound := tail.cursor_le
    have keep : e.start - cursor ≤ source.length := by omega
    have cut : e.stop - e.start ≤ (source.drop (e.start - cursor)).length := by
      simp only [List.length_drop]
      omega
    have length :
        ((source.drop (e.start - cursor)).drop (e.stop - e.start)).length + e.stop =
          source.length + cursor := by
      simp only [List.length_drop]
      omega
    have next := ih ((source.drop (e.start - cursor)).drop (e.stop - e.start)) e.stop
      (by rw [length]; exact tail)
    simp only [partition, List.take_append_of_le_length keep, List.drop_append_of_le_length keep,
      List.drop_append_of_le_length cut, List.flatMap_cons, Chunk.after, next, List.append_assoc]

/-- Frame the final break as part of the terminal kept run, where it can acquire a CR
    immediately before it. Its original bytes and every trailer byte are retained. -/
def framedChunks (slot : Slot) (edits : List Splice) : List (Chunk Cell) :=
  partition (encode slot) (rowCells slot.rows ++ [.newline (finalEnding slot.rows) []])
    0 (ordered edits)

theorem render_final_token (p : Bool) (stream : List Cell) (e : Ending) :
    (repairCells p (stream ++ [.newline e []])).flatMap Cell.raw =
      (repairCells p stream).flatMap Cell.raw ++ (repairEnding (exitCR p stream) e).bytes := by
  simp [repairCells_append, repairCells, Cell.repair, Cell.raw]

/-- The repaired plan and snapshot have identical kept runs in the same order.
    The only writes are the put chunks and the header separator at the first gap. -/
theorem R3 (slot : Slot) (edits : List Splice) (head : slot.rows ≠ [])
    (lexical : Lexical slot) (valid : OrderedWithin slot.content.length 0 (ordered edits))
    (priorCR : Bool := false) (guard : headOK priorCR slot.lines := by simp [headOK]) :
    let layout := front slot.rows ++
      separator headReads slot (ordered edits).head? (editedCells slot edits)
    let patches := bytePatches (priorCR && layout.isEmpty) (framedChunks slot edits)
    slot.bytes = front slot.rows ++ patches.flatMap Chunk.before ++ slot.trailer.flatMap Line.bytes ∧
    writeSlot headReads slot edits priorCR = layout ++ patches.flatMap Chunk.after ++ slot.trailer.flatMap Line.bytes ∧
    (patches.map Chunk.cut).take (ordered edits).length =
      (ordered edits).map (fun e =>
        (slot.bytes.drop (slot.gap e.start)).take (slot.gap e.stop - slot.gap e.start)) := by
  let stream := rowCells slot.rows
  let final := Cell.newline (finalEnding slot.rows) []
  let original := stream ++ [final]
  let cs := framedChunks slot edits
  have before : cs.flatMap Chunk.before = original := partition_before _ _ _ _
  let layout := front slot.rows ++
    separator headReads slot (ordered edits).head? (editedCells slot edits)
  let adjacent := priorCR && layout.isEmpty
  have sourceSafe := (slot_source slot head lexical priorCR guard).2.2.2.2
  have safe : repairCells adjacent original = original := by
    by_cases immediate : adjacent = true
    · have sourceAdjacent : (priorCR && (front slot.rows).isEmpty) = true := by
        have h : priorCR = true ∧ layout = [] := by
          simpa only [adjacent, Bool.and_eq_true, List.isEmpty_iff] using immediate
        simpa [h.1, List.isEmpty_iff] using (List.append_eq_nil_iff.mp h.2).1
      rw [immediate]
      rw [sourceAdjacent] at sourceSafe
      exact sourceSafe
    · have state : adjacent = false := Bool.eq_false_iff.mpr immediate
      rw [state]
      exact repairCells_false _ _ sourceSafe
  have chunksSafe : repairCells adjacent (cs.flatMap Chunk.before) = cs.flatMap Chunk.before := by
    rw [before]
    exact safe
  have kept := keeps_safe cs adjacent chunksSafe
  have hbefore := bytePatches_before adjacent cs
  have hafter := bytePatches_after adjacent cs kept
  have entry := firstKeep_safe adjacent cs chunksSafe
  rw [entry, List.nil_append] at hafter
  simp only [Slot.content, List.length_map] at valid
  change OrderedWithin stream.length 0 (ordered edits) at valid
  have suffix := partition_after_suffix (encode slot) stream [final] 0 (ordered edits)
    (by simpa using valid)
  change cs.flatMap Chunk.after = editedCells slot edits ++ [final] at suffix
  change _ = _ ++ (bytePatches adjacent cs).flatMap Chunk.before ++ _ ∧
    _ = _ ++ _ ++ (bytePatches adjacent cs).flatMap Chunk.after ++ _ ∧ _
  refine ⟨?_, ?_, ?_⟩
  · rw [hbefore, before, slot_render]
    simp [original, stream, final, Cell.raw, List.append_assoc]
  · by_cases edited : edits = []
    · subst edits
      have same : editedCells slot [] = stream := by
        simp [editedCells, chunks, ordered, partition, Chunk.after, stream]
      have noSep : separator headReads slot (ordered []).head? (editedCells slot []) = [] := by
        simp [separator, ordered]
      rw [noSep, R1_slot headReads slot priorCR, ← hafter, suffix, same]
      change slot.bytes = _ ++ [] ++ (repairCells adjacent original).flatMap Cell.raw ++ _
      rw [safe, slot_render]
      simp [original, stream, final, Cell.raw, List.append_assoc]
    · rw [plan_render headReads _ _ head edited priorCR, ← hafter, suffix, render_final_token]
      simp only [layout, adjacent, List.append_assoc]
  · have bounded : OrderedWithin original.length 0 (ordered edits) :=
      valid.limit_mono (by simp [original])
    have cuts := partition_cut (encode slot) original 0 (ordered edits) bounded
    simp only [List.drop_zero] at cuts
    rw [bytePatches_cuts]
    change ((cs.map ((fun c : List Cell => c.flatMap Cell.raw) ∘ Chunk.cut)).take
      (ordered edits).length) = _
    rw [← List.map_map, ← List.map_take]
    change (((partition (encode slot) original 0 (ordered edits)).map Chunk.cut).take
      (ordered edits).length).map (fun c => c.flatMap Cell.raw) = _
    rw [cuts, List.map_map]
    apply List.map_congr_left
    intro e mem
    have hs := valid.ranges e mem
    have bounds := valid.stop_le e mem
    have start : e.start ≤ stream.length := Nat.le_trans hs bounds
    have length : e.stop - e.start ≤ (stream.drop e.start).length := by
      simp only [List.length_drop]
      omega
    simp only [Function.comp_def, original, List.drop_append_of_le_length start,
      List.take_append_of_le_length length]
    exact cut_bytes slot e.start e.stop hs

/-- Snapshot bytes before the deleted range of the indexed patch. -/
def beforePatch (cs : List (Chunk α)) (i : Nat) : List α :=
  (cs.take i).flatMap Chunk.before ++ (((cs.drop i).head?).map Chunk.keep).getD []

theorem beforePatch_succ (c : Chunk α) (cs : List (Chunk α)) (i : Nat) :
    beforePatch (c :: cs) (i + 1) = c.before ++ beforePatch cs i := by
  simp [beforePatch, List.append_assoc]

theorem OrderedWithin.start_ge {limit cursor : Nat} {edits : List Splice}
    (h : OrderedWithin limit cursor edits) (e : Splice) (mem : e ∈ edits) :
    cursor ≤ e.start := by
  induction edits generalizing cursor with
  | nil => simp at mem
  | cons d rest ih =>
    rcases List.mem_cons.mp mem with rfl | mem
    · exact h.1
    · have later := ih h.2.2 mem
      have before := h.1
      have interval := h.2.1
      omega

theorem take_gap (source : List α) {cursor start stop : Nat}
    (hstart : cursor ≤ start) (hstop : start ≤ stop) :
    (source.drop cursor).take (start - cursor) ++ (source.drop start).take (stop - start) =
      (source.drop cursor).take (stop - cursor) := by
  have split : stop - cursor = (start - cursor) + (stop - start) := by omega
  have near : cursor + (start - cursor) = start := by omega
  conv => rhs; rw [split, List.take_add, List.drop_drop, near]

theorem partition_patchStart (encode : Splice → List α) (source : List α) (cursor : Nat)
    (edits : List Splice) (valid : OrderedWithin limit cursor edits)
    {i : Nat} {edit : Splice} (found : edits[i]? = some edit) :
    beforePatch (partition encode (source.drop cursor) cursor edits) i =
      (source.drop cursor).take (edit.start - cursor) := by
  induction edits generalizing i cursor with
  | nil => simp at found
  | cons e rest ih =>
    obtain ⟨start, stop, tail⟩ := valid
    cases i with
    | zero =>
      simp only [List.getElem?_cons_zero, Option.some.injEq] at found
      subst edit
      simp [beforePatch, partition]
    | succ i =>
      simp only [List.getElem?_cons_succ] at found
      have a : cursor + (e.start - cursor) = e.start := by omega
      have b : e.start + (e.stop - e.start) = e.stop := by omega
      have c : e.stop ≤ edit.start := tail.start_ge edit (List.mem_of_getElem? found)
      rw [partition, beforePatch_succ]
      simp only [Chunk.before, List.drop_drop, a, b]
      rw [take_gap source start stop, ih e.stop tail found, take_gap source (by omega) c]

theorem bytePatches_prefix (p : Bool) (cs : List (Chunk Cell)) (i : Nat) :
    beforePatch (bytePatches p cs) i = (beforePatch cs i).flatMap Cell.raw := by
  induction cs generalizing p i with
  | nil => simp [bytePatches, beforePatch]
  | cons c rest ih =>
    cases i with
    | zero => simp [beforePatch, bytePatches]
    | succ i =>
      rw [bytePatches, beforePatch_succ, beforePatch_succ, ih, List.flatMap_append]
      simp [Chunk.before]

/-- Every replacement in the byte program starts at its own mapped snapshot gap.
    A leading repair for the next retained LF is the last byte of the preceding patch,
    so its source insertion point is that patch's right endpoint. -/
theorem R3_patchStart (slot : Slot) (edits : List Splice) (p : Bool)
    (valid : OrderedWithin slot.content.length 0 (ordered edits)) {i : Nat} {edit : Splice}
    (found : (ordered edits)[i]? = some edit) :
    let layout := front slot.rows ++
      separator headReads slot (ordered edits).head? (editedCells slot edits)
    let patches := bytePatches (p && layout.isEmpty) (framedChunks slot edits)
    (front slot.rows).length + (beforePatch patches i).length = slot.gap edit.start := by
  dsimp only
  rw [bytePatches_prefix]
  let source := rowCells slot.rows
  let frame := Cell.newline (finalEnding slot.rows) []
  have atStart := partition_patchStart (encode slot) (source ++ [frame]) 0 (ordered edits) valid found
  simp only [List.drop_zero, Nat.sub_zero] at atStart
  have within : edit.start ≤ source.length := by
    have hr : edit ∈ ordered edits := List.mem_of_getElem? found
    have b := valid.stop_le edit hr
    have order := valid.ranges edit hr
    simpa [Slot.content, source] using Nat.le_trans order b
  rw [framedChunks, atStart, List.take_append_of_le_length within, Slot.gap, offset_eq]

theorem bytePatches_terminal (p : Bool) (encode : Splice → List Cell) (source : List Cell)
    (cursor : Nat) (edits : List Splice) :
    ∃ kept, (bytePatches p (partition encode source cursor edits)).drop edits.length =
      [⟨kept, [], []⟩] := by
  induction edits generalizing p source cursor with
  | nil => exact ⟨source.flatMap Cell.raw, rfl⟩
  | cons e rest ih =>
    simp only [partition, bytePatches, List.length_cons, List.drop_succ_cons]
    exact ih _ _ _

/-- Fold the earned space into the first replacement without changing the rendered bytes. -/
theorem R3_separatorPatch (slot : Slot) (edits : List Splice) (head : slot.rows ≠ [])
    (lexical : Lexical slot) (valid : OrderedWithin slot.content.length 0 (ordered edits))
    (priorCR : Bool) (guard : headOK priorCR slot.lines)
    (inserted : separator headReads slot (ordered edits).head? (editedCells slot edits) ≠ []) :
    let sep := separator headReads slot (ordered edits).head? (editedCells slot edits)
    let patches := bytePatches (priorCR && (front slot.rows ++ sep).isEmpty) (framedChunks slot edits)
    ∃ edit first rest, (ordered edits).head? = some edit ∧ edit.start = 0 ∧
      patches = first :: rest ∧ first.keep = [] ∧
      writeSlot headReads slot edits priorCR =
        front slot.rows ++ ({ first with put := sep ++ first.put } :: rest).flatMap Chunk.after ++
          slot.trailer.flatMap Line.bytes := by
  dsimp only
  obtain ⟨edit, first, start⟩ := separator_firstPatch headReads slot edits valid inserted
  let sep := separator headReads slot (ordered edits).head? (editedCells slot edits)
  let layout := front slot.rows ++ sep
  let patches := bytePatches (priorCR && layout.isEmpty) (framedChunks slot edits)
  have keeps : patches.head?.map Chunk.keep = some [] := by
    cases order : ordered edits with
    | nil => simp [order] at first
    | cons e es =>
      simp only [order, List.head?_cons, Option.some.injEq] at first
      subst e
      simp [patches, framedChunks, order, partition, start, bytePatches]
  have render := (R3 headReads slot edits head lexical valid priorCR guard).2.1
  change writeSlot headReads slot edits priorCR =
    layout ++ patches.flatMap Chunk.after ++ slot.trailer.flatMap Line.bytes at render
  cases patchesEq : patches with
  | nil => simp [patchesEq] at keeps
  | cons patch rest =>
    have kept : patch.keep = [] := by simpa [patchesEq] using keeps
    rw [patchesEq] at render
    refine ⟨edit, patch, rest, first, start, patchesEq, kept, ?_⟩
    simpa [layout, sep, Chunk.after, kept, List.append_assoc] using render

/-- A parser may cut through a physical line body, but cannot split CRLF. -/
/- Admissible outlines and inverse byte addresses -/

def safeCut (file : Bytes) (pos : Nat) : Prop :=
  pos = 0 ∨ file[pos - 1]? ≠ some cr ∨ file[pos]? ≠ some lf

def atLineStart (file : Bytes) (pos : Nat) : Prop :=
  pos = 0 ∨ file[pos - 1]? = some lf ∨
    (file[pos - 1]? = some cr ∧ file[pos]? ≠ some lf)

def slotAdmissible (file : Bytes) (pos : Nat) (spec : SlotSpec) : Prop :=
  let lines := lex ((file.drop pos).take spec.width)
  safeCut file pos ∧ pos + spec.width ≤ file.length ∧ 0 < spec.width ∧
    spec.trims ≠ [] ∧ spec.trims.length ≤ lines.length ∧
    (spec.trims.zip lines).all (fun (w, line) => decide (w ≤ line.body.length)) ∧
    (¬atLineStart file pos → spec.trims.head? = some 0) ∧
    cleanBody spec.continuation

def admitSlots (file : Bytes) (pos : Nat) : List SlotSpec → Prop
  | [] => pos = file.length
  | spec :: rest => slotAdmissible file pos spec ∧ admitSlots file (pos + spec.width) rest

def Admissible (outline : Outline) (file : Bytes) : Prop :=
  outline.preamble ≤ file.length ∧ admitSlots file outline.preamble outline.slots

theorem cells_nonempty (row : Piece) (tail : List Piece) (trailer : List Line)
    (shape : lexShape ((row :: tail).map Piece.line ++ trailer))
    (cell : Cell) (mem : cell ∈ cells row tail) : cell.raw ≠ [] := by
  induction tail generalizing row with
  | nil =>
    simp [cells] at mem
    obtain ⟨b, _, rfl⟩ := mem
    simp [Cell.raw]
  | cons next tail ih =>
    simp only [cells, List.mem_append, List.mem_cons, List.mem_map] at mem
    rcases mem with ⟨b, _, rfl⟩ | rfl | hm
    · simp [Cell.raw]
    · cases he : row.ending with
      | eof =>
        have impossible := shape.2.1
        simp [endingOK, Piece.line, he] at impossible
      | lf | crlf | cr => simp [Cell.raw, Ending.bytes]
    · exact ih next shape.2.2 hm

theorem offset_step {stream : List Cell} {i : Nat} {cell : Cell}
    (h : stream[i]? = some cell) :
    offset stream (i + 1) = offset stream i + cell.raw.length := by
  induction stream generalizing i with
  | nil => simp at h
  | cons hd tl ih =>
    cases i with
    | zero => simp_all [offset]
    | succ i =>
      simp only [List.getElem?_cons_succ] at h
      simp only [offset, ih h, Nat.add_assoc]

theorem gap_step {slot : Slot} {i : Nat} {cell : Cell}
    (h : (rowCells slot.rows)[i]? = some cell) :
    slot.gap (i + 1) = slot.gap i + cell.raw.length := by
  simp only [Slot.gap, offset_step h, Nat.add_assoc]

theorem slot_cell_nonempty (slot : Slot) (lexical : Lexical slot) (cell : Cell)
    (mem : cell ∈ rowCells slot.rows) : cell.raw ≠ [] := by
  have shape := (lexical_iff slot).mp lexical
  cases rows : slot.rows with
  | nil => simp [rows, rowCells] at mem
  | cons row tail =>
    exact cells_nonempty row tail slot.trailer
      (by simpa only [Slot.lines, rows] using shape) cell
      (by simpa only [rows, rowCells] using mem)

theorem gap_strict (slot : Slot) (lexical : Lexical slot) {i j : Nat}
    (lt : i < j) (bound : j ≤ slot.content.length) : slot.gap i < slot.gap j := by
  have len : slot.content.length = (rowCells slot.rows).length := by simp [Slot.content]
  have hi : i < (rowCells slot.rows).length := by omega
  let cell := (rowCells slot.rows)[i]
  have h : (rowCells slot.rows)[i]? = some cell := List.getElem?_eq_getElem hi
  have mem : cell ∈ rowCells slot.rows := List.getElem_mem hi
  have positive : 0 < cell.raw.length := List.length_pos_iff.mpr (slot_cell_nonempty slot lexical cell mem)
  have step := gap_step h
  have endBound := gap_monotone slot (show i + 1 ≤ j by omega)
  omega

/-- A file text byte has a unique content offset, even if another byte has the same value. -/
theorem text_address_inverse (spec : SlotSpec) (bytes : Bytes) (i : Nat) (b : UInt8)
    (h : (rowCells (readSlot spec bytes).rows)[i]? = some (.text b)) :
    let slot := readSlot spec bytes
    slot.content[i]? = some b ∧
      (bytes.drop (slot.gap i)).take 1 = [b] ∧
      ∀ j, j ≤ slot.content.length → slot.gap j = slot.gap i → j = i := by
  let slot := readSlot spec bytes
  obtain ⟨ib, _⟩ := List.getElem?_eq_some_iff.mp h
  have len : slot.content.length = (rowCells slot.rows).length := by simp [Slot.content]
  have bound : i ≤ slot.content.length := by
    change i < (rowCells slot.rows).length at ib
    omega
  have inv := content_text_inverse slot h
  rw [readSlot_lossless] at inv
  refine ⟨inv.1, inv.2, ?_⟩
  intro j hj eq
  have lexical := readSlot_lexical spec bytes
  rcases Nat.lt_trichotomy j i with lt | rfl | gt
  · exact (Nat.ne_of_lt (gap_strict slot lexical lt bound) eq).elim
  · rfl
  · exact (Nat.ne_of_lt (gap_strict slot lexical gt hj) eq.symm).elim

def renderSlots (slots : List Slot) (edits : Nat → List Splice) (i : Nat) (priorCR : Bool) :
    Bytes :=
  match slots with
  | [] => []
  | slot :: tail =>
    writeSlot headReads slot (edits i) priorCR ++ renderSlots tail edits (i + 1) slot.endsInCR

theorem renderPlan_slots (slots : List Slot) (edits : Nat → List Splice) (i : Nat) (priorCR : Bool) :
    (planSlots headReads slots edits i priorCR).flatMap Slot.bytes = renderSlots headReads slots edits i priorCR := by
  induction slots generalizing i priorCR with
  | nil => rfl
  | cons slot tail ih => simp [planSlots, renderSlots, writeSlot, ih]

/-- Page-level composition with untouched preamble and unassigned tail. -/
theorem write_page_ranges (page : Page) (edits : Nat → List Splice) :
    write headReads page edits =
      page.preamble.flatMap Line.bytes ++ renderSlots headReads page.slots edits 0 (linesEndInCR page.preamble) ++
        page.unassigned.flatMap Line.bytes := by
  simp [write, planPage, Page.bytes, renderPlan_slots]

theorem valid_snapshot (limit : Nat) (edits : List Splice)
    (bounds : ∀ edit ∈ edits, edit.start ≤ edit.stop ∧ edit.stop ≤ limit)
    (disjoint : edits.Pairwise separate) : OrderedWithin limit 0 (ordered edits) := by
  have recipe (sorted : List Splice) (cursor : Nat)
      (order : sorted.Pairwise (fun a b => keyLE a b = true))
      (sep : sorted.Pairwise separate)
      (bound : ∀ edit ∈ sorted, edit.start ≤ edit.stop ∧ edit.stop ≤ limit)
      (hc : cursor ≤ limit)
      (lower : ∀ edit ∈ sorted, cursor ≤ edit.start) :
      OrderedWithin limit cursor sorted := by
    induction sorted generalizing cursor with
    | nil => exact hc
    | cons a rest ih =>
      have aself := bound a (by simp)
      refine ⟨lower a (by simp), aself.1, ?_⟩
      apply ih a.stop (List.pairwise_cons.mp order).2 (List.pairwise_cons.mp sep).2
        (fun edit mem => bound edit (by simp [mem])) aself.2
      intro b mem
      have bself := bound b (by simp [mem])
      have keys := (List.pairwise_cons.mp order).1 b mem
      have apart := (List.pairwise_cons.mp sep).1 b mem
      simp only [keyLE, decide_eq_true_eq] at keys
      rcases apart.2 with proper | reverse
      · exact proper
      · exfalso
        have sameStart : a.start = b.start := by omega
        have sameStop : a.stop = b.stop := by omega
        exact apart.1 (Prod.ext sameStart sameStop)
  apply recipe _ _ (ordered_pairwise _)
  · exact (List.mergeSort_perm edits _).symm.pairwise disjoint (by
      intro a b hab
      exact ⟨Ne.symm hab.1, hab.2.symm⟩)
  · intro e he
    exact bounds e (List.mem_mergeSort.mp he)
  · exact Nat.zero_le _
  · intro e _
    exact Nat.zero_le _

theorem fusion (rest : Bytes) :
    lex (cr :: lf :: rest) = Line.mk [] .crlf :: lex rest := by
  simp [lex, prependByte, cr, lf]

theorem fusion_repaired (rest : Bytes) :
    lex (cr :: cr :: lf :: rest) = Line.mk [] .cr :: Line.mk [] .crlf :: lex rest := by
  simp [lex, prependByte, cr, lf]

theorem admitted_heads (file : Bytes) (specs : List SlotSpec) (pos : Nat)
    (h : admitSlots file pos specs) :
    ∀ slot ∈ (readSlots specs (file.drop pos)).1, slot.rows ≠ [] := by
  induction specs generalizing pos with
  | nil => simp [readSlots]
  | cons spec rest ih =>
    rcases h with ⟨ok, remaining⟩
    simp only [readSlots, List.mem_cons]
    intro slot mem
    rcases mem with rfl | mem
    · have trims := ok.2.2.2.1
      have count := ok.2.2.2.2.1
      simp only [readSlot]
      cases t : spec.trims with
      | nil => exact (trims t).elim
      | cons width widths =>
        cases lines : lex ((file.drop pos).take spec.width) with
        | nil => simp [t, lines] at count
        | cons line tail => simp
    · apply ih (pos + spec.width) remaining slot
      rwa [List.drop_drop] at mem

theorem readSlots_lexical (specs : List SlotSpec) (file : Bytes) :
    ∀ slot ∈ (readSlots specs file).1, Lexical slot := by
  induction specs generalizing file with
  | nil => simp [readSlots]
  | cons spec rest ih =>
    intro slot mem
    simp only [readSlots, List.mem_cons] at mem
    rcases mem with rfl | mem
    · exact readSlot_lexical _ _
    · exact ih _ slot mem

theorem planSlots_lexical (old : List Slot) (edits : Nat → List Splice) (i : Nat)
    (source : ∀ slot ∈ old, Lexical slot)
    (changed : ∀ (slot : Slot) (index : Nat), (slot, index) ∈ old.zipIdx i →
      edits index ≠ [] → ∀ priorCR, lexShape (plannedSlot headReads slot (edits index) priorCR).lines)
    (priorCR : Bool := false) :
    ∀ slot ∈ planSlots headReads old edits i priorCR, Lexical slot := by
  induction old generalizing i priorCR with
  | nil => simp [planSlots]
  | cons head tail ih =>
    intro slot mem
    simp only [planSlots, List.mem_cons] at mem
    rcases mem with rfl | mem
    · by_cases h : edits i = []
      · simpa [h] using source head (by simp)
      · exact (lexical_iff _).mpr (changed head i (by simp) h priorCR)
    · exact ih (i + 1) (fun s hs => source s (by simp [hs]))
        (fun s j hj => changed s j (by simp [hj])) head.endsInCR slot mem

def lexicalCheck (slot : Slot) : Bool := decide (lex slot.bytes = slot.lines)

theorem lexicalCheck_iff (slot : Slot) : lexicalCheck slot = true ↔ lexShape slot.lines := by
  simp [lexicalCheck, ← lexical_iff, Lexical]

/-- Only edited slots need a shape premise. The reader supplies lexical validity for untouched slots. -/
theorem readBack_edits (before after : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible before file)
    (moves : after.moves (planPage headReads (read before file) edits))
    (shape : ∀ (slot : Slot) (i : Nat), (slot, i) ∈ (read before file).slots.zipIdx →
      edits i ≠ [] → ∀ priorCR, lexShape (plannedSlot headReads slot (edits i) priorCR).lines) :
    layoutContent (read after (write headReads (read before file) edits)).slots =
      ((read before file).slots.zipIdx).map
        (fun (slot, i) => (slot.level, applyContent slot.content (edits i))) := by
  refine readBack headReads before after file edits (admitted_heads _ _ _ admitted.2) moves ?_
  exact planSlots_lexical headReads _ _ _ (readSlots_lexical _ _) shape _

def SlotInputOK (slot : Slot) (edits : List Splice) : Prop :=
  CRFree edits ∧ (edits = [] ∨ ¬ EmptySliceEnd slot edits)

theorem slotCheck_exact (slot : Slot) (edits : List Splice) (priorCR : Bool)
    (head : slot.rows ≠ []) (lexical : Lexical slot) (indent : cleanBody slot.continuation) :
    lexicalCheck (plannedSlot headReads slot edits priorCR) = true ↔ SlotInputOK slot edits := by
  rw [lexicalCheck_iff]
  constructor
  · intro shape
    have free := repairedShape_CRFree headReads slot edits head priorCR shape
    exact ⟨free, (repairedShape_iff headReads slot edits head lexical indent free priorCR).mp shape⟩
  · rintro ⟨free, input⟩
    exact (repairedShape_iff headReads slot edits head lexical indent free priorCR).mpr input

def crFreeCheck (edits : List Splice) : Bool :=
  edits.all fun e => !e.replacement.contains cr

theorem crFreeCheck_exact (edits : List Splice) : crFreeCheck edits = true ↔ CRFree edits := by
  simp [crFreeCheck, CRFree, List.all_eq_true]

def checkedWriteSlot (slot : Slot) (edits : List Splice) (priorCR : Bool := false) : Option Bytes :=
  let candidate := plannedSlot headReads slot edits priorCR
  if crFreeCheck edits && lexicalCheck candidate then some candidate.bytes else none

theorem checkedWriteSlot_exact (slot : Slot) (edits : List Splice) (priorCR : Bool)
    (head : slot.rows ≠ []) (lexical : Lexical slot) (indent : cleanBody slot.continuation) :
    checkedWriteSlot headReads slot edits priorCR = some (writeSlot headReads slot edits priorCR) ↔
      SlotInputOK slot edits := by
  simp only [checkedWriteSlot, writeSlot, Bool.and_eq_true, ite_eq_left_iff, reduceCtorEq,
    imp_false, Classical.not_not, crFreeCheck_exact, slotCheck_exact headReads _ _ _ head lexical indent]
  exact ⟨And.right, fun h => ⟨h.1, h⟩⟩

theorem admitted_continuations (file : Bytes) (specs : List SlotSpec) (pos : Nat)
    (h : admitSlots file pos specs) :
    ∀ slot ∈ (readSlots specs (file.drop pos)).1, cleanBody slot.continuation := by
  induction specs generalizing pos with
  | nil => simp [readSlots]
  | cons spec rest ih =>
    rcases h with ⟨ok, remaining⟩
    simp only [readSlots, List.mem_cons]
    intro slot mem
    rcases mem with rfl | mem
    · exact ok.2.2.2.2.2.2.2
    · apply ih (pos + spec.width) remaining slot
      rwa [List.drop_drop] at mem

def PageInputOK (page : Page) (edits : Nat → List Splice) : Prop :=
  ∀ slot i, (slot, i) ∈ page.slots.zipIdx → SlotInputOK slot (edits i)

theorem planSlots_check_exact (slots : List Slot) (edits : Nat → List Splice) (i : Nat)
    (priorCR : Bool)
    (source : ∀ slot ∈ slots, slot.rows ≠ [] ∧ Lexical slot ∧ cleanBody slot.continuation) :
    (planSlots headReads slots edits i priorCR).all lexicalCheck = true ↔
      ∀ slot j, (slot, j) ∈ slots.zipIdx i → SlotInputOK slot (edits j) := by
  induction slots generalizing i priorCR with
  | nil => simp [planSlots]
  | cons slot tail ih =>
    obtain ⟨head, lexical, indent⟩ := source slot (by simp)
    have rest := ih (i + 1) slot.endsInCR (fun s h => source s (by simp [h]))
    simp only [planSlots, List.all_cons, Bool.and_eq_true, slotCheck_exact headReads _ _ _ head lexical indent,
      rest, List.zipIdx_cons, List.mem_cons]
    constructor
    · rintro ⟨current, other⟩ s j (eq | later)
      · cases eq
        exact current
      · exact other s j later
    · intro h
      exact ⟨h slot i (Or.inl rfl), fun s j mem => h s j (Or.inr mem)⟩

theorem pageCheck_exact (outline : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible outline file) :
    (planPage headReads (read outline file) edits).slots.all lexicalCheck = true ↔
      PageInputOK (read outline file) edits := by
  apply planSlots_check_exact
  intro slot mem
  exact ⟨admitted_heads _ _ _ admitted.2 slot mem, readSlots_lexical _ _ slot mem,
    admitted_continuations _ _ _ admitted.2 slot mem⟩

/-- All-or-nothing file-layer gate. Placements and in-bounds disjoint splices are supplied by
    the callers to this layer. Only edits addressed to existing slots participate. -/
def checkedWrite (page : Page) (edits : Nat → List Splice) : Option Bytes :=
  let plan := planPage headReads page edits
  if (page.slots.zipIdx.all fun (_, i) => crFreeCheck (edits i)) && plan.slots.all lexicalCheck then
    some plan.bytes
  else none

theorem checkedWrite_exact (outline : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible outline file) :
    checkedWrite headReads (read outline file) edits = some (write headReads (read outline file) edits) ↔
      PageInputOK (read outline file) edits := by
  simp only [checkedWrite, write, Bool.and_eq_true, ite_eq_left_iff, reduceCtorEq, imp_false, Classical.not_not,
    pageCheck_exact headReads outline file edits admitted, List.all_eq_true, crFreeCheck_exact]
  exact ⟨And.right, fun h => ⟨fun (slot, i) mem => (h slot i mem).1, h⟩⟩

theorem checkedWrite_commute (page : Page) {edits other : Nat → List Splice}
    (perm : ∀ i, (edits i).Perm (other i)) (disjoint : ∀ i, (edits i).Pairwise separate) :
    checkedWrite headReads page edits = checkedWrite headReads page other := by
  have plans (slots : List Slot) (i : Nat) (p : Bool) :
      planSlots headReads slots edits i p = planSlots headReads slots other i p := by
    induction slots generalizing i p with
    | nil => rfl
    | cons slot tail ih =>
      simp [planSlots, plannedSlot_commute headReads slot (perm i) (disjoint i) p, ih]
  have check (i : Nat) : crFreeCheck (edits i) = crFreeCheck (other i) := (perm i).all_eq
  simp only [checkedWrite, planPage, plans, check]

theorem checkedWrite_refuses (outline : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible outline file) :
    checkedWrite headReads (read outline file) edits = none ↔ ¬PageInputOK (read outline file) edits := by
  rw [← checkedWrite_exact headReads outline file edits admitted]
  simp [checkedWrite, write]

theorem readBack_checked (before after : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible before file)
    (moves : after.moves (planPage headReads (read before file) edits))
    (input : PageInputOK (read before file) edits) :
    layoutContent (read after (write headReads (read before file) edits)).slots =
      ((read before file).slots.zipIdx).map
        (fun (slot, i) => (slot.level, applyContent slot.content (edits i))) := by
  have checked := (pageCheck_exact headReads before file edits admitted).mpr input
  refine readBack headReads before after file edits (admitted_heads _ _ _ admitted.2) moves ?_
  intro slot mem
  exact (lexical_iff _).mpr ((lexicalCheck_iff _).mp (List.all_eq_true.mp checked slot mem))

/- Physical cuts between outline slices -/

theorem linesEndInCR_exact (lines : List Line) (shape : lexShape lines) :
    linesEndInCR lines = decide ((lines.flatMap Line.bytes).getLast? = some cr) := by
  induction lines with
  | nil => rfl
  | cons line tail ih =>
    cases tail with
    | nil =>
      cases e : line.ending with
      | lf | crlf | cr =>
        simp [linesEndInCR, Line.bytes, e, Ending.bytes, cr, lf]
      | eof =>
        have noCR : line.body.getLast? ≠ some cr :=
          fun h => (shape.1 cr (List.mem_of_getLast? h)).2 rfl
        simp [linesEndInCR, Line.bytes, e, Ending.bytes, noCR]
    | cons next tail =>
      have nonempty : ((next :: tail).flatMap Line.bytes) ≠ [] := by
        intro empty
        have h := lex_render _ shape.2.2
        rw [empty] at h
        contradiction
      have present := List.getLast?_eq_some_getLast nonempty
      rw [List.flatMap_cons, List.getLast?_append, present]
      have tailEnds := ih shape.2.2
      rw [present] at tailEnds
      exact tailEnds

theorem lexical_headOK_bytes (slot : Slot) (p : Bool) (lexical : Lexical slot) :
    headOK p slot.lines ↔ (p = true → slot.bytes.head? ≠ some lf) := by
  have shape := (lexical_iff slot).mp lexical
  rw [← lines_bytes]
  generalize slot.lines = lines at shape ⊢
  cases lines with
  | nil => simp [headOK]
  | cons line rest =>
    rcases line with ⟨body, ending⟩
    rw [headOK_cons]
    cases body with
    | nil => cases ending <;> simp_all [lexShape, endingOK, Line.bytes, Ending.bytes, cr, lf]
    | cons b bs =>
      have h := shape.1 b (by simp)
      simp [Line.bytes, h.1]

theorem endsInCR_preserved (slot : Slot) (edits : List Splice) (p : Bool) :
    (plannedSlot headReads slot edits p).endsInCR = slot.endsInCR := by
  unfold plannedSlot
  split
  · rfl
  · let stream := editedCells slot edits
    let layout := front slot.rows ++ separator headReads slot (ordered edits).head? stream
    let adjacent := p && layout.isEmpty
    let e := repairEnding (exitCR adjacent stream) (finalEnding slot.rows)
    change linesEndInCR ((assembleRows layout e (repairCells adjacent stream)).map Piece.line ++
      slot.trailer) = linesEndInCR (slot.rows.map Piece.line ++ slot.trailer)
    cases trailer : slot.trailer with
    | cons t ts =>
      have last := List.getLast?_eq_some_getLast (show t :: ts ≠ [] by simp)
      simp [linesEndInCR, last]
    | nil =>
      have ending := assemble_ending layout e (repairCells adjacent stream)
      have unchanged := (repairEnding_facts (exitCR adjacent stream) (finalEnding slot.rows)).1
      have facts (rows : List Piece) :
          linesEndInCR (rows.map Piece.line) = (finalEnding rows == .cr) := by
        unfold linesEndInCR finalEnding
        rw [List.getLast?_map]
        cases rows.getLast? <;> rfl
      rw [List.append_nil, List.append_nil, facts, facts, ending]
      exact unchanged

theorem sliceHead_at (file : Bytes) (pos width : Nat) (positive : 0 < width) :
    ((file.drop pos).take width).head? = file[pos]? := by
  rw [List.head?_eq_getElem?, List.getElem?_take_of_lt positive, List.getElem?_drop, Nat.add_zero]

theorem sliceLast_at (file : Bytes) (pos width : Nat)
    (positive : 0 < width) (bound : pos + width ≤ file.length) :
    ((file.drop pos).take width).getLast? = file[pos + width - 1]? := by
  have length : ((file.drop pos).take width).length = width := by
    simp only [List.length_take, List.length_drop]
    omega
  rw [List.getLast?_eq_getElem?, length, List.getElem?_take_of_lt (by omega), List.getElem?_drop]
  congr 1
  omega

/-- Source-checked contexts for R3 and for the identity of unedited slice starts. -/
def joinsSafe (priorCR : Bool) : List Slot → Prop
  | [] => True
  | slot :: rest => headOK priorCR slot.lines ∧ joinsSafe slot.endsInCR rest

theorem admitted_joins (file : Bytes) (specs : List SlotSpec) (pos : Nat)
    (h : admitSlots file pos specs) (p : Bool)
    (incoming : p = true → 0 < pos ∧ file[pos - 1]? = some cr) :
    joinsSafe p (readSlots specs (file.drop pos)).1 := by
  induction specs generalizing pos p with
  | nil => trivial
  | cons spec rest ih =>
    obtain ⟨ok, tail⟩ := h
    let bytes := (file.drop pos).take spec.width
    let slot := readSlot spec bytes
    have lexical := readSlot_lexical spec bytes
    have shape := (lexical_iff slot).mp lexical
    change headOK p slot.lines ∧ joinsSafe slot.endsInCR
      (readSlots rest ((file.drop pos).drop spec.width)).1
    obtain ⟨cut, bound, positive, _⟩ := ok
    refine ⟨(lexical_headOK_bytes slot p lexical).mpr ?_, ?_⟩
    · intro active
      obtain ⟨hp, previous⟩ := incoming active
      rw [readSlot_lossless, sliceHead_at _ _ _ positive]
      rcases cut with zero | noCR | noLF
      · omega
      · exact (noCR previous).elim
      · exact noLF
    · rw [List.drop_drop]
      apply ih (pos + spec.width) tail slot.endsInCR
      intro active
      have last := linesEndInCR_exact slot.lines shape
      rw [lines_bytes, readSlot_lossless, sliceLast_at _ _ _ positive bound] at last
      change linesEndInCR slot.lines = true at active
      rw [active] at last
      exact ⟨by omega, of_decide_eq_true last.symm⟩

theorem read_joins (outline : Outline) (file : Bytes) (admitted : Admissible outline file) :
    joinsSafe (linesEndInCR (read outline file).preamble) (read outline file).slots := by
  obtain ⟨bound, slots⟩ := admitted
  apply admitted_joins file outline.slots outline.preamble slots
  intro active
  have positive : 0 < outline.preamble := by
    by_cases zero : outline.preamble = 0
    · simp [read, zero, lex, linesEndInCR] at active
    · omega
  have prev := linesEndInCR_exact (lex (file.take outline.preamble)) (lex_shape _)
  change linesEndInCR (lex (file.take outline.preamble)) = true at active
  have atCut : (file.take outline.preamble).getLast? = file[outline.preamble - 1]? := by
    simpa using sliceLast_at file 0 outline.preamble positive (by simpa using bound)
  rw [active, lex_lossless, atCut] at prev
  exact ⟨positive, of_decide_eq_true prev.symm⟩

theorem repairedSlot_headOK (slot : Slot) (edits : List Splice) (head : slot.rows ≠ [])
    (lexical : Lexical slot) (indent : cleanBody slot.continuation) (free : CRFree edits)
    (p : Bool) (sourceGuard : headOK p slot.lines) :
    headOK p (plannedSlot headReads slot edits p).lines := by
  by_cases edited : edits = []
  · simpa [edited] using sourceGuard
  · exact (editedSlot_shape headReads slot edits head lexical indent free p edited).1

theorem planSlots_joins (slots : List Slot) (edits : Nat → List Splice) (i : Nat) (p : Bool)
    (source : ∀ slot ∈ slots, slot.rows ≠ [] ∧ Lexical slot ∧ cleanBody slot.continuation)
    (joins : joinsSafe p slots)
    (free : ∀ slot j, (slot, j) ∈ slots.zipIdx i → CRFree (edits j)) :
    joinsSafe p (planSlots headReads slots edits i p) := by
  induction slots generalizing i p with
  | nil => trivial
  | cons slot tail ih =>
    obtain ⟨head, lexical, indent⟩ := source slot (by simp)
    obtain ⟨guard, rest⟩ := joins
    change headOK p (plannedSlot headReads slot (edits i) p).lines ∧
      joinsSafe (plannedSlot headReads slot (edits i) p).endsInCR
        (planSlots headReads tail edits (i + 1) slot.endsInCR)
    rw [endsInCR_preserved]
    exact ⟨repairedSlot_headOK headReads slot _ head lexical indent (free slot i (by simp)) p guard,
      ih _ _ (fun s hs => source s (by simp [hs])) rest (fun s j hs => free s j (by simp [hs]))⟩

theorem repairedPage_joins (outline : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible outline file) (input : PageInputOK (read outline file) edits) :
    joinsSafe (linesEndInCR (read outline file).preamble) (planPage headReads (read outline file) edits).slots := by
  have source (slot : Slot) (mem : slot ∈ (read outline file).slots) :=
    And.intro (admitted_heads _ _ _ admitted.2 slot mem)
      (And.intro (readSlots_lexical _ _ slot mem) (admitted_continuations _ _ _ admitted.2 slot mem))
  exact planSlots_joins headReads _ _ _ _ source (read_joins _ _ admitted)
    (fun slot i mem => (input slot i mem).1)

/-- The raw bytes on either side of every outline cut cannot form a split CRLF. -/
def byteJoinsSafe (previous : Option UInt8) : List Slot → Prop
  | [] => True
  | slot :: tail =>
    (previous = some cr → slot.bytes.head? ≠ some lf) ∧
      byteJoinsSafe slot.bytes.getLast? tail

theorem joinsSafe_bytes (slots : List Slot) (previous : Option UInt8)
    (lexical : ∀ slot ∈ slots, Lexical slot)
    (safe : joinsSafe (decide (previous = some cr)) slots) :
    byteJoinsSafe previous slots := by
  induction slots generalizing previous with
  | nil => trivial
  | cons slot tail ih =>
    obtain ⟨head, rest⟩ := safe
    have slotLex := lexical slot (by simp)
    have start := (lexical_headOK_bytes slot _ slotLex).mp head
    have ending := linesEndInCR_exact slot.lines ((lexical_iff _).mp slotLex)
    rw [lines_bytes] at ending
    refine ⟨fun crLast => start (decide_eq_true crLast), ?_⟩
    exact ih slot.bytes.getLast? (fun s hs => lexical s (by simp [hs]))
      (by simpa only [Slot.endsInCR, ending] using rest)

theorem repairedPage_byteJoins (outline : Outline) (file : Bytes) (edits : Nat → List Splice)
    (admitted : Admissible outline file) (input : PageInputOK (read outline file) edits) :
    byteJoinsSafe ((read outline file).preamble.flatMap Line.bytes).getLast?
      (planPage headReads (read outline file) edits).slots := by
  have checked := (pageCheck_exact headReads outline file edits admitted).mpr input
  have safe := repairedPage_joins headReads outline file edits admitted input
  have preamble := linesEndInCR_exact (read outline file).preamble (lex_shape _)
  apply joinsSafe_bytes
  · intro slot mem
    exact (lexical_iff _).mpr ((lexicalCheck_iff _).mp (List.all_eq_true.mp checked slot mem))
  · rwa [preamble] at safe

theorem readSlots_segments (specs : List SlotSpec) (slots : List Slot) (tail : Bytes)
    (widths : specs.map SlotSpec.width = slots.map (fun slot => slot.bytes.length)) :
    (readSlots specs (slots.flatMap Slot.bytes ++ tail)).1 =
      List.zipWith (fun spec slot => readSlot spec slot.bytes) specs slots := by
  induction specs generalizing slots with
  | nil => rfl
  | cons spec specs ih =>
    cases slots with
    | nil => simp at widths
    | cons slot slots =>
      simp only [List.map_cons, List.cons.injEq] at widths
      simp only [readSlots, List.flatMap_cons, List.append_assoc, widths.1, List.take_append_length,
        List.drop_append_length, List.zipWith_cons_cons, ih slots widths.2]

/-- Slot boundaries isolate re-reads even when a different slot fails its lexical check. -/
theorem readBack_segments (after : Outline) (page : Page) (edits : Nat → List Splice)
    (preamble : after.preamble = (page.preamble.flatMap Line.bytes).length)
    (widths : after.slots.map SlotSpec.width =
      (planPage headReads page edits).slots.map (fun slot => slot.bytes.length)) :
    (read after (write headReads page edits)).slots =
      List.zipWith (fun spec slot => readSlot spec slot.bytes) after.slots (planPage headReads page edits).slots := by
  let plan := planPage headReads page edits
  change (read after plan.bytes).slots = _
  change after.preamble = (plan.preamble.flatMap Line.bytes).length at preamble
  rw [read, Page.bytes, preamble]
  simpa only [List.append_assoc, List.drop_append_length] using
    readSlots_segments after.slots plan.slots (plan.unassigned.flatMap Line.bytes) widths

theorem readBack_untouched (previous next : SlotSpec) (bytes : Bytes)
    (placement : next.placement = (readSlot previous bytes).placement) :
    (readSlot next bytes).content = (readSlot previous bytes).content := by
  have pieces := (readSlot_placement next (readSlot previous bytes) placement
    (readSlot_lexical previous bytes)).1
  rw [readSlot_lossless] at pieces
  simp only [Slot.content, pieces]

section Witnesses

private def headingBoundary : Bytes → Bool
  | [] => true
  | b :: _ => b == space || b == 9 || b == 12

-- Local marker and structural-trim rules of lsdoc v0.5.13, on bare and
-- once-padded Markdown dash and Org star layouts. Page placement is separate.
private def parserHead (layout text : Bytes) : Bool :=
  layout.getLast? == some space ||
    (text.head? != some space &&
      (headingBoundary text ||
        (layout.getLast? == some 45 && text.head? == some 35 &&
          headingBoundary (text.dropWhile (· == 35)))))

private def bareHeader (bytes : Bytes) : Outline := ⟨0, [⟨bytes.length, 1, [1], true, []⟩]⟩
private def checkedBareHeader (bytes : Bytes) (edits : List Splice) : Option Bytes :=
  checkedWrite parserHead (read (bareHeader bytes) bytes) (fun _ => edits)
private def insertX : List Splice := [⟨0, 0, [120]⟩]

-- Markdown "-\tTitle", Org "*\fTitle", and Markdown "-# Heading".
private def mdTab : Bytes := [45, 9, 84, 105, 116, 108, 101]
private def orgFormFeed : Bytes := [42, 12, 84, 105, 116, 108, 101]
private def mdHeading : Bytes := [45, 35, 32, 72, 101, 97, 100, 105, 110, 103]

example : oldWriteSlot (readSlot ⟨7, 1, [1], true, []⟩ mdTab) insertX =
      [45, 120, 9, 84, 105, 116, 108, 101] ∧
    oldWriteSlot (readSlot ⟨7, 1, [1], true, []⟩ orgFormFeed) insertX =
      [42, 120, 12, 84, 105, 116, 108, 101] ∧
    oldWriteSlot (readSlot ⟨10, 1, [1], true, []⟩ mdHeading) insertX =
      [45, 120, 35, 32, 72, 101, 97, 100, 105, 110, 103] := by decide +kernel

example : Admissible (bareHeader mdTab) mdTab ∧
    Admissible (bareHeader orgFormFeed) orgFormFeed ∧
    Admissible (bareHeader mdHeading) mdHeading := by
  simp [Admissible, bareHeader, admitSlots, slotAdmissible, safeCut, atLineStart, cleanBody,
    mdTab, orgFormFeed, mdHeading, lex, prependByte, cr, lf]

example : checkedBareHeader mdTab insertX = some [45, 32, 120, 9, 84, 105, 116, 108, 101] :=
  by decide +kernel

example : checkedBareHeader orgFormFeed insertX = some [42, 32, 120, 12, 84, 105, 116, 108, 101] :=
  by decide +kernel

example : checkedBareHeader mdHeading insertX = some [45, 32, 120, 35, 32, 72, 101, 97, 100, 105, 110, 103] :=
  by decide +kernel

-- One ASCII space would be consumed as layout, while tab and form feed remain text.
-- Doubling the hash run here still leaves a Markdown bullet with exact text.
example : checkedBareHeader [45, 35, 10] [⟨0, 1, [32]⟩] = some [45, 32, 32, 10] ∧
    checkedBareHeader [45, 35, 10] [⟨0, 1, [9]⟩] = some [45, 9, 10] ∧
    checkedBareHeader [42, 35, 10] [⟨0, 1, [12]⟩] = some [42, 12, 10] ∧
    checkedBareHeader [45, 35, 10] [⟨0, 0, [35]⟩] = some [45, 35, 35, 10] := by decide +kernel

-- Deleting the first byte can expose a different retained first byte.
example : checkedBareHeader [45, 35, 88, 10] [⟨0, 1, []⟩] = some [45, 32, 88, 10] :=
  by decide +kernel

-- "-# H\n** bt ffvac\n\t\t\t> lf d\r\n", replacing the first four
-- content bytes (including LF) with "#". Two continuation tabs are layout.
private def mdRunBody : Bytes := [42, 42, 32, 98, 116, 32, 102, 102, 118, 97, 99]
private def mdRunLastText : Bytes := [9, 62, 32, 108, 102, 32, 100]
private def mdRunTail : Bytes := [10, 9, 9] ++ mdRunLastText ++ [13, 10]
private def mdRunFile : Bytes := [45, 35, 32, 72, 10] ++ mdRunBody ++ mdRunTail
private def mdRunWritten : Bytes := [45, 32, 35] ++ mdRunBody ++ mdRunTail
private def mdRunOutline : Outline := ⟨0, [⟨28, 1, [1, 0, 2], true, [32, 32]⟩]⟩
private def mdRunNext : Outline := ⟨0, [⟨26, 1, [2, 2], false, [32, 32]⟩]⟩

example : checkedWrite parserHead (read mdRunOutline mdRunFile) (fun _ => [⟨0, 4, [35]⟩]) =
      some mdRunWritten ∧
    layoutContent (read mdRunNext mdRunWritten).slots =
      [(1, [35] ++ mdRunBody ++ [10] ++ mdRunLastText)] ∧
    parserHead [45] ([35] ++ mdRunBody) = false ∧
    parserHead [45, 32] ([35] ++ mdRunBody) = true := by decide +kernel

-- "*\n", inserting " é " (UTF-8). The first space is layout and the second is text.
private def orgLeadingText : Bytes := [32, 195, 169, 32]
private def orgLeadingWritten : Bytes := [42, 32] ++ orgLeadingText ++ [10]

example : checkedBareHeader [42, 10] [⟨0, 0, orgLeadingText⟩] = some orgLeadingWritten ∧
    layoutContent (read ⟨0, [⟨7, 1, [2], false, []⟩]⟩ orgLeadingWritten).slots =
      [(1, orgLeadingText)] ∧
    parserHead [42] orgLeadingText = false ∧
    parserHead [42, 32] orgLeadingText = true := by decide +kernel

-- A first-row edit at offset 1 cannot put a space at the earlier gap 0.
example : checkedBareHeader [45, 35, 32, 72, 10] [⟨1, 2, [120]⟩] =
      some [45, 35, 120, 72, 10] ∧
    parserHead [45] [35, 120, 72] = false ∧
    parserHead [45, 32] [35, 120, 72] = true := by decide +kernel

-- Identical first-row text preserves layout even with an arbitrary predicate.
example : checkedWrite (fun _ _ => false) (read (bareHeader mdTab) mdTab)
      (fun _ => [⟨0, 1, [9]⟩]) = some mdTab := by decide +kernel

private def orgTrailer : SlotSpec := ⟨5, 1, [2], false, []⟩

-- "* T\\n\\n" has one content row and one physical trailer row.
example : (readSlot orgTrailer [42, 32, 84, 10, 10]).content = [84] ∧
    (readSlot orgTrailer [42, 32, 84, 10, 10]).trailer = [⟨[], .lf⟩] ∧
    writeSlot parserHead (readSlot orgTrailer [42, 32, 84, 10, 10]) [⟨0, 1, [85]⟩]
      = [42, 32, 85, 10, 10] := by decide +kernel

-- An Org page "* A\rline\n* Next\n": the first slot's content "A\nline" (its
-- newline is the bare CR) has 1..6 replaced by one newline; the next slot is
-- independent of the edit.
private def orgCR : Outline := ⟨0, [⟨9, 1, [2, 0], false, []⟩, ⟨7, 1, [2], false, []⟩]⟩
private def orgCRFile : Bytes := [42, 32, 65, 13, 108, 105, 110, 101, 10, 42, 32, 78, 101, 120, 116, 10]
private def orgCREdit (i : Nat) : List Splice := if i = 0 then [⟨1, 6, [10]⟩] else []

example : write parserHead (read orgCR orgCRFile) orgCREdit =
    [42, 32, 65, 10, 10, 42, 32, 78, 101, 120, 116, 10] := by decide +kernel

example : layoutContent (read ⟨0, [⟨5, 1, [2, 0], false, []⟩, ⟨7, 1, [2], false, []⟩]⟩
    (write parserHead (read orgCR orgCRFile) orgCREdit)).slots =
      [(1, [65, 10]), (1, [78, 101, 120, 116])] := by decide +kernel

-- "-\\n- N\\n" with "x" inserted into its bare first header.
private def bareOutline : Outline := ⟨0, [⟨2, 1, [1], true, []⟩, ⟨4, 1, [2], false, []⟩]⟩
private def bareEdits (i : Nat) : List Splice := if i = 0 then [⟨0, 0, [120]⟩] else []

example : write parserHead (read bareOutline [45, 10, 45, 32, 78, 10]) bareEdits =
    [45, 32, 120, 10, 45, 32, 78, 10] := by decide +kernel

example : layoutContent (read ⟨0, [⟨4, 1, [2], false, []⟩, ⟨4, 1, [2], false, []⟩]⟩
    (write parserHead (read bareOutline [45, 10, 45, 32, 78, 10]) bareEdits)).slots =
      [(1, [120]), (1, [78])] := by decide +kernel

-- Three real break kinds and a nonempty EOF row in one file.
private def mixed : Bytes := [45, 32, 65, 10, 32, 66, 13, 32, 67, 13, 10, 32, 68]
private def mixedSpec : SlotSpec := ⟨13, 1, [2, 1, 1, 1], false, [32]⟩

example : (lex mixed).map Line.ending = [.lf, .cr, .crlf, .eof] ∧
    (readSlot mixedSpec mixed).content = [65, 10, 66, 10, 67, 10, 68] ∧
    (readSlot mixedSpec mixed).gap 3 = 6 ∧ (readSlot mixedSpec mixed).gap 4 = 8 ∧
    (readSlot mixedSpec mixed).gap 5 = 9 ∧ (readSlot mixedSpec mixed).gap 6 = 12 := by decide +kernel

-- The external-outline abstraction also allows an empty first physical body.
example : oldWriteSlot (readSlot ⟨2, 1, [0, 0], false, []⟩ [13, 88]) [⟨1, 1, [10]⟩] =
    [13, 10, 88] := by decide +kernel

-- A newline inserted at content gap 1 of the bare header "-\rX" (content "\nX",
-- whose newline is the bare CR): the retained CR and the newly written LF would
-- fuse into one CRLF; the old write below shows it.
private def fuseSlot : Slot := readSlot ⟨3, 1, [1, 0], true, []⟩ [45, 13, 88]
private def fuseEdit : List Splice := [⟨1, 1, [10]⟩]

example : oldWriteSlot fuseSlot fuseEdit = [45, 13, 10, 88] ∧
    (readSlot ⟨4, 1, [1, 0], true, []⟩ (oldWriteSlot fuseSlot fuseEdit)).content = [10, 88] ∧
    applyContent fuseSlot.content fuseEdit = [10, 10, 88] := by decide +kernel

example : lex [45, 13, 13, 10, 88] = [⟨[45], .cr⟩, ⟨[], .crlf⟩, ⟨[88], .eof⟩] ∧
    (readSlot ⟨5, 1, [1, 0, 0], true, []⟩ [45, 13, 13, 10, 88]).content =
      applyContent fuseSlot.content fuseEdit := by decide +kernel

-- Deletion can expose a kept CR to a kept LF, with no emitted LF to repair.
private def eraseSlot : Slot := readSlot ⟨4, 1, [1, 0], true, []⟩ [45, 13, 88, 10]
private def eraseEdit : List Splice := [⟨1, 2, []⟩]

example : oldWriteSlot eraseSlot eraseEdit = [45, 13, 10] ∧
    (readSlot ⟨3, 1, [1], true, []⟩ (oldWriteSlot eraseSlot eraseEdit)).content = [] ∧
    applyContent eraseSlot.content eraseEdit = [10] := by decide +kernel

-- With no continuation indent, a trailing LF cannot by itself materialize an empty EOF row.
private def emptyHeader : Slot := readSlot ⟨1, 1, [1], true, []⟩ [45]
private def trailingLF : List Splice := [⟨0, 0, [10]⟩]

example : oldWriteSlot emptyHeader trailingLF = [45, 10] ∧
    (readSlot ⟨2, 1, [1], true, []⟩ (oldWriteSlot emptyHeader trailingLF)).content = [] ∧
    applyContent emptyHeader.content trailingLF = [10] := by decide +kernel

-- The old emitted-LF collision is repaired by a one-byte extension to the insertion.
example : checkedWriteSlot parserHead fuseSlot fuseEdit = some [45, 13, 13, 10, 88] ∧
    (readSlot ⟨5, 1, [1, 0, 0], true, []⟩ (writeSlot parserHead fuseSlot fuseEdit)).content =
      applyContent fuseSlot.content fuseEdit := by decide +kernel

-- Deleting X retains both original breaks and puts CR at the deletion's right endpoint.
example : checkedWriteSlot parserHead eraseSlot eraseEdit = some [45, 13, 13, 10] ∧
    (readSlot ⟨4, 1, [1, 0], true, []⟩ (writeSlot parserHead eraseSlot eraseEdit)).content =
      applyContent eraseSlot.content eraseEdit := by decide +kernel

private theorem order_two (a b : Splice) :
    ordered [a, b] = if keyLE a b then [a, b] else [b, a] := by
  simp [ordered, List.mergeSort, List.merge]

-- Two nonempty touching ranges jointly expose a retained LF. The right range owns the extra CR.
private def touchSlot : Slot := readSlot ⟨5, 1, [1, 0], false, []⟩ [45, 13, 88, 89, 10]
private def touchEdits : List Splice := [⟨1, 2, []⟩, ⟨2, 3, []⟩]

example : ((bytePatches false (framedChunks touchSlot touchEdits)).map Chunk.put) = [[], [13], []] ∧
    checkedWriteSlot parserHead touchSlot touchEdits = some [45, 13, 13, 10] ∧
    checkedWriteSlot parserHead touchSlot touchEdits.reverse = some [45, 13, 13, 10] := by
  simp only [checkedWriteSlot, plannedSlot, framedChunks, touchEdits,
    List.reverse_cons, List.reverse_nil, List.nil_append, List.singleton_append, order_two]
  decide +kernel

-- Insertions on both sides of the same retained CR are likewise independent of supplied order.
private def aroundCR : Slot := readSlot ⟨4, 1, [1, 0], false, []⟩ [45, 65, 13, 88]
private def aroundEdits : List Splice := [⟨1, 1, [10]⟩, ⟨2, 2, [10]⟩]

example : checkedWriteSlot parserHead aroundCR aroundEdits = some [45, 65, 10, 13, 13, 10, 88] ∧
    checkedWriteSlot parserHead aroundCR aroundEdits.reverse = some [45, 65, 10, 13, 13, 10, 88] := by
  simp only [checkedWriteSlot, plannedSlot, aroundEdits,
    List.reverse_cons, List.reverse_nil, List.nil_append, List.singleton_append, order_two]
  decide +kernel

example : checkedWriteSlot parserHead emptyHeader trailingLF = none ∧
    checkedWriteSlot parserHead emptyHeader [⟨0, 0, [13]⟩] = none ∧
    checkedWriteSlot parserHead { emptyHeader with continuation := [32] } trailingLF =
      some [45, 10, 32] := by decide +kernel

-- With no layout on the first and only row, a deletion can empty it without inserting LF.
example : checkedWriteSlot parserHead (readSlot ⟨1, 1, [0], false, []⟩ [88]) [⟨0, 1, []⟩] =
    none := by decide +kernel

-- A nonfinal slice can lose its last row at a mid-line outline cut.
private def midSlice : Outline := ⟨0, [⟨1, 1, [0], false, []⟩, ⟨2, 1, [0], false, []⟩]⟩
private def eraseFirst (i : Nat) : List Splice := if i = 0 then [⟨0, 1, []⟩] else []

example : Admissible midSlice [88, 89, 10] := by
  simp [Admissible, midSlice, admitSlots, slotAdmissible, safeCut, atLineStart, cleanBody,
    lex, prependByte, cr, lf]

example : checkedWrite parserHead (read midSlice [88, 89, 10]) eraseFirst = none := by decide +kernel

-- The same collision can straddle an outline cut. Only the right slice acquires a byte.
private def seamOutline : Outline := ⟨1, [⟨2, 1, [0], false, []⟩]⟩

example : Admissible seamOutline [13, 88, 10] := by
  simp [Admissible, seamOutline, admitSlots, slotAdmissible, safeCut, atLineStart, cleanBody,
    lex, prependByte, cr, lf]

example : oldWriteSlot (readSlot ⟨2, 1, [0], false, []⟩ [88, 10]) (eraseFirst 0) = [10] ∧
    checkedWrite parserHead (read seamOutline [13, 88, 10]) eraseFirst = some [13, 13, 10] ∧
    lex (write parserHead (read seamOutline [13, 88, 10]) eraseFirst) = [⟨[], .cr⟩, ⟨[], .crlf⟩] :=
  by decide +kernel

-- Adjacent ranges use one snapshot, including an insertion at a replacement boundary.
#guard writeSlot parserHead (readSlot ⟨5, 1, [2], false, []⟩ [45, 32, 65, 66, 67])
    [⟨1, 2, [89]⟩, ⟨0, 1, [88]⟩, ⟨1, 1, [33]⟩] == [45, 32, 88, 33, 89, 67]

-- Empty file and no-op writes also traverse the real lexer.
example : write parserHead (read ⟨0, []⟩ []) (fun _ => []) = [] := by decide +kernel
example : writeSlot parserHead emptyHeader [] = [45] := by decide +kernel

-- A cut into a line body preserves bytes and gives the suffix header no removed prefix.
example : (read ⟨2, [⟨3, 1, [0], false, []⟩]⟩ [97, 98, 99, 100, 10]).bytes =
    [97, 98, 99, 100, 10] := by decide +kernel

end Witnesses

#print axioms R0
#print axioms R1
#print axioms gap_total
#print axioms gap_monotone
#print axioms gap_strict
#print axioms text_address_inverse
#print axioms content_newline
#print axioms R3
#print axioms R3_patchStart
#print axioms R3_separatorPatch
#print axioms bytePatches_terminal
#print axioms separator_firstPatch
#print axioms separator_origin
#print axioms changedHead_separatorCheck
#print axioms lastBodyEmpty_separator
#print axioms write_page_ranges
#print axioms readBack_edits
#print axioms readBack_segments
#print axioms readBack_untouched
#print axioms lexicalCheck_iff
#print axioms valid_snapshot
#print axioms readSlot_exact_iff
#print axioms commute
#print axioms slotCheck_exact
#print axioms pageCheck_exact
#print axioms checkedWrite_exact
#print axioms checkedWrite_refuses
#print axioms checkedWrite_commute
#print axioms repairedPage_byteJoins
#print axioms readBack_checked
#print axioms fusion_repaired

end BlockText
