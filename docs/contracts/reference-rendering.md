# Reference rendering lifetime (development)

Linked and Unlinked References keep ordered membership, group headers and group
disclosure intent. A reference occurrence owns its stable surface identity,
nested fold choices, breadcrumb context, full-excerpt choices and measured
geometry independently of its disposable rendered child. Removing membership
disposes that state. Queries, embeds and previews keep their existing lifetime.

Reference content enters near the viewport and retires when distant, leaving
its last measured height. Width changes may refine the measurement on re-entry;
a visited group does not revert to an estimate. Initial unvisited space is a
literal-line estimate. Outline and reference windows share scroller observers,
geometry transactions and measured-height observation. Mobile overscan is
relative to the scroller height and follows viewport resizing.

The current editor (including IME and draft), a pending header edit, native text
selection, model block selection and drag interactions pin their content and
source page, so working-set eviction cannot replace selected live DOM with DTOs. Pins
may exceed the ordinary viewport bound. Find does not count reference panels
(Linked and Unlinked References, a block's reference panel; GH #935): it neither
holds nor restores their groups, and loads none to build a full-reference search
index. The find bar names the panels it leaves out and hands its query to the
panel's own search, which covers every group (the panel root carries
`data-inpage-find-references`). Large property headers use measured
chunks, retaining their geometry with the occurrence. Large root lists use the
ordinary outline windows and model-based reveal/navigation scope.

Visible admissions precede overscan work. At most two source reads wait
concurrently. Shell creation and document admission yield between groups at
frame/task boundaries. Queued obsolete work is discarded; in-flight replies
must still own their graph and occurrence immediately before document admission.
Backend reads are not forcibly interrupted. Browsing adds no authored writes;
editing continues through the existing guarded save path.

Performance acceptance for GH #886 is measurement-gated. The receipt records
the fixed budgets and native evidence; component tests alone do not certify
responsiveness, memory plateaus, or behavior on other native engines.
