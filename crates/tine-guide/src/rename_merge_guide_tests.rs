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
        "holds other text besides names, such as a file link",
        "write the aliases there as plain names separated by commas",
        "A file link or a macro such as `{{embed x}}` alone on an alias line declares no alias",
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
        // CM step 2 slice 2, model r6 O4 (touched names) and O7 (report).
        "and another page is already `B/x`, or when several files claim one",
        "Tine lists the files where that happened",
    ] {
        assert!(
            tips.contains(control),
            "missing family-16 Guide control: {control}"
        );
    }
    // Model r6 O7 and O8 (slice 2): the report and the settings step.
    let pages = include_str!("templates/pages-links-references-search.md");
    for control in [
        "A page whose title is a date can receive a merge",
        "A title in a first-bullet property header follows a rename too",
        "Blank lines before a first-bullet property header keep its properties and children in place during a merge",
        "it writes the new title there and keeps the header's child blocks beneath it",
        "An indented source can merge while keeping its block depths",
        "Different indentation on later blocks keeps their continuation text and fenced code intact",
        "An Org page ending in blank lines can receive a merge, including a page containing only a header",
        "Opening prose with spaces or tabs on trailing lines can merge too",
        "resolve the conflict before merging; nothing is merged",
        "**Save it and rename** saves that file normally",
        "Each further file needs another click",
        "a successful save stays even if the rename cannot finish",
        "Tine's own check failed means a bug in Tine",
        "choose two different files, use matching Markdown or Org formats",
        "Some ordinary files exceed the merge writer's current capabilities",
        "rewrite an alias declaration as plain comma-separated names",
        "remove blank lines before or between the property lines at the top of that file",
        "nothing was renamed or merged",
        "Pending edits may already have been saved before a refusal; those saves stay",
        "Tine lists the files where either happened",
        "and a property key `a/x::` into `b/x::`",
        "The label of a file link that names the page",
        "after a merge each page is listed once in Favorites, in its first place",
        "none of it changes and Tine says so",
    ] {
        assert!(pages.contains(control), "missing Guide control: {control}");
    }
}
