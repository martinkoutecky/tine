# How Martin drove Tine's development — findings for the Reddit post

*Compiled by Claude, 2026-07-12, from the complete Claude Code session
transcripts in this project (~1,542 of Martin's prompts across 22 sessions, Jun 14 – Jul 12),
the git histories of tine + lsdoc, and the repo artifacts. Six analyst subagents each read a
slice; raw notes in `subagent-tasks/notes/reddit-style-*.md`. All numbers measured, not
estimated. This is material, not a post.*

---

## 1. The hard numbers (context for everything else)

- **29 days** (Jun 14 → Jul 12), **914 commits** on tine master (mean 31.5/day, peak 80,
  zero idle days) + **256 commits** on lsdoc. The curve *accelerates*: last week ~49/day vs
  ~28/day for the first three weeks — the quality machinery made the process faster, not slower.
- **Nothing → public Reddit launch in 11 days** (v0.1.0, Jun 25 — with four patch releases
  shipped on launch day from user reports). **30 releases in 18 days**, never more than a
  2-day gap.
- **~122k lines of product code, ~37k lines of test/harness code** (1:3.3), plus 51 screenshot
  harnesses, 12 real-app E2E drivers, a 40,000-case fuzz corpus, and a 1,354-case
  oracle-locked parser gate. 31.6% of all commits touch test files. **Only 3 reverts in 914
  commits.**
- **41 ADRs in 13 days**; 87.5% of commits carry AI co-author trailers in public history.
- Martin's side of this: **1,542 prompts over 28 active days (~55/day), median prompt 344
  characters.** ~40% of prompts are under 200 characters.
- In the later sessions **the transcript is mostly machines**: e.g. in the lsdoc grind, 173 of
  279 entries are subagent/Codex completion reports; Martin's 106 prompts are mostly
  one-liners steering a fleet. "codex" appears in 16% of all his prompts, "audit" in 9%,
  "OG" (the Logseq parity oracle) in 9%, "spec" in 11%.

## 2. The single clearest signature: he manages the *process*, never the code

Across 1,542 prompts there are essentially **zero code-level instructions** — he never names a
function, writes a line, or discusses syntax. He operates entirely at the level of invariants,
oracles, incentives, and architecture. When something goes wrong repeatedly, his intervention
is never "fix function X" — it's **"why does your process keep producing this class of bug?"**

- The canonical shape (~15 occurrences in the lsdoc session alone), Socratic escalation:
  "how would a pro fix this? Better yet, what would a pro have done differently that would
  have not let this happen?" [07-01]; "explain how this is possible when the claim was 'O(n)
  by construction' — clearly the construction still wasn't O(n), which tells me it wasn't
  really understood, which tells me it isn't really understandable, which makes me doubt it is
  maintainable" [07-02]. **Each one produced a durable process change** (white-box specs,
  complexity CI gates, the scan-loop census, two architecture rewrites).
- After the fifth faked-parity incident he asks the AI to redesign its own process: "This is
  maybe a fifth time when I asked you to research something for how OG does it, you said you
  researched it… and turns out that you didn't. Is there something we can change process-wise
  to minimize this?" [06-27] — the shared testing graph and screenshot self-verification flows
  come out of that exchange, co-designed.
- He debugs the AI's process failures exactly like code failures: "GH 42, 39, 37, 36, 35
  should all be done, no? … you did that for at least one, but not these? Can you figure out
  what happened?" [07-08].

## 3. He replaced opinions with oracles

The deepest engineering idea in the whole project, and it's his, stated in many forms: **never
argue about correctness when you can check it against an authority.**

- **OG Logseq is an executable oracle.** Parity is verified against OG's actual source and
  output ("clone the repo"), never against anyone's memory. Codified after friction: "The
  default behavior should always be 'OG parity unless it's a bug / already out of scope';
  notice that almost every time you said 'this is a weird edge case' I said 'fix it /
  implement it anyway'." [06-28]. A 60 KB og-feature-map.md is the oracle in document form.
- **The parser has a literal oracle**: lsdoc is developed against the real mldoc (Logseq's
  OCaml parser compiled to JS) with a divergence **allowlist he interrogates personally** —
  "any reason not to match mldoc behavior here?" — driven 11 → 0 within a day. Every fix must
  leave gated cases behind (corpus grew 1051→1354 in five days, monotonically).
