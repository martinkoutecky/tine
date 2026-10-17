
# Query and supporting controls (GH #107). Variables carry authored text or syntax verbatim.
document-review-unsaved = Review unsaved
document-create-unreadable = Couldn't create “{ $name }”: { $hasOwner ->
        [1] { $owner }
       *[other] a page file
    } can't be read and may already be this page. Fix or move that file; your edits stay in the editor.
document-read-refused = Couldn't read “{ $name }” from disk; your edits stay in the editor.
document-draft-refused = Couldn't keep a crash-recovery copy of “{ $name }”; your edits stay in the editor.
document-save-refused = Couldn't save “{ $name }” — { $detail }
document-save-twin = Couldn't save “{ $name }”: { $existing } is already this page. Your edits stay in the editor.
document-save-undecodable = Couldn't save “{ $name }”: its file is not readable text. Your edits stay in the editor.
# step is an unchanged diagnostic suffix naming a save operation and OS error.
document-save-retrying = Couldn't save “{ $name }” yet{ $step }; Tine keeps trying and keeps a crash-recovery copy.
document-operation-unrecorded = A rename or delete of “{ $name }” that Tine couldn't confirm did not happen: it couldn't be recorded. Nothing changed.
document-draft-unwritten = Couldn't write the crash-recovery copy of “{ $name }”.
# after is a caller's description of the operation the refusal stopped.
document-load-refused = “{ $page }” is open from { $hasHolder ->
        [1] { $holder }
       *[other] a new page that is not saved yet
    } with unsaved work, so { $hasRequested ->
        [1] { $requested }
       *[other] the page
    } was not loaded. Finish or save that edit first.{ $hasAfter ->
        [1] { " " }{ $after }
       *[other] { "" }
    }
document-move-incomplete =
    { $completed ->
        [0] The move did not happen: “{ $pages }” could not be saved first.
       *[other] Only part of the move happened: the blocks from “{ $pages }” did not move.
    }
document-external-updates = { NUMBER($pages, useGrouping: 0) } pages updated externally{ $conflicts ->
        [0] { "" }
        [one] { " " }· { NUMBER($conflicts, useGrouping: 0) } conflict to review
       *[other] { " " }· { NUMBER($conflicts, useGrouping: 0) } conflicts to review
    }

document-wiring-tine-can-t-save-this-graph = Tine can't save this graph: { $value1 }

document-wiring-saved-the-page-but-could-not-refresh-its-title = Saved the page, but could not refresh its title: { $value1 }

document-wiring-saved-the-title-but-its-page-identity-could-not-be = Saved the title, but its page identity could not be adopted safely. Reopen this page by its file path.

document-wiring-resolve-the-conflict-on-first = Resolve the conflict on “{ $value1 }” first.

document-wiring-changed-meanwhile-choose-again = “{ $value1 }” changed meanwhile. Choose again.

document-wiring-couldn-t-overwrite = Couldn't overwrite “{ $value1 }”.



document-wiring-couldn-t-read-the-page-is-an-alias-of-its = Couldn't read { $value1 }, the page “{ $value2 }” is an alias of; its text stays here, unsaved.

document-wiring-changed-since-was-added-to-it-copy-the-rest-of = “{ $value1 }” changed since “{ $value2 }” was added to it; copy the rest of “{ $value3 }” over by hand.

document-wiring-moved-into-its-alias-owner = Moved “{ $value1 }” into its alias owner “{ $value2 }”.


