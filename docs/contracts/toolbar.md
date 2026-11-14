# Toolbar placement

Prior decisions: follow D-20261113-UI (toolbar customization and a persistent
right-sidebar button), TOOLBAR-390B (unchanged default bar), and the approved
two-list device record in the #926 implementation brief.

Optional actions are Calendar, Journals, Share, Theme, Right sidebar,
Workspaces and Settings. The left-sidebar toggle, Search, Back, Forward and
overflow trigger are fixed. The existing fixed navigation width tiers apply.

`src/toolbarChoice.ts` owns optional action order and placement. No choice means
the shipped responsive default, including all six non-workspace optional
buttons on a 390 px phone. The compact workspace switcher gives way at the
386 px content-box boundary. Right-sidebar visibility does not depend on content.

The `toolbar_actions` device string preference in `tine-settings.json` encodes
one JSON record, `{ "bar": [action ids], "menu": [action ids] }`. Both lists
preserve order. Unknown ids are ignored, duplicates count once (bar wins), and
new actions join the end of the menu. Reset removes this key through the
audited device settings update; no graph files change.

Saved choices overflow a right suffix of the bar into the menu before its
chosen rows. Enabled toolbar buttons follow the saved order for keyboard focus.
The menu includes exactly the available optional actions absent
from the bar, followed by fixed navigation when its tiers require it and
Customize toolbar. Disabled actions such as Share with no page remain present.

Appearance reuses the Settings descriptor/search framework. Dragging and
explicit up/down and section-move buttons save immediately through the shared
preference writer; failed writes roll back and show its persistent error toast.
The overflow customization row opens Appearance and uses the existing setting
search jump to reveal Toolbar, even below a long theme list.

Cost: one startup preference read, one ordered device write per chosen move,
one ResizeObserver content-box measurement per resize and O(optional buttons)
layout work. The number of chosen buttons fundamentally bounds the fit scan.
No graph reads, stats, graph writes, or IPC on resize, navigation or keystrokes.
The observer disconnects on unmount; hidden menus mount on demand.

Unit cost: 0 bytes, files and transport bytes per note edit on both 1-block and
60-block pages. One preference move rewrites the existing device settings file
through its audited update. The record's two lists contain at most seven known
ids (97 UTF-8 payload bytes with all seven on the bar). A settings file holding
only this key is 143 bytes with the existing pretty-JSON writer and newline;
other device preferences share that file. No sync transport.
