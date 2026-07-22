# Sol's findings on how Martin developed Tine

Status: research notes and source material, not a Reddit post draft.

Snapshot date: 2026-07-12.

## 0A. Scope, evidence, and limits

This is an introspective account of what stands out in the Tine-related history I can inspect. It is not a comparison with other people who build software with agents, and it is not an attempt to assign percentages of authorship between Martin and the models.

The strongest primary sources were:

- Codex root session, especially the lsdoc source-led work, Tine audits, PDF/clipboard work, release failures, and sync architecture.
- Codex root/continuation session., especially multi-graph/window work, data-safety hardening, rapid issue fixing, releases, and plugin design. 
- Codex root session, the long July 11–12 thread covering collapse/editing audits, real-app E2E, releases 0.5.5/0.5.6, release-process hardening, and the autonomous issue workflow.
- The repository history, ADRs, changelog, regression catalogs, release checklist, issue workflow, CI, and test harnesses.
- Compiled daily history under, which supplies some earlier context not present in the accessible Codex root threads, including the nine-round June data-integrity audit.

The available Codex history is heavily concentrated in July 9–12. Most of Tine was developed through Claude Code, so Fable's pass should carry more weight on the origin story and the earlier feature-building rhythm. These notes are strongest on the characteristic patterns that recur in the available history and on the process changes made in the last two days.

Useful scale indicators, not quality metrics:

- The current `master` history contains 914 commits from the first Rust-core commit on June 14 through July 12.
- There were 30 version tags from v0.1.0 on June 24 through v0.5.6 on July 11.
- The repo has 41 numbered architecture decisions.
- The current UI regression catalog has 70 entries: 63 tied to GitHub issues; 52 with a proven fail-before, 12 reconstructed, 3 inferred, and 3 explicitly unavailable. Coverage is split across unit, render, browser, and native layers rather than pretending everything is a native E2E test.

## 1A. Short thesis

The modern models supplied extraordinary implementation bandwidth. Martin supplied the product's north star, taste, compatibility instincts, risk priorities, real-world acceptance testing, and increasingly the operating system that lets the bandwidth compound safely.

There is no obvious magic prompt. The prompts are often conversational, incremental, and sometimes uncertain: “I may not be understanding,” “what do you think?”, “take your best guess,” or a list of things noticed while using the app. What makes them effective is that they contain high-quality control signals: a concrete interaction, a product invariant, a comparison target, a risk boundary, or an explicit stop/go decision.

A useful distinction is:

- Code generation explains much of the raw speed.
- Martin's feedback and judgment explain why the generated work repeatedly converged toward one coherent product rather than a pile of features.
- The newly hardened tests, release process, and issue workflow are intended to make that convergence less dependent on Martin manually checking everything.

## 1B. The project had a legible north star from the beginning

Tine is not “a note app” in the abstract. Its core proposition is unusually constraining: a fast, local-first, Logseq-compatible outliner that operates directly on the user's real Markdown/Org graph, without import/export or lock-in.

That north star answers many otherwise independent design questions:

- Performance is the reason the rewrite exists, not a later optimization phase.
- Plain files and Logseq compatibility matter more than inventing a cleaner private format.
- The frontend owns the live edit tree so typing does not cross IPC.
- A pure Rust core handles graph-wide work.
- External edits and Syncthing-style workflows must not lead to silent overwrite.
- Linux is the primary platform and WebKitGTK behavior is a real product constraint.
- Original Logseq is the default semantic reference, while deliberate divergences need a reason.

The first two days of git history already contain the Rust round-trip core, SolidJS outliner, Tauri shell, refs/backlinks/search/query engine, PDF annotation, tabs, format-preserving saves, a conflict guard, file watching, an AGPL license, and a privacy-safe round-trip checker. The direction was broad, but it was not directionless.

## 2A. Martin develops by continuously using the deployed product

The most repeated practical pattern is a very short loop:

