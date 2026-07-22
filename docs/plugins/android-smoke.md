# Android plugin smoke test

Use the APK produced from the exact `plugins` branch commit. Record the phone model,
Android version, APK SHA-256, and pass/fail notes. Do not tag 0.6.0 until every step
passes or a failure is explicitly accepted.

1. Install the APK. If an older Tine build uses a different certificate, uninstall
   that build first; export or back up any phone-only graph before doing so.
2. Open an existing graph and confirm ordinary editing and saving still work before
   installing anything.
3. Open **Settings → Plugins**. Confirm the signed catalogue shows Bullet threading,
   Query filter shortcuts, and Heading level shortcuts with their safety labels.
4. Install Bullet threading, enable it, and verify nested connectors. Open its
   settings, switch to **Active ancestry only**, focus a nested block, and verify
   only that ancestry is emphasized. Disable, re-enable, and finally uninstall it;
   notes must remain unchanged.
5. Install and enable Query filter shortcuts. Turn a query into a table or board,
   focus its source block, open search, type `hide completed`, and tap
   **Query view: hide completed rows**. Confirm completed rows disappear, Undo
   restores the block, and running the command on ordinary prose changes nothing.
6. Install and enable Heading level shortcuts. Edit a normal block, open search,
   type `heading level 1`, and run **Heading: level 1**. Confirm its rendering changes,
   Undo works, and **Heading: clear level** restores plain text.
7. Open **Settings → Appearance**, install each community theme, select it, switch
   light/dark mode, then uninstall it. Confirm only colors change and the graph text
   does not.
8. Force-stop and reopen Tine. Confirm enabled plugins, settings, and the selected
   theme persist, and that the graph still opens normally while offline.
9. Reconnect, reopen the catalogue, and confirm no crash, repeated install, or stale
   “Verifying…” state. Capture screenshots and report any Android permission prompt;
   ordinary plugins and themes should request none.
