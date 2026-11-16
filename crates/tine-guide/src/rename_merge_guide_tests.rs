#[test]
fn rename_merge_and_journal_rename_proposals_are_documented_in_the_bundled_guide() {
    let tips = include_str!("templates/tips.md");
    for control in [
        "offers to **merge** them",
        "the aliases join",
        "A page merge keeps both bodies' original text and layout",
        "clashing or repeated properties, including equal values",
        "stops with **Q-TITLE**",
        "It changes no files",
        "use their first spelling",
        // CM step 2, model r6 O6: union by reading, survivor header, leftover.
        "the page that remains keeps its own header",
        // Review round 2: keys and the leftover title after the rename's map.
        "`old::` and `new::` are then one key",
        "names that read as the same page",
        "made only when there are such lines",
        "Org source header lines, drawer entries included, stay inside a quote",
        "blocks hang under the source page's header but nothing of that header is left",
        "an Org source drawer entry clashes with one the remaining page has",
        "**F-LINK** means the parser could not locate an alias name safely",
        "holds other text besides names",
        "Saving it again keeps the survivor's name",
        "Tag-shaped aliases keep their `#` and existing brackets",
        // GH #535: when an unsaved page still stops a rename, or a refusal
        // reads as arbitrary.
        "stops the rename only if the rename would change that page",
        "Other pages keep their unsaved edits through the rename",
        "or in Org by `#+TITLE:`, gets the new name",
        "Opening a graph never renames journal files",
        "**Rename to date names**",
        // CM M-Q6, M-Q7, R14, B4's survivor substitution, model r6 O2/O3.
        "offers to merge into the journal, whichever way the date is written",
        "is refused, and the message names the page that name opens",
        "Renaming a page to an alias it declares itself is one step",
        "a day cannot be renamed, and no rename respells one",
        "name the page that remains (`Machine Learning`)",
        "`ML notes/x` becomes `Machine Learning/x`",
        "inside its own namespace that is already used",
        "is refused with that page's name",
    ] {
        assert!(
            tips.contains(control),
            "missing family-16 Guide control: {control}"
        );
    }
}