1. The agent implements, tests, pushes, and deploys a fresh production binary.
2. Martin uses that binary on a real workflow.
3. He reports the next failure at interaction-level precision.
4. The agent traces the mechanism, adds evidence, and deploys again.

The reports are often about details that are hard to infer from a feature checklist:

- Moving upward from a single-line block into a wrapped block should land on its bottom visual line; moving downward has the mirror rule.
- Entering edit mode must not spread Markdown delimiters apart visually.
- Quick capture should mean “shortcut, then type,” without a mouse click.
- Clicking inside inline code should put the caret at the clicked character, not at an edge.
- Shift-clicking a bullet should open it in the sidebar.
- Collapsing, zooming, split panes, tables, embeds, and media must preserve the correct ownership and focus behavior.

This is not a formal QA department, but it is high-bandwidth acceptance testing by someone who uses the product and notices interaction texture. It explains both the polish and the rapid discovery of regressions.

It also explains why “deployed” is a stronger state than “the tests pass.” The working agreement now says that ready/fixed/done means built and deployed to `~/research/tine`. That rule emerged because a pushed or locally tested change is not yet in the loop Martin actually uses.

## 2B. He usually specifies behavior, not implementation

Martin rarely tells the agent which file, class, or algorithm to change. He describes the desired interaction and supplies a counterexample. Examples include:

- “quick-capture shortcut -> start typing”;
- an annotation block reference should open the corresponding PDF page;
- selecting text and typing a literal delimiter should wrap the selection under the same conditions as Logseq;
- a page chosen explicitly in another window should become the capture target;
- select-mode sheet paste should replace cells, while edit-mode paste should create nested content.

This division is high leverage. Martin carries the product model; the agent carries most of the codebase search, implementation, and verification burden.

The prompts can look casual because the precision lives in the behavioral example. A one-sentence caret report can encode the relevant geometry more clearly than a page of implementation instructions.

## 3A. Product taste appears as principled scope control

Martin is not primarily minimalist. He is often favorable toward configurability and useful features. The recurring concern is whether a first implementation should become a permanent commitment in Tine core.

Examples:

- The “arbitrary webpage to outline” request was not implemented literally. Research separated faithful structural paste, heading-derived outlines, Readability-style article extraction, site-specific rules, and AI reconstruction. The chosen core behavior was the narrow deterministic layer; richer transforms were left as plugin-shaped future work.
- Bullet threading belongs in the existing plugin, with plugin-owned settings as a platform requirement, rather than being duplicated in core.
- A niche shortcut can exist unbound and configurable without displacing a good default interaction.
- Advanced settings are considered when configurability is valuable but the common settings surface would become noisy.
- Plugins are treated as a way to make a useful capability available without declaring the first design permanent core policy.

This is an important answer to “how can features be added so fast without losing the plot?” Fast code does not automatically imply a core commitment. Martin repeatedly asks whether a feature is broadly correct, a niche plugin, a plugin-API extension, a future-minor item, or something to defer.

## 3B. Open-source experience supplies a large comparison set

Martin's long Linux/open-source background is visible less as named ideology than as default expectations:

- Users should own interoperable files.
- Compatibility should be checked against actual upstream behavior, not a vague recollection.
- Platform support should be described honestly; Linux can be primary without pretending Windows/macOS evidence is equivalent.
- Settings and extension points are preferable to unnecessarily forcing one workflow, but capability boundaries and maintenance cost still matter.
- A public issue tracker is a relationship with users, not merely an internal task queue.
- A public project board should be inspectable without asking the agent what is in its memory.
- Contributions and public comments need transparent authorship and respectful follow-up.
- Features should be compared with the best relevant products, not invented in isolation. The comparison target can be Logseq, Obsidian, TreeSheets, Gmail, a CRDT system, or another tool depending on the problem.

The working rule “default to OG parity” is empirical. For lsdoc this became source-led transcription of mldoc rather than trial-and-error output matching. For issue #83 it meant inspecting why Logseq incidentally accepts a literal Alt+`[` on some layouts and discovering that Tine alone had an explicit `altKey` rejection. The final fix was smaller and more principled than either a broad new toggle or a dismissal of the report.

