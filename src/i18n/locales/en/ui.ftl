
# Query and supporting controls (GH #107). Variables carry authored text or syntax verbatim.
ui-sync-conflicts =
    { $count ->
        [one] { NUMBER($count, useGrouping: 0) } new sync conflict needs review
       *[other] { NUMBER($count, useGrouping: 0) } new sync conflicts need review
    }
ui-review = Review
ui-save-preference-failed = Could not save { $preference ->
        [keyboard] keyboard shortcuts
       *[other] display preference
    }.

ui-ui-could-not-load-display-preference = Could not load display preference.


ui-ui-could-not-save-journal-title-format = Could not save journal title format: { $value1 }

ui-ui-could-not-check-for-duplicate-journal-days = Could not check for duplicate journal days: { $value1 }


ui-ui-could-not-enter-fullscreen-focus-mode = Could not enter fullscreen focus mode.

ui-ui-could-not-leave-fullscreen-focus-mode = Could not leave fullscreen focus mode.

ui-ui-could-not-check-some-sidebar-blocks-try-again-after-the = Could not check some sidebar blocks. Try again after the graph loads.

ui-ui-pdf-export-needs-the-desktop-app-a-mobile-webview-cannot = PDF export needs the desktop app: a mobile WebView cannot print.

ui-conflict-policy-could-not-remember-always-ask = Could not remember “Always ask”: { $value1 }

ui-conflict-policy-could-not-load-always-ask = Could not load “Always ask”: { $value1 }

ui-main-plugins-unavailable = Plugins unavailable: { $value1 }

ui-main-community-extensions-unavailable = Community extensions unavailable: { $value1 }

