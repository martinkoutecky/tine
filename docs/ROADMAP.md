# Roadmap

*Written by Claude (Opus 5.5), read and approved by Martin. Last updated 2026-11-14.*

This page says where Tine is going as Martin sees it today. It is not a promise: it changes whenever the plan
changes, and this file's git history shows every change. Each issue with a planned version carries it as a
GitHub milestone, so the issue lists linked below are always current.

Tine has three lines. **Stable** is what most people run. **Beta** is the next version, for testers; it takes
bug fixes only. **Development** is the version after that, where new work lands first; only Martin runs it day
to day.

## 0.9: in Beta now

Beta since 29 October. It brings git sync on phones and computers, **Changed since you last looked**, encrypted
quick-share links, margin comments, and page saving tested against a published storage model. It becomes
stable after a soak with no open severe regression.

[Issues planned for 0.9](https://github.com/martinkoutecky/tine/milestone/4)

## 0.10: in development

- **The CodeMirror editor becomes the default.** It shows formatting as you type; Martin has used it daily
  since early November. The classic editor stays: **Settings → Editor → Advanced → Use classic editor**. If you
  switch back because something in the new editor is wrong, please report it: the plan is to retire the
  classic editor.
- **Margin comments, second round**: threads that collapse, next and previous comment, an "on click" mode,
  and layouts for narrow windows and phones.
- **Right-to-left languages**, Persian first ([#823](https://github.com/martinkoutecky/tine/issues/823)).
- Many smaller changes, for example a Logseq-style properties panel ([#920](https://github.com/martinkoutecky/tine/issues/920)) and several keys for one command
  ([#878](https://github.com/martinkoutecky/tine/issues/878)).

[Issues planned for 0.10](https://github.com/martinkoutecky/tine/milestone/5)

## 0.11: plugins, and three things moved from 0.10

One release with everything plugins need, so that "Tine has plugins" means it when it is announced: a much
wider plugin API ([#790](https://github.com/martinkoutecky/tine/issues/790)), plugins that draw a whole view of the graph (for example columns, or a graph view,
[#183](https://github.com/martinkoutecky/tine/issues/183)), a plugin and theme store ([#822](https://github.com/martinkoutecky/tine/issues/822)), and a keyboard-shortcut engine shared with plugins ([#284](https://github.com/martinkoutecky/tine/issues/284), discussion
[#383](https://github.com/martinkoutecky/tine/discussions/383)).

Plugin design starts once 0.9 is stable. What you want plugins to do is collected in discussion
[#728](https://github.com/martinkoutecky/tine/discussions/728); every request there gets an answer from the design.

Moved here from 0.10 on 14 November:

- **A written model of what pages, journals, names and renames mean** ([conceptual model](conceptual-model/SPEC.md)),
  with machine-checked proofs, and Tine changed to follow it. This is where edge cases such as renaming onto
  a date or an alias get one consistent answer. The first parts are in 0.10; the rest comes here.
- **Conflict review**: when two devices changed the same page, Tine shows the merged outline before you apply
  it ([#901](https://github.com/martinkoutecky/tine/discussions/901)), and conflicts can be resolved from the command line
  ([#902](https://github.com/martinkoutecky/tine/issues/902)).
- **MCP, first version**: agents can read and write your graph through the Model Context Protocol, with the
  same commands and the same safe save path as the `tine` command line. Off by default; you switch it on per
  graph on each device ([#109](https://github.com/martinkoutecky/tine/issues/109)).

How these and plugins are ordered inside 0.11 is not settled yet.

[Issues planned for 0.11](https://github.com/martinkoutecky/tine/milestone/6)

## Later

- **Shared graphs**: several people writing one graph (who wrote what, what each person sees). One design
  discussion covering the related issues, probably for 0.11.
- **Supertags**, after plugins (0.12 at the earliest).
- [Everything else that is open](https://github.com/martinkoutecky/tine/issues?q=is%3Aissue+is%3Aopen+no%3Amilestone):
  no version yet. That is not a no; a no is said on the issue.

## How choices are made

Logseq's behaviour is the default, and a difference from it is a decision someone can name. When two good
defaults pull against each other, Martin decides. A feature some people don't need but others rely on is
something those people can switch off, rather than something removed. Speed matters at every step: each
operation should cost as little as it fundamentally can.

Martin does not read Tine's code; Claude and other AI models write and review it, against these rules and a
checklist each change is reviewed against. Keep backups of your graph (git, or a copy of the folder).

## How this page is kept

Claude updates it at every Beta release and whenever Martin changes what a version contains. The per-issue
detail lives in the milestones, not here.