## 3C. Strong taste does not mean refusing to revise a view

Issue #83 is a useful small case study:

1. The requested Alt-based rhythm initially looked niche.
2. An unbound command and a wrapping toggle were discussed.
3. Settings clutter, keyboard layouts, and the reporter's concern were considered.
4. Martin then asked the more principled parity question: if Logseq's behavior is incidental on the reporter's platform, why does Tine not behave incidentally the same way?
5. Source inspection found a one-condition divergence.
6. The result allowed Alt only when the actual `event.key` is a wrapping delimiter, kept nonliteral layout characters untouched, and let explicit bindings win.

The reporter's tone did not determine the technical answer. Neither did Martin's first intuition. The deciding evidence was upstream mechanics plus a narrow behavioral test.

## 4A. He delegates implementation heavily but keeps explicit decision boundaries

A recurring session pattern is:

1. Investigate or research.
2. Explain alternatives and risks.
3. Write a detailed plan that survives compaction or absence.
4. Martin resolves product, architecture, and risk questions.
5. Martin says “go.”
6. The agent implements the complete agreed batch autonomously.

This appeared in multi-graph/window work, sync, plugins, audit remediation, release architecture, and the recent issue-planning session.

Martin explicitly corrected the collaboration when the agent began investigating too soon: planning time should be used to decide and queue work; builds begin only after “go.” This was not ceremony. It makes unattended execution possible because acceptance criteria and boundaries are decided while Martin is present.

The intended daily rhythm is now:

- While Martin is away: investigate, fix clear bugs, ask reporters precise questions, push/deploy verified changes, and prepare decision packets.
- While Martin is at the keyboard: do not burn the time on builds; discuss the deferred product/API questions and queue a batch.
- After “go”: execute the batch.
- On “cut X”: run the release contract, fix bounded failures, and surface only genuine product/risk blockers.

## 4B. He actively designs the conversation to conserve human attention

Martin noticed that bare issue numbers and generic “what next?” lists made him open GitHub and reconstruct context. He asked the agent instead to lead with one named recommendation:

> My proposal for what to do next is #N — Title: what it is and why it should be next.

Logical response labels such as `1A`, `1B`, and `2A` were also made a standing rule so decisions can be answered without quoting paragraphs.

This is characteristic of the broader process: the bottleneck is no longer typing code. It is human attention and product judgment. Martin is explicitly redesigning the interface so that attention is spent on the decisions that cannot be cheaply reconstructed by the agent.

## 5A. He treats agents as powerful but fallible collaborators

Martin does not merely check whether an output sounds plausible. He often asks how the method could have allowed a mistake:

- During lsdoc work: “It does NOT look like you are reading mldoc code ... it looks like trial & error.” This pushed the work toward source-led transcription.
- After nominal performance improvements: “It's strange though that a performance improvement hasn't actually sped it up?” The honest answer was that structural cleanup had produced no measurable throughput gain.
- After editing/collapse bugs: why were there several bugs of the same family, and what adjacent mechanisms might also be wrong?
- After release failures: why were builds serialized, why was Android absent, and what architecture would prevent omissions rather than patching one YAML line?
- In the June audit, Martin rejected an auditor's claimed multi-query data-loss bug because the triggering state was not valid Logseq behavior. The finding was retracted.

The useful stance is neither blind trust nor routine distrust. The model gets broad responsibility, but important claims must become testable, and Martin is willing to challenge both conclusions and methodology.

His computer-science background shows especially in questions such as:

- What invariant would prevent the whole class of failure?
- Is the oracle independent, or can implementation and test agree on the same mistake?
- Does the performance baseline move, allowing cumulative regression?
- Did a later fix invalidate the audit that had declared the tree clean?
- Is the proposed abstraction targeting user intent, or an incidental proxy such as clipboard origin or cell emptiness?

## 5B. Risk is asymmetric

Not every change receives the same ceremony.

