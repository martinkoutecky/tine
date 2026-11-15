# How Tine is built: fifteen principles

Tine is written by AI agents (Claude Code and Codex) and directed by one person, Martin, who decides what it should do and never reads the code. These fifteen principles are given to every agent that changes Tine, and each change is reviewed against them, together with a longer checklist, before it is merged. The list is kept to fifteen on purpose: a new principle has to replace or merge with an old one. The first outranks the rest.

This file is the only copy: every agent's prompt includes it as it stands here, and it changes only on Martin's decision. It was published as [discussion #962](https://github.com/martinkoutecky/tine/discussions/962) in November 2026.

**1. Your files come first.** Every save goes through one careful path: write a temporary file, flush it to disk, rename it into place, and check that nothing changed the file in the meantime. Your Markdown and Org files are the truth; everything Tine derives from them is a disposable cache that is rebuilt, never migrated, and never a reason to refuse to open your notes. Whatever Tine writes stays readable by Logseq.

**2. The lowest possible cost per operation.** Every operation should be as fast as it can possibly be. If it is not constant time, ask "is there any fundamental obstacle to this being faster?", and if the answer is no, make it faster. Caches and indices are often the right answer, but they must never cover up a slow design. Speed is why Tine exists: no slowdown is dismissed as a corner case, Tine should run well on old machines too, and a performance budget is never moved to excuse a slowdown. Memory follows the same rule as time: Tine keeps only what it still needs, so what a closed page needed is let go. But memory weighs more, because a slow operation makes Tine freeze for a moment, while too much memory can make it unusable or crash it.

**3. Start from the lower bound.** Before writing an operation, write down what it fundamentally needs (files read and written, round trips, re-renders) and make the code match that within a small constant. Test both counts and timings, because "Timings catch when the complexity increases because of something that doesn't get counted and counts catch the slowly accumulating cruft." Big constants come from code working out again what it was already told, so a change carries what changed, freshness is a cheap comparison rather than a re-check, and work for hidden parts of the screen waits until they are shown.

**4. Reuse before inventing.** Before adding a new screen, setting, command or file format, show why something Tine already has can't do the job: a page (your Favorites are just a page), a query, the outline itself, an editable preview. "The point is good UX and good code; this is not an ideological stance", and it can be overdone.

**5. Logseq's behaviour by default; the state of the art beyond it.** Where Logseq has an answer, Tine starts from it, checked against Logseq's source, and any difference is a named decision, never an accident. Before designing anything new, we study how others solve it, above all tools refined in high-stakes settings, "where somebody probably sank real money into figuring out what is a good way to design something". We match the best of them, then ask whether Tine's own features let it go a step further. When two good answers pull against each other, Martin decides.

**6. Editable everywhere.** "Anything that is editable somewhere is editable everywhere": a block shown as itself (in a reference, an embed, a query result, the sidebar, a hover preview) is the real block, and you can edit it right there. The exceptions are old versions, search excerpts, exports and the built-in Guide.

**7. Calm and discoverable.** Controls appear when you reach for them, yet you can still find out what Tine can do. We cut duplicates and use progressive disclosure, and anything hidden keeps a way to find it. If you don't need a feature that others rely on, you can switch it off: "Configurability is good; it should just not clutter configuration UIs."

**8. One answer, one place.** Every question Tine answers (what links here, which file a page name opens, a setting's default) is computed in exactly one place. Two computations of the same answer are a bug, even if a test keeps them in step. A fact both the interface and the core need lives once, in the core. Logseq's structure is read by the parser, never by a regular expression.

**9. Deep modules, honest interfaces.** Small interfaces over hidden machinery; callers depend on what a module answers, not how. A function's signature tells you what it costs (graph-wide work says so in its name) and whether its answer can be out of date, and callers never have to ask whether something is "ready yet".

**10. Minimum code.** Nothing speculative: no features or options nobody asked for, no abstraction without a second use, no handling of impossible cases. Delete what a change makes obsolete; a rewrite replaces the old code in the same change, with no compatibility shims. If 200 lines could be 50, rewrite them.

**11. Fix the shared cause.** Every changed line traces to the task; neighbouring code is left alone. A bug is fixed at the narrowest place that explains the whole family of failures (every way in: keyboard, mouse, pane, platform), not only the line in the report. If that grows into an app-wide change, it is split out or brought to Martin.

**12. The second patch stops the line.** A second fix in the same area means: stop, ask what question that area answers and how many places answer it, and fix the design rather than patching again.

**13. Legible to one person.** Tine is run by one person through agents, so complexity is measured by the length of our own code and models, and size is watched. "Logseq does it this way" does not make a rule simple: "The total complexity is just the length of our model." Specifications state the rules as they are; the history is in git.

**14. Proof, not claims.** A fix comes with a test that fails before it and passes after it, checking what you would see where you saw it. A claim in a comment or a design document ("never called from X") is enforced by a test or deleted. Tests are never weakened or deleted to get a green run.

**15. Nothing important is decided silently.** File formats, public and plugin interfaces, defaults, security boundaries and product behaviour are decisions: the agent stops and brings Martin a recommendation. Earlier decisions are looked up, not remembered, and contradicting one is treated as a defect. A model of what something means is finished before its code is written.

*Tine is young software, and no human has reviewed its code. Keep backups.*
