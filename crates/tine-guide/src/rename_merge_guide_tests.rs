#[test]
fn rename_merge_and_journal_rename_proposals_are_documented_in_the_bundled_guide() {
    let tips = include_str!("templates/tips.md");
    for control in [
        "offers to **merge** them",
        "the aliases join",
        "A page merge keeps both bodies' original text and layout",
        "clashing properties, including equal values",
        "Org source directives stay inside a quote",
        "Saving it again keeps the survivor's name",
        "Tag-shaped aliases keep their `#` and existing brackets",
        // GH #535: when an unsaved page still stops a rename, or a refusal
        // reads as arbitrary.
        "stops the rename only if the rename would change that page",
        "Other pages keep their unsaved edits through the rename",
        "or in Org by `#+TITLE:`, gets the new name",
        "Opening a graph never renames journal files",
        "**Rename to date names**",
        // CM step 2 batch 12: M-Q5 (D-20261102-MC5), M-Q6, M-Q7, rulings (a)(b), R14.
        "offers to merge into the journal, whichever way the date is written",
        "**Make “ML” its own page (still an alias of “Machine Learning”)**",
        "links to `[[ML]]` still show in its Linked References",
        "name the page that remains (`Machine Learning`)",
        "inside its own namespace that is already used",
        "is refused with that page's name",
    ] {
        assert!(
            tips.contains(control),
            "missing family-16 Guide control: {control}"
        );
    }
}