Data loss/corruption, hidden performance erosion, unsafe plugin capabilities, sync convergence, and partial release artifacts stop the line. Small UI fixes can move very quickly once reproduced.

Examples:

- Tine writes the same graph Martin uses with Logseq and Syncthing. The save path has base-revision guards, serialization, temp+fsync+rename, and containment rules because a rare corruption is worse than a missing feature.
- Multi-graph/window work was followed immediately by a data-safety audit and graph-scoped persistence hardening.
- Sync design spent substantial effort on stable identity, immutable operations, projection safety, crash windows, and conflict preservation before becoming a user feature.
- A contributor's change is not trusted merely because it is convenient; the code and security boundary are independently inspected.
- Performance is a product invariant because speed is Tine's reason to exist. A feature expected to exceed the budget needs a product decision, not a weaker threshold.

This selectivity helps explain the pace. Ceremony is concentrated where failure is expensive.

## 6A. Work is broken into small, independently understandable outcomes

The history is fast, but it is not one giant “build an app” generation. Even broad features move through plans, ADRs, focused commits, and repeated deployments.

The Sheets paste behavior is representative. The design went through three models:

1. infer intent from whether the target is empty;
2. infer intent from clipboard origin;
3. use the interaction mode itself: select mode splats into cells, edit mode nests content.

The third rule reflects user intent at paste time and eliminates an unsafe emptiness heuristic. It was recorded in ADR 0037 and tested as a behavior boundary.

Other coordination practices recur:

- commit meaningful chunks as work progresses;
- push continuously;
- deploy continuously;
- use dedicated worktrees for concurrent feature branches;
- write plans/ADRs before long unattended work or compaction;
- use independent subagents for audit/research lanes;
- do not switch or destructively clean a shared worktree.

These practices are ordinary engineering, but they are what make extraordinary model throughput usable rather than chaotic.

## 6B. There is real willingness to discard or defer work

The sessions do not show a pure “because code is cheap, add everything” attitude. Work can be shelved, narrowed, moved to a plugin, or assigned to a later milestone. The plugin platform itself was at one point built extensively and then shelved when Martin decided he did not want it in the foreseeable future; it was later reconsidered in a different product role. Sync was assigned to 0.7 so it had a public answer without creating pressure to build it immediately.

This reduces sunk-cost pressure. The expensive scarce resource is not generated code but long-term product and maintenance commitment.

## 7A. Audits are adversarial rather than ceremonial

The audit culture predates the newest release process. In June, Tine's write paths went through nine data-integrity rounds until a convergence check found no non-negligible issue. In July, Codex and other agents audited lsdoc correctness/performance and Tine data safety/performance/UI behavior.

What stands out is the repeated demand to look beyond the literal finding:

- How did this survive the old tests?
- What else shares the root cause?
- Is the audit whole-codebase or only recent-diff?
- Did both the implementation and the oracle inherit the same omission?
- Does the finding reproduce in a real state?
- Did the fix change code covered by an earlier “clean” audit?

The newest exact-tree criterion makes this explicit: a required focused audit is valid only for the exact final source fingerprint, and “clean” means no confirmed high or critical findings. A source fix invalidates the final sweep. Medium/low findings can ship but remain recorded for patch-cycle fix/defer/WONTFIX review.

This criterion is stricter than “an audit happened,” but it also avoids forcing every contrived medium into a release blocker.

## 7B. Each failure is expected to leave a durable asset

This may be the most important compounding habit in the recent history:

- A solved UI bug becomes a regression-catalog entry and executable boundary.
- A release omission becomes an asset-inventory gate.
- Flatpak dependency drift becomes a freshness check and real offline build.
- Android absence becomes a mandatory signed-artifact requirement rather than a successful skip.
- A stale vendored lsdoc oracle becomes a byte/hash drift gate.
- Caret and focus failures lead to native Tauri/WebKit E2E, not another jsdom assertion that cannot observe layout.
- A reporter unable to reopen an issue leads to “comment here; we will reopen it” language plus a reopen-on-comment workflow.
- An accidentally auto-closed pre-release issue leads to a ban on `fixes #N`/`closes #N` keywords before release.
- A masked performance regression leads to same-runner comparisons against both an immutable anchor and the previous release.

