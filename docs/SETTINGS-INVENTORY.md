# Core settings disclosure inventory

Reviewed 2026-07-12 for GH #112. Basic settings stay directly visible because
they are common workflow choices, accessibility controls, or safety-relevant.
Advanced settings are niche compatibility, experimental, or troubleshooting
controls. Search indexes both levels.

| Tab | Setting | Level | Rationale |
|---|---|---|---|
| Appearance | Theme and theme gallery | Basic | Primary appearance choice. |
| Appearance | Accent color | Basic | Common appearance choice. |
| Appearance | Interface size | Basic | Accessibility and readability. |
| Appearance | Wide mode | Basic | Common reading-layout choice. |
| Appearance | Document mode | Basic | Common prose/outliner choice. |
| Appearance | Typographic replacements | Basic | Visible text-rendering behavior. |
| Appearance | Auto-pair brackets & quotes | Basic | Core typing convention with reasonable preferences both ways. |
| Appearance | Space after inserting a reference | Basic | Existing OG-compatibility choice; divergence remains explicit. |
| Appearance | Dim in focus mode | Basic | Focus/readability behavior. |
| Appearance | Load local-file images | Basic | Security-sensitive permission must remain visible. |
| Appearance | Smooth scrolling (experimental) | Advanced | Experimental WebKit feel workaround. |
| Appearance | System title bar and window controls | Basic | Primary window-management choice. |
| Editor | File format | Basic | Determines new files' durable format. |
| Editor | Spell checker and languages | Basic | Accessibility and language support. |
| Editor | Click a block reference to zoom in | Basic | Common navigation convention and explicit OG divergence. |
| Editor | Link autocomplete default | Advanced | Niche completion ordering/compatibility preference. |
| Editor | Reuse already-open tabs | Advanced | Niche navigation policy. |
| Editor | Learn Ctrl+K choices and reset ranking | Advanced | Optional device-local, graph-scoped tie-breaking; saved search/query order remains deterministic. |
| Editor | Copy parent sub-blocks | Advanced | Clipboard compatibility policy. |
| Editor | Strip collapsed:: when copying | Advanced | Clipboard metadata compatibility policy. |
| Journals | Journal date format | Basic | Primary journal identity/display choice. |
| Journals | First day of week | Basic | Locale/calendar behavior. |
| Journals | Carry-over buttons | Basic | Feature discoverability. |
| Journals | Carry-over keeps context | Basic | Data-shape choice during a visible operation. |
| Journals | Carry-over header | Basic | Visible output choice. |
| Journals | Carry last N days | Basic | Parameter for a visible command. |
| Journals | Task workflow | Basic | Core task markers. |
| Journals | Time tracking | Basic | Durable LOGBOOK writes must remain visible. |
| Journals | New-journal template | Basic | Common journal workflow. |
| Journals | Agenda window | Basic | Primary agenda scope. |
| Journals | Quick-capture Enter key | Advanced | Niche alternate submit convention with a documented shortcut. |
| Files | New asset filename | Basic | Durable on-disk naming. |
| Files | Watch for external edits | Basic | Data-safety and filesystem compatibility. |
| Files | Diagram editor commands | Advanced | Optional external-tool integration. |
| Files | Orphan assets and trash | Basic | Data-management and recovery actions. |
| Backups & recovery | Snapshots to keep | Basic | Recovery policy. |
| Backups & recovery | Journal and sync conflicts | Basic | Data-safety actions. |
| Graph | Graph folder and publishing | Basic | Primary graph/export operations. |
| Help improve Tine | Diagnostics controls | Basic | Explicit report/privacy workflow. |
| Keyboard shortcuts | Command bindings | Basic | Dedicated searchable tab; no second disclosure layer. |
| About | Version, updates, licenses | Basic | Product and legal information. |

Plugin-owned settings are intentionally not covered by this master-branch
inventory. Their `basic`/`advanced` schema and host rendering remain part of the
plugins work tracked for 0.6.x.
