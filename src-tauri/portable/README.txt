Tine portable data folder
=========================

Because this "tine-data" folder sits next to the Tine executable, Tine runs
in portable mode. It keeps its settings, sessions, backups, plugins,
diagnostics and WebView2 data here, not in your user profile's AppData
folder:

  tine-data\<app id>\roaming   (an installed Tine uses %APPDATA%\<app id>)
  tine-data\<app id>\local     (an installed Tine uses %LOCALAPPDATA%\<app id>)

<app id> is page.tine.Tine for Tine and page.tine.TineBeta for Tine Beta.
Your notes stay in your graph folder, wherever you keep it.

To update, quit Tine and extract the new portable ZIP into this same folder.
Keep this tine-data folder.

If Tine cannot write to this folder (for example a write-protected drive),
it does not start: it says so and names the folder. Move the whole Tine
folder somewhere you can write to and start it again. It never falls back to
AppData, so your settings are never split between two places.

To move settings from an earlier run of portable Tine on this computer, quit
Tine and copy %APPDATA%\<app id> to tine-data\<app id>\roaming and
%LOCALAPPDATA%\<app id> to tine-data\<app id>\local. Tine never moves or
deletes those folders itself.

Without a tine-data folder beside it, Tine uses the usual AppData location.
A tine-data folder inside an installed Tine's folder (the one with
uninstall.exe) has no effect, and neither does a shortcut, link or junction
named tine-data: it must be a real folder.