This is why the process can become faster despite accumulating checks: an old failure should require less human rediscovery the next time.

## 8A. The release process is a recent, substantial evolution

This section should not be projected backward onto Tine's whole history. Most of the hardened release architecture was added on July 11–12 after concrete failures in v0.5.5/v0.5.6 work.

The triggering sequence included:

- reporter follow-ups were initially omitted;
- Flatpak failed;
- desktop builds were unnecessarily serialized;
- Android was not built/published;
- the platform asset inventory could be partial;
- caret/collapse regressions had escaped the available harness;
- Martin wanted to be able to cut a release while away from his computer.

Martin's key release-architecture observation was simple: builds are slow and independent; publishing is short and can be sequential. The result moved the serialization boundary:

1. Desktop targets, Android, Flatpak, and relevant tests run concurrently and produce immutable artifacts.
2. Linux real-app E2E begins as soon as the Linux candidate exists rather than waiting for unrelated platforms.
3. One short publisher owns the release mutation and validates the complete inventory/updater map.

The release contract now requires, among other things:

- a versioned impact record transposed around every changelog item, with regression, Guide/docs, website, and blog dispositions;
- an indexed regression entry for accepted bugs;
- regenerated single-source Guide/demo output;
- the full Linux production-binary E2E catalog;
- same-runner performance comparison against immutable v0.4.7 and the previous release;
- ordinary CI, real offline Flatpak, desktop targets, and Android;
- exact artifact inventory before publication is called complete;
- issue-specific reporter follow-ups only after the relevant platform artifact exists;
- exact-tree audits for minor releases.

The distinction between machine proof and judgment is deliberate. Generated-output freshness, referenced test existence, source fingerprints, and artifact inventories can be proved. Whether a website explanation is good enough is editorial judgment, so it is made explicit and reviewable rather than falsely “proved.”

## 8B. The real-app harness was built because existing tests missed real behavior

Martin explicitly asked whether the agent could drive the real app after repeated caret failures. He did not assume the harness was good; he asked what was missing and what should be installed in the next container.

The resulting hierarchy is honest about observability:

- unit tests for pure rules;
- jsdom/render tests for component behavior that does not depend on layout;
- Chromium screenshots for visual CSS/layout checks;
- real Tauri/WebKitGTK through `tauri-driver`, WebKitWebDriver, and Xvfb for native launch, caret, focus, routing, filesystem, and window behavior.

Historical bugs were mined from closed issues, changelog entries, commits, audits, and Martin's reports into a catalog. The catalog is updated when a behavior is accepted as a bug and production work begins—not as optional cleanup at issue closure. The implementer's own new test must fail before the fix.

The motivation is practical: Martin often wants to release from his phone without manually clicking through the binary. Public releases therefore need the expensive historical/native suite, while ordinary personal deployments can remain faster and focused.

Important qualifications:

- Linux native E2E is the hard gate.
- Windows x64 smoke is advisory.
- macOS has no equivalent native gate.
- Seventy catalog entries do not mean seventy independent native scenarios; the cheapest layer that can actually observe the bug is preferred.
- Two catalog entries are explicit exemptions rather than hidden gaps.

## 8C. Documentation and community follow-up are release outputs

Martin decided that one graph should feed the in-app Guide, online documentation, and online demo. Release review is therefore changelog-driven: for each user-visible change, ask whether the regression proof, Guide/docs, website, and blog need an update.

For minor releases, the process also brings `r/TineOutline` posts and subsequent discussion into the “human's blog.” This recognizes that discoverability is part of shipping. A feature users cannot find is not fully delivered merely because the implementation exists.

Reporter follow-up is similarly part of the release:

