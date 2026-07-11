# Porting a Logseq or Obsidian plugin

Do not build an API-compatibility shim. Give the original source and this document to
an agent and ask it to re-express the **user-visible behavior** using Tine's manifest,
events, and effects.

## A useful agent prompt

> Study this plugin and describe its user-visible behavior separately from its
> Logseq/Obsidian implementation. Port only the parts expressible through Tine plugin
> API 0.1. Do not emulate the legacy API, add ambient authority, write files, call the
> network, inject HTML/CSS, or bypass Tine effects. Declare the minimum capabilities,
> explicit platforms, public source/license, and AI-development provenance. Run
> `npm run plugin:check -- <dir> --json` and include the report.

## Common mappings

| Legacy behavior | Tine-native shape |
|---|---|
| command palette registration | `commands.register` + a command contribution |
| slash command inserting text | `slash-commands.register` + `insert-at-caret` |
| update the currently edited block | `graph.write.block` + expected-text replacement |
| decorative bullet/thread UI | a host-owned `blockDecorations` kind |
| plugin settings | plugin-local scalar settings |
| direct Datascript query | not available; request a narrow host query event if broadly useful |
| `provideUI`, React component, DOM/CSS injection | not available; propose a constrained host-rendered contribution |
| filesystem, shell, Git, network | privileged and unavailable in ordinary API 0.1 |

If the behavior does not fit, stop and document the missing semantic operation. Do
not smuggle it through encoded notices, giant settings values, raw markup, or a
different platform. A good missing operation may become a small reusable host API;
an app-specific compatibility layer will not.

Porting changes maintenance ownership: the resulting plugin targets Tine's small API
and need not track Logseq's Datascript schema, Obsidian's Electron/DOM assumptions, or
either application's private internals. It may deliberately support fewer features.