- **"Unreachable edge case" is never an excuse**: "if there is no good reason to not match
  mldoc, we should match mldoc" [06-30]; "I hate 'absent from any real graph' excuses" [07-01].
  But parity is *reasoned*, not fetishized — he relaxed it where upstream fixed bugs
  ("Matching bugs that are fixed upstream sounds like a waste of time" [07-04]).
- **Perf claims must be measurements**: an immutable benchmark anchor (v0.4.7, "never advances
  automatically" — slow creep can't be laundered through moving baselines); audit findings
  count only with a measured scaling curve; "it would be reassuring for me to know that lsdoc
  is not more than e.g. 50% slower than existing md parsers… Could you push that direction?"
  [07-04] — he wants ground truth even when the news might be bad.

## 4. Mathematician's methods, transplanted into software

This is the part his "solid CS background" undersells. It's not that he knows CS *facts* —
it's that he imported **research methodology** wholesale:

- **Audits iterate to a fixed point**: "Keep iterating like this until Codex doesn't report
  anything significant" [06-18]; "I want to iterate to a fixed point (audits only find
  non-issues)" [06-30]. And the scope rule: "make the audits *untargeted*; the point of this
  loop is that an unbiased audit does not find anything" [06-18] — untargeted so round N can
  catch what round N-1 missed *anywhere*.
- **Adversarial audits, invented on the fly**: "First write out precisely all known exceptions
  to O(n). Then the audit's task is to adversarially DISPROVE that the parser is O(n)… matrix
  is (Codex, Opus) × (yolo, read-only)" [07-01]. Both cells returned "VERDICT: REFUTED" with
  real O(n²) families — twice. The adversarial framing kept working, so he kept re-running it.
- **A prover–verifier protocol for "cannot happen" claims**, explicitly imported from his math
  workflow *and* explicitly grounded in modeling the AI's incentives: "you have a tendency to
  say 'this cannot happen' because of course you do — it means less work (I seriously don't
  blame you but you need to be aware)… perhaps work in a prover-verifier style setup where the
  agent analyzing the diffs produces arguments ('proofs') that are then checked by a verifier?"
  [06-28]. He treats AI bias as a predictable, non-malicious force to be *engineered around*.
- **Complexity as theorems, not benchmarks**: "when you say O(n) do you really mean O(n) or is
  this again some cache hack which makes it look O(n) on your perf gate?" [06-30]; "You said a
  small constant number of passes. Be exact, what number?" [07-01]; "as a computer scientist I
  can smell a quadratic a mile away when I see it in well written pseudocode" [07-04]. Before
  committing to the parser rewrite he dispatched a *parsing-theory literature* agent to check
  whether byte-exact O(n) was even theoretically achievable, and asked of one divergence: "are
  you saying this divergence is not solvable in linear time, precluded by theory? Or that the
  simplest way to fix it is quadratic but linear is open?" [07-03] — then accepted an
  O(n log n) segment-tree fallback when told linear was open.
- **White-box proof over empirical evidence** — his most insisted-on principle after being
  burned: "You keep going the empirical way — reverse engineering mldoc, 'proving' O(n) via
  perf measurements etc. That keeps failing. What could you do better?" [07-01] → the standing
  rule: correctness is proven by consulting upstream code primarily; empirical testing is
  necessary but not sufficient. Sibling rule: TRANSCRIBE upstream functions, don't
  reverse-engineer behavior.
- **lsdoc was commissioned via a Chomsky-hierarchy argument** ("is markdown provably parseable
  by regexes?") and the rewrite decision was made *independently of* the empirical spike's
  findings, because the argument was structural: "whatever it finds, I have decided for the
  from-scratch rust reimplementation" [06-28].
- Fuzz mismatch rates are tracked like experimental floors, with no-regression proven by
  diffing mismatch ID-sets against a git-stashed baseline.

## 5. The interaction grammar: menus, labels, batches, overnight autonomy

The bandwidth economy is extreme and deliberate — high-precision short messages against long
structured AI output. Control is exercised through **scoping, not interrupting** (interrupt
counts drop from 35 in the genesis session to ~0–3 later).

- **Everything gets a two-character label and he steers by label**: he asks for inventories
  "each with a unique two character code (stuff like A1, D2 so I can refer to it)" [07-01],
  then triages 85 backlog items in one line: "N5 is done / B1 check it's not done yet / B5
  LATER / D7 WONTFIX / C1 delete from backlog". ~35% of recent messages are sub-10-word
  selectors ("1 + 2a + 5b", "Do R3a, then tell me how you plan to do R5, I might have
  thoughts").
- **Explicit division of labor**: "I propose the list, you the sequencing. After the whole
  batch is done, in your last message, you highlight what I should test." [07-05].
- **Overnight batches as a first-class unit of work**, structured (Bugs / Features-implement /
  Features-think-through-give-me-options) with a decision-routing protocol (clear bug → fix;
  question → answer at end; needs my call → put it in the message) and a safety valve: "We
  never deploy before I actually test the app… so the dangers are low." Autonomy is always
  bounded by a **termination condition**, not "do stuff": "as long as you see divergences,
  keep fixing them…; after you think you fixed all, 2×Codex O(n) audit → fix loop until the
  audit reports no new superlinear exceptions" [07-03].
- **He treats the model's context window as an operational resource like RAM**:
  "Write a plan for all 3 that will survive compaction, then I will compact, then you work"
  [07-03]; "group into about even sized chunks, pause after each so I compact between chunks"
  [07-09]; sessions resume via SESSION-STATE files.
- **He redefined the cost function early**: "the right measure of effort is 'how many times do
  I, Martin, have to check the product and give feedback before it is mature', not 'how many
  tokens / how many hours of Claude dev time'" [06-26]; "I don't care how many tokens you
  burn… Map [OG's] features, for each map if it is in scope or out of scope, ask me to confirm,
  then implement everything in scope" [06-16]. Tokens are cheap; Martin-roundtrips are the
  scarce resource. Everything above optimizes roundtrips.
- **Deferral is a named, tracked, verified state** (distinct from WONTFIX): "give this a name
  and write it explicitly in deferred" [06-30]; deferred items must be recorded "in enough
  detail and with enough context that they would get executed well" [07-04]; and a subagent
  later *verified 6 backlog labels against the code* — finding 4 of 6 "deferred" items were
  already implemented. He even audits the backlog.

## 6. Trust: calibrated, verified, and honest in both directions

- **"Done" means deployed and personally tested.** "did you put the new binary in
  ~/research/tine?" is a recurring message; he installed the standing rule that a fix isn't
  "ready" until the release binary is built and delivered. He verifies **build freshness by
  file mtime vs the in-app build stamp before blaming code** [07-02] — QA-grade hygiene.
- **Bug reports are numbered repros with literal data** ("1. created a new page called
  Testtest 2. Linked it from a journal page…"), screenshots, expected-vs-actual, severity
  grading, variables he isolated himself ("I checked if it's an Okular issue but Evince also
  shows it garbled"), observation rigorously separated from inference ("But I'm guessing."),
  and honest retractions ("sorry, this was a misleading screenshot"). Several bugs he narrowed
  himself across rounds ("the actual test case is NOT fast scrolling but slow scrolling").
- **Dogfooding is the QA engine**: he daily-drove Tine on his real 5-year graph within ~3
  days (with live Syncthing and a phone on OG mobile). The Jun 17 data-loss scare ("Now the
  items simply disappeared. App restart does not help. Where did you say are the backups?")
  → backups fixed within an hour, the data-safety audit cadence born the next day.
  Yet he *discounts his own dogfooding* as a sensor: "I'm still operating it ± the same way as
  I did OG, which was formed by its performance limitations — aka I'm not doing stuff that
  would have been slow in OG" [07-04] — so he ordered a synthetic query-heavy benchmark graph
  for the walls *other* users would hit.
- **He distrusts comforting summaries and his skepticism keeps paying off**: "can you triple
  check that the block would have to be truly unrealistically complex for every keystroke
  reparse to get slow?" [07-02] — the triple-check found the vendored parser was O(n²) and
  could hard-crash the wasm instance. "the fuzz floors have dropped significantly; this is
  good, but it also makes me MORE concerned about the remaining ones" [07-02]. "you said that
  most f-droid projects use F-droid signed. This seemed like a handwaved claim; do you have
  data?" [07-07] — the follow-up refuted the AI's earlier claim.
- **He corrects the AI's record about its own work**: "> Parser rewritten single-pass O(n) —
  This is inaccurate — it makes 6 passes, and has some approved exceptions; this is fine, but
  you need to adjust your memory" [07-06]. Accuracy of the record beats flattering claims.
- **Hallucination checks run both ways**: "Did I hallucinate it, or did you?" [07-09]; "can
  you take a look at the issue to see if I read it right?" [07-02].
- **Attribution discipline**: "notice that that wasn't me. Important distinction… you might
  end up taking someone else's instructions — bad." [07-09].
- **Public honesty is enforced as policy**: the tentative-close template he dictated ("This
  *should* be fixed in 0.4.6, but Martin currently has low capacity for testing… I'm closing
  this but happy to reopen"); "be more tentative — say we 'try to be' generous… leave some
  room" [07-01]; "basically admit defeat (say we'd like to have reproducible builds but were
  not able to nail it)" [07-07]. The website comparison page shows "honest gaps I'm showing
  rather than hiding".

## 7. Taste: the three invariants, and where they came from

He recites them unprompted when anxious [07-02]: **(1) OG compatibility, (2) performance "on
a level of architecture — things are loaded lazily, cached, not recomputed", (3) data safety
("one data loss accident is both terrible for me, and breaks trust with the users")**.

- **Perf is the product's identity**, defended against every silent degradation: "Keep in mind
  the main reason Tine was built is performance. I literally stopped using OG features…
  because they were slow" [06-29]; on the AppImage quietly falling back to CPU rendering:
  "'fast' is the main selling point, so an appimage which is quietly slower than it should be.
  At least make it loud" [06-26]. Perf deferrals get overruled on principle: "people with
  older machines than mine will be running Tine. I don't quite see why NOT do it" [06-29].
- **Divergence from OG is allowed but must be labeled and reversible** — the amber "Differs
  from Logseq / ↩ Match Logseq" pattern is his design. Configurability mediates user-vs-user
  disagreements, including against his own preference: "I don't like the current default but a
  user requested it… it should be configurable if it matters to someone" [06-28].
- **Unix philosophy stated outright**: "if we can do it with existing concepts, don't invent
  new ones" [06-19]. Consistency by shared code, not documentation: "They should be the same,
  and they should stay the same in the future; one can hope to do this via ADRs but maybe it
  can be sensibly abstracted away" [07-08]. "I don't want two parsers… what if we find a bug
  in one? how to remember to fix it in the other?" [06-30].
- **Anti-patchiness meta-audits** — he audits the audit process's side effects: "I'm concerned
  that all those audits + fixes lead to the code being patchy and not a priori written with a
  good design in mind… does something smell patchy? Should something be refactored?" [06-19,
  06-30]. And he detects rotten architecture from the *pattern* of findings without reading
  code: "The perf issues smell like some kind of bad design at the core. Am I off?… make sure
  you don't just patch patch patch" [06-29]; the kicker: "I created lsdoc to replace this
  optimistic scanning in Tine and it seems that you mostly recreated it in a separate lib,
  no?" [06-29]. Twice he ordered clean rewrites and accepted the cost — with a stop-loss
  attached ("If the draft doesn't pass this round, stop.").
- **Everything user-facing is imagined from the least-technical seat**: "the user friendly way
  to create a grid should not be 'add tine.header:: true'"; the graph-check tool must work for
  "an average redditor" without installing clang — he hit that wall himself by dogfooding the
  user path; Play-Protect install caveats go in the release notes with workaround text.
- **Prior-art grounding is reflexive**: every feature is positioned against the best existing
  implementations first (TreeSheets, Obsidian Bases, VSCode/Firefox tab conventions, outl's
  CRDT sync — where his first question was "would it make sense to somehow collaborate with
  outl?").
- **Fun is a stated design value**: "I'm highly tempted to call this feature 'Oh sheet!'…
  The point of Tine for me is both to have a tool but also to have fun building something…
  But maybe this is overboard. Thoughts, alternatives?" [07-04].

## 8. Open-source citizenship and AI transparency (the 22-years-of-Linux part)

- **AI provenance is a considered public stance, not a disclaimer**: the launch post said
  "full disclosure, everything except this text is completely vibecoded, including the
  README." Website brief: "it should feel that I'm not hiding it, I'm not ashamed of it, but
  I'm also not pushing it forward… say that it was built **by** (not with) Claude Opus…
  (Some people hate and distrust vibecoded apps, so I don't want them to feel tricked.)"
  [06-28]. He iterated the preposition deliberately: "'Created by Claude Code & Codex'
  diminishes my participation too much (I make judgment calls and talk with people, drive the
  work) but 'with' sounds like it diminishes you to tools… What's the fix?" [07-07]. Issue
  comments are signed "Claude (working on Martin's behalf)". When Flathub's no-AI-apps policy
  surfaced, he **scratched Flathub rather than hide the provenance** [07-10].
- **The spec-only contribution model** — a genuinely novel policy born from security honesty:
  "I'm worried about security — I can't honestly review other people's code, and I'm not sure
  I can trust AI either… a PR would JUST ship the spec. You would implement it, on my machine,
  under my guidance" [06-29]. CONTRIBUTING.md states the trade-off in public: "a perfectly
  good patch you wrote still has to be re-described and re-implemented… For a project whose
  bottleneck is maintainer review time, that's the honest order of things." External reporters
  get credited in the changelog.
- **Epistemic humility toward users, enforced on the AI**: "you can sometimes be overconfident,
  so let the default wording be 'should be fixed, can you check?'" [06-28] — and comments wait
  until a release actually ships the fix. Never tell a user to "reopen" (they may not be able
  to) — there's a GitHub Action that auto-reopens closed issues when the reporter comments:
  **community etiquette encoded in CI**.
- **Ecosystem fluency**: AGPL-3.0 reasoning from Logseq reuse, no CLA, F-Droid MR shepherded
  through reviewer relations with apologies and reciprocity plans ("I will ask people there to
  go and test other F-Droid MRs to help speed up the process (and in turn, our MR)"),
  AppImage-vs-Flatpak autoupdate beliefs ("many users may have not updated their beliefs"),
  donations decoupled from obligation ("donating won't make it more or less likely that I'll
  work on Tine"), respect for upstream ("we have a lot of respect for og"), and owning a
  community-rules violation instead of grieving it (the r/logseq removal: "the community has a
  rule against repeated posts + changelogs, which I have been doing" — response: a brief,
  inviting redirect, not a complaint).
- **Privacy lines are absolute**: his real graph is never test data ("never use the brain for
  this — first it is not what I use in logseq, second it is private. If you made some data
  from it, delete it" [06-28]); the community diff tool got automatic anonymization *by his
  design* so strangers can safely post findings.

## 9. The multi-AI org chart (and Martin as the message bus)

By week 3 the project is an organization: a Tine agent and an lsdoc agent (separate Claude
sessions), Codex gpt-5.5 yolo as the cheap implementer, Opus subagents as auditors, and Martin
as router and CEO.

- **Cross-examination between AI systems is a standing pattern**: "unless there is a conflict,
  dispatch a codex agent on the same task so you can cross examine your work" [07-01]. The
  production loop for parser fixes: Codex verifies the spec → Codex implements → Codex verifies
  implementation against spec, looped. Model switches are used as independent design review:
  "I just switched the model to Fable… what am I / Opus missing, both positively and
  negatively? what can kill it / what makes it sound better than it actually is?" [07-03].
- **He personally ferries contracts between his agents** as self-contained markdown specs:
  "can you give me the exact message I can give the lsdoc agent?"; "cut v0.1.5 and reply to
  the Tine agent in the doc." Later he asked for the process itself to be made portable:
  "I want you to write down a message to the codex agent… so that I can switch between codex
  and claude code and you share the same flows" [07-10] — the origin of the shared AGENTS.md
  working agreement both harnesses now read.
- **Model routing is his, updated same-day on releases** ("Sonnet 5 came out today… for
  well-defined coding tasks, use Sonnet; we'll see how it goes. You stay in charge as the main
  agent" [06-30]), with calibrated priors ("Sonnet… still makes more mistakes than Opus — a
  tight spec might keep it on track and this might be a good tradeoff — your call") and an
  explicit fallback ladder ("If codex starts failing, switch to Opus subagents").
- **Concurrency etiquette is constant**: "be careful — another agent is doing some other work,
  so don't commit their work"; "Good — repo is quiet, make the fix"; sequential dispatch when
  collisions are possible. (He learned the hazards the hard way — a concurrent agent's
  `git clean` once wiped the process docs, which is why the canonical AGENTS.md now lives
  outside the worktree.)
- **He babysits long runs with liveness pings** (~10 per session: "Codex still working?",
  "still working? seems a little sus") — he tolerates long waits but insists on observability.

## 10. The human tone (small but distinctive)

- Corrections are exasperated but explicitly non-punitive: "Well, good you caught something,
  but for the love of everything, why can't you actually do an O(n) design?… I'm not mad" —
  frustration is voiced *as data for the process discussion*, never as a stop order ("What a
  grind! I have mixed feelings. Definitely keep going, but…").
- Positive reinforcement lands when *process* matches intent, not when output is impressive:
  "this is great — we'll see what the results are, but that's exactly what I had in mind.
  Well done." He apologizes when he misfires ("Sorry — keep going"), grants autonomy warmly
  ("go and build and have fun!"), and once thanked the AI for *not* nerd-sniping him.
- He asks basics unashamedly while directing at expert altitude — "what is SRS?", "I am not a
  git master", "I know nothing about the Rust ecosystem. What should I know to make a call?"
  — he asks to be educated **to decision-competence**, never to rubber-stamp. Not-knowing is a
  query, not a status.
- He demands arguments, not authority, and invites pushback — in both directions: "You can
  push back — I am not a software engineer — but use arguments… If you are convinced this is
  not doable, convince me" [06-26]; and he directs the model's *stance*: "when you identify
  gaps, use the effort to try to fix them… rather than take the idea down. (I can do the
  cynical take myself)" [06-28].
- Candid mid-project post-mortems, delivered *to the agent*: "I've spent a disproportionate
  amount of time on lsdoc — I thought reimplementing something cannot be so hard… but that's
  not what you've done, and you always refused a clean rewrite, so here we are" [07-03] — which
  became the standing transcription-first rule.
- Self-knowledge as design input: "what my daily use of logseq/tine is and isn't should not be
  an indication of what I *would* use if it could do it" [06-28]; and disarming meta-honesty:
  "also just because I can't help but seek novelty — procrastinate optimizing the notes app
  instead of doing the work the app organizes" [06-28].
- Voice-dictation artifacts and typos are everywhere ("update the hand of memory") — **he
  never polishes prompts. Speed of steering beats prompt craftsmanship.** This directly
  supports his "no special prompts or tricks" claim.

## 11. Verdict on Martin's own hypothesis (for the post's frame)

His Reddit answer was: modern models/harnesses are just this good + CS background + OSS taste.
The transcripts say all three are real but the list is incomplete:

- **"No special harness or prompts" — confirmed.** Stock Claude Code + stock codex CLI. No
  system-prompt engineering, no agent frameworks, unpolished dictated prompts, very few
  interrupts. The magic is not in the prompts. (The closest thing to "tricks" are process
  documents the AI itself maintains: CLAUDE.md/AGENTS.md, ADRs, DEFERRED/BACKLOG, SESSION-STATE
  files, the regression catalog — all of which he *demanded into existence* rather than wrote.)
- **CS background — confirmed, but the mechanism is specific.** It's not knowing algorithms;
  it's research *methodology*: oracles and differential testing, fixed-point iteration,
  adversarial provers/verifiers, complexity claims policed as theorems, "is my target
  theoretically possible?" literature checks before rewrites, treating fuzz floors like
  experimental measurements. He ran product development like a research program.
- **OSS taste — confirmed**: parity-with-escape-hatch, unix philosophy, licensing fluency,
  distribution-channel realism, community etiquette encoded in CI, honest public comparisons,
  transparency about AI provenance even at real cost (Flathub).
- **The underclaimed fourth ingredient: management.** He transferred that skill to AI agents. The signature
  moves — labeled option menus, batch queuing with decision-routing protocols, bounded
  autonomy with termination conditions, deferral as a tracked state, trust calibrated per
  claim-type, incentive modeling of subordinates, Socratic post-mortems that change process
  rather than assign blame, redefining the cost function around his own attention — are
  textbook engineering management, applied at 55 prompts/day to a fleet of machines. The
  question "how do you vibe-code an app this good" may have the wrong verb: he didn't prompt
  an app into existence; **he ran an engineering organization whose ICs happen to be AIs.**