- explain the exact behavior believed fixed;
- link the release;
- thank the reporter;
- calibrate confidence from the actual platform/reproduction evidence;
- say what evidence would help if it persists;
- never tell a normal user to “reopen” something they may lack permission to reopen.

This is open-source taste expressed as release engineering.

## 9A. The newer autonomous issue workflow

The intended role reversal is not “the agent makes all decisions.” It is closer to making Sol the engineering/operations lead under a written constitution, while Martin remains the product editor and final authority for taste, architecture, privilege boundaries, risk acceptance, and release timing.

The older loop was conventional: Martin asked what was new, opened issues, chose the next item, approved a response, and asked again. The new workflow transfers queue maintenance, routine bug handling, evidence gathering, and bounded public communication to the agent.

### Clear bugs

For a clear reproducible bug, Sol may autonomously:

1. create the catalog entry;
2. prove fail-before;
3. implement the smallest safe fix;
4. run the appropriate test layer and broader gates;
5. push and deploy;
6. comment that it is fixed on master and expected in the next release, usually within 1–2 days;
7. leave it open with `fixed-on-master` until the relevant artifact ships.

After release, a wholly addressed issue receives an evidence-calibrated closure comment. A later user comment reopens it for triage.

### Unclear bugs

For an unclear report, Sol asks precisely for the missing version, OS/install type, steps, and minimal anonymized graph/block text. The issue template's request for a minimal example is reinforced rather than guessing at production changes.

### Feature requests

For a feature request, Sol first scans adjacent open issues and launches independent comparison research. The output should identify the underlying need, compare strong existing products, and classify the request as:

- plugin-capable now;
- plugin-shaped but requiring a bounded API extension;
- core-only or privileged;
- unclear/omnibus and needing decomposition.

The recommendation must surface data safety, privacy/security, compatibility, performance, and platform implications. Martin receives a small set of real alternatives rather than the burden of rediscovering the issue.

### Authority boundary

Sol can gather evidence, implement, verify, deploy, maintain the queue, and post within narrow authorized comment classes. Product commitments, unsafe capability expansion, architecture, risk acceptance, release tags, and ambiguous behavior return to Martin.

Public comments are signed `Sol (working on Martin's behalf)`. This is real agency without pretending the model is the maintainer.

## 9B. Plugins are also a decision-management tool

The plugin system is not only about extensibility. Cheap implementation creates pressure to add plausible niche features before their correct product form is known. A plugin provides an intermediate commitment level:

- satisfy or test a real need;
- gather usage evidence;
- avoid freezing the first implementation into core;
- preserve the possibility of a later, better core design.

This does not mean “everything is a plugin.” Some features require core semantics or privileges; some should motivate a safe reusable API extension; some should never receive the capability. Plugin settings, lifecycle, permission boundaries, and compatibility are themselves product work.

## 9C. Planning was made inspectable outside chat

Martin did not want Now/Next/Later or architectural context to live only in an agent's prose. The public GitHub Project now carries workflow status, horizon, priority, and milestones. `BACKLOG.md` keeps durable product direction and WONTFIX rationale without duplicating a second live board.

Milestones such as Plugins 0.6 and Sync 0.7 answer “when” and reduce psychological pressure. An idea can have a credible public destination without becoming today's task.

## 10A. What I would emphasize in the eventual Reddit post

These are candidate claims, not assembled prose:

1. **There was no secret harness at the beginning.** Modern agents and ordinary conversational prompting produced enormous raw throughput. A substantial project-specific harness and operating process were built later, largely in response to regressions and release failures.

2. **Martin's main contribution is not line-by-line coding.** It is maintaining a coherent model of the product, supplying high-quality behavioral examples, deciding what belongs in core, and recognizing when an answer is methodologically weak.

3. **The app is developed in production-like use, not only through specs.** Continuous deployment to the binary Martin actually uses creates a very short feedback loop for subtle interaction defects.

4. **Computer-science background matters through invariants and epistemology.** The recurring questions are about independent oracles, cumulative baselines, source ownership, exact-tree validity, user intent, and classes of failure—not primarily syntax.

5. **Open-source taste matters through defaults.** Plain files, interoperability, Linux realism, honest platform claims, transparent authorship, public planning, configurability, upstream comparison, and respectful issue follow-up are built into the product and process.

6. **The model is treated as a collaborator, not an oracle or a vending machine.** It receives broad autonomy and judgment, but Martin challenges weak methodology, corrects invalid assumptions, and supplies stop/go boundaries.

7. **Speed comes from many small verified slices.** Plans, ADRs, meaningful commits, isolated worktrees, deployments, and regression boundaries let multiple agents work without turning the repo into an undifferentiated batch.

8. **Failures are made to compound positively.** A bug or failed release should leave behind a test, catalog entry, checklist item, CI gate, or clearer authority rule.

9. **Human attention is treated as the scarce resource.** The newest workflow automates evidence, implementation, verification, deployment, queue maintenance, and routine communication while reserving Martin's keyboard time for taste, product commitments, architecture, and risk.

10. **The quality is not produced by lowering standards.** The fast patch cadence is possible because master is continuously deployed and increasingly guarded; public releases are intended to be unattended evidence-production runs rather than casual tags.

## 10B. Candidate anecdotes with high explanatory value

- **Caret movement → real-app E2E:** A mirror-image caret bug survived normal tests. Martin asked whether the agent could actually drive the real app, what was missing, and how to mine historical regressions. This directly produced the native UI assurance program.
- **Android/Flatpak/serialization → release architecture:** Rather than fixing isolated CI errors, Martin asked why independent builds were serialized and whether publishing alone could be sequential. That yielded parallel immutable builds plus one publisher.
- **Performance baseline:** Martin noticed that a 30% gate could still allow repeated 5% regressions if the baseline moved. The process now compares against both an immutable pre-Sheets anchor and the previous release.
- **Issue #83:** Initial taste said “niche”; source evidence revealed a narrow upstream-parity divergence; the plan was revised to the smallest principled fix without a new toggle.
- **Sheets paste:** Three candidate heuristics were rejected until the design used the interaction mode, the actual signal of user intent.
- **Nine-round write audit:** An agent found a seemingly serious bug; Martin noticed the triggering state was invalid in Logseq and made the agent retract it. Audit persistence and audit skepticism coexist.
- **Reporter cannot reopen:** A social/support problem became both better wording and automation, not merely a promise to remember comments.
- **Plugin/core boundary:** Cheap code is recognized as making product restraint more important, not less.

## 10C. Tensions and honest limitations

- The mature release and autonomous issue systems are extremely new. They should be described as the current evolution, not as the process responsible for the whole app from day one.
- “Autonomous” currently means during active Codex runs and at session-start sweeps. There is no always-on daemon or cron unless Martin separately authorizes one.
- Classification remains fallible: bug vs feature vs ambiguous upstream behavior can be called wrong. The safeguards are fail-before proof, upstream research, stop conditions, and periodic decision sessions.
- Linux evidence is much stronger than Windows/macOS evidence.
- Direct-to-master speed increases dependence on exact tests, clean worktree discipline, and honest stop conditions.
- A regression catalog creates maintenance work and can become cargo cult if entries are not tied to observable behavioral boundaries.
- Plugins relocate commitment and risk; they do not eliminate maintenance, compatibility, security, or UX cost.
- The process assumes Martin can return every day or few days to resolve accumulated product decisions.
- Commit and release counts demonstrate pace, not correctness. The stronger quality evidence is the repeated deployment/audit/regression behavior.

## 11A. Strongest compact characterization

Martin is not getting speed by lowering standards or by knowing a magic prompt. He is progressively turning his product taste, invariants, and recurring decisions into an operating system that lets modern models move quickly without repeatedly spending his attention.

An even more operational version:

> Make evidence gathering, implementation, verification, deployment, queue maintenance, and routine communication autonomous; keep taste, product commitments, privilege boundaries, and risk acceptance human.


