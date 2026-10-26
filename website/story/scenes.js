// The story of Tine as scenes. A scene is a small set piece at one place in time: props on stage, then
// steps. Each step is one line in the dialogue box (who + text) and optionally an action on stage (act)
// and lasting effects (gain). Each world is an arc that makes sense on its own: a wish or a problem, what
// happens, and how it ends.
//
// Step fields: who ('narrator' | 'martin' | 'system'), text, mode ('ask' | 'aha'), act ('<verb>:<propId>'
//   or '<verb>'), gain ({ party: [who], lv, tower, calm }), at (exact date of a quote), src (public
//   source line), more (card text), dur (seconds, for steps without text).
// Martin's lines are verbatim from his chats ("..." marks a cut); all were checked against the
// transcripts. Claude and Codex never get invented quotes: what they do is shown as PARTY lines.
// Regulars are not named in the story until they agree; the credits use public GitHub handles.
//
// Prop kinds: snail, qblock {pop}, bug {label}, flags {flags, big}, machine {built, shaky}, disk {label},
//   lever, wall {label}, pipe, box {label} (opens on 'pop'), visitor {name, col}, panes {label},
//   lines {labels} (paths fork on 'fork'), bots {n, keep} ('crash' leaves keep of them),
//   sign {label, later} ('defer' turns it to the later label).
//
// Adding a release: append a world and its scenes, move `finale` to the new release, and turn the old
// finale scene into an ordinary ending of its world.
window.TINE_SCENES = {
  worlds: [
    { id: 'w1', n: 1, title: 'Slow Logseq, fast Tine', sub: 'From an idea to v0.1.0 in ten days',
      sky: ['#2b4e7e', '#f2b880'], mood: 'day', drone: 0 },
    { id: 'w2', n: 2, title: 'Launch week', sub: 'Strangers arrive, with wishes',
      sky: ['#2f7fd0', '#a8dcf0'], mood: 'day', drone: 0 },
    { id: 'w3', n: 3, title: 'Feature surge', sub: 'New hands, new features, a plan to slow down',
      sky: ['#3b6fb6', '#f6d79a'], mood: 'day', drone: 0 },
    { id: 'w4', n: 4, title: 'The dark ages', sub: 'A database inside a notes app',
      sky: ['#221d36', '#5e4b66'], mood: 'dark', drone: 0.8 },
    { id: 'w5', n: 5, title: 'The reckoning', sub: 'Pull it out. Then start again.',
      sky: ['#170f1c', '#6e2c2a'], mood: 'storm', drone: 0.5, calm: ['#1d2b4a', '#c98f6a'] },
    { id: 'w6', n: 6, title: 'Proof-minded rebirth', sub: 'How can we be sure this time?',
      sky: ['#0f1a36', '#3d6496'], mood: 'night', drone: 0 },
    { id: 'w7', n: 7, title: 'Calm and quick', sub: 'Keep building, without shaking what people rely on',
      sky: ['#2d86c4', '#cdeee6'], mood: 'day', drone: 0 },
  ],
  // Model changes show on the agents' name tags as the date passes them (no dialogue line).
  models: [
    { who: 'claude', name: 'OPUS 4.8', at: '2026-06-14T12:00' },
    { who: 'codex', name: 'GPT-5.5', at: '2026-06-14T12:00' },
    { who: 'codex', name: 'GPT-5.6 SOL', at: '2026-07-10T10:30' },
    { who: 'claude', name: 'OPUS 5', at: '2026-07-27T12:00' },
    { who: 'claude', name: 'OPUS 5.5', at: '2026-09-22T12:00' },
    { who: 'codex', name: 'GPT-6 SOL', at: '2026-09-24T12:00' },
    { who: 'codex', name: 'GPT-6.1 SOL', at: '2026-09-29T18:00' },
  ],
  // The release the story ends on (at: the GitHub release's publish time; pending: true marks a placeholder
  // date before the cut).
  finale: {
    tag: 'v0.8.0', at: '2026-10-26T11:08:29', name: 'Calm and quick',
    // What Martin learned (R3-learning.md chapters), one real question each
    syllabus: [
      { chapter: 'How one builds an app at all', q: 'What is an ARD folder?' },
      { chapter: 'What a pro would do', q: 'how would a pro (a dev who does parser and compiler design for a living) design it?' },
      { chapter: 'How computers save and sync files', q: 'What is "changes a durability guarantee"?' },
      { chapter: 'What good architecture means', q: 'What should be the invariants?' },
      { chapter: 'Proving it', q: 'What is TLC?' },
      { chapter: 'The layers below, and release craft', q: 'what is semver and is this a big deal?' },
    ],
    trophies: [
      { label: 'Calm', text: 'Quieter query blocks with one menu, a Journal sections page for what today shows, a Soft theme refined by a tester.' },
      { label: 'Your words are safe', text: 'One background saver per graph: every edit saved as you type, from any window.' },
      { label: 'Speaks Chinese', text: 'The interface in Simplified Chinese, and Pinyin search for page titles.' },
      { label: 'Lives in your phone', text: 'Share sheets on Android and iOS, a Quick Settings tile, Spotlight, Shortcuts.' },
      { label: 'Desktop grows up', text: 'Pages in their own windows, a tray icon, and Homebrew on macOS.' },
      { label: 'More Logseq', text: 'Advanced queries translate, journal queries, page icons, colored text.' },
      { label: 'Tools', text: 'A command line, find and replace, an All pages table for tens of thousands of pages.' },
    ],
    closing: 'Not vibe coding: agentic engineering. Taking a project past v0.1 to something people rely on takes care, tests and attention to detail, and you can learn all of it on the way.',
  },
  scenes: [
    // ── World 1: an itch, a party, a first release ─────────────────────────────
    { world: 'w1', at: '2026-06-14T12:16', props: [{ id: 'lag', kind: 'snail', x: 470, label: 'THE LAG' }], steps: [
      { who: 'narrator', text: 'June 2026. Martin keeps his notes in Logseq, an outliner he loves.' },
      { who: 'martin', text: 'the UI is SLOW', act: 'tap', src: "Martin's chat with Claude, 2026-06-14" },
    ] },
    { world: 'w1', at: '2026-06-14T12:16:22', props: [{ id: 'q1', kind: 'qblock', x: 174, pop: 'RUST + SOLIDJS + TAURI' }], steps: [
      { who: 'narrator', text: 'He is a computer scientist. Building apps is new to him, so he asks Claude.' },
      { who: 'system', text: 'CLAUDE joins the party, and CODEX as the reviewer.', gain: { party: ['claude', 'codex'] } },
      { who: 'martin', mode: 'ask', act: 'hit:q1', src: "Martin's chat with Claude, 2026-06-14",
        text: "Plan it out - what stack do you want to use? ... I really don't understand the current landscape and software engineering in general" },
    ] },
    { world: 'w1', at: '2026-06-17T19:19', props: [{ id: 'bug1', kind: 'bug', x: 450, label: 'VANISHING TASKS' }], steps: [
      { who: 'martin', act: 'scare', src: "Martin's chat with Claude, 2026-06-17",
        text: 'Well. Now the items simply disappeared. App restart does not help. Where did you say are the backups?' },
      { who: 'system', act: 'fight:bug1', text: 'CODEX learns AUDIT LOOP: check safety and speed until nothing big is left.' },
    ] },
    { world: 'w1', at: '2026-06-23T16:56', props: [{ id: 'lag2', kind: 'snail', x: 300, label: 'THE LAG' }], steps: [
      { who: 'martin', act: 'overtake:lag2', text: "I'm so far quite happy daily driving Tine.", src: "Martin's chat with Claude, 2026-06-23" },
    ] },
    { world: 'w1', at: '2026-06-24T23:37', props: [{ id: 'f1', kind: 'flags', x: 300, flags: ['LINUX', 'WINDOWS', 'MACOS'], big: '0.1.0' }], steps: [
      { who: 'narrator', act: 'raise:f1', text: 'June 24: v0.1.0 for Linux, Windows and macOS, open source on GitHub.' },
      { who: 'martin', act: 'rain', src: "Martin's chat with Claude, 2026-06-24",
        text: 'I announced Tine on reddit - want to be done with this phase and move on to using + polishing.' },
    ] },

    // ── World 2: strangers arrive with wishes; the team delivers ─────────────
    { world: 'w2', at: '2026-06-25T22:39', props: [{ id: 'org', kind: 'box', x: 430, label: 'ORG MODE' }], steps: [
      { who: 'narrator', text: 'Strangers download Tine. One asks for Org mode, the other plain-text format. It is bedtime.' },
      { who: 'martin', act: 'night', src: "Martin's chat with Claude, 2026-06-25",
        text: "work autonomously, don't stop until you have working org-mode" },
      { who: 'narrator', act: 'pop:org', at: '2026-06-26T05:21', text: 'By morning, Org mode works.' },
    ] },
    { world: 'w2', at: '2026-06-29T15:49:36', props: [{ id: 'q3', kind: 'qblock', x: 174, pop: 'ADR' }], steps: [
      { who: 'martin', mode: 'ask', act: 'hit:q3', src: "Martin's chat with Claude, 2026-06-29",
        text: 'What is an ARD folder? Sounds like a good idea to doc architectural decisions.' },
      { who: 'narrator', text: 'An ADR: a short note on why the code is the way it is.' },
    ] },
    { world: 'w2', at: '2026-07-06T12:38', props: [{ id: 'f2', kind: 'flags', x: 260, flags: ['ANDROID'], big: '0.4.0' }], steps: [
      { who: 'narrator', act: 'raise:f2', text: 'Twelve days after launch, Tine runs on Android.' },
      { who: 'martin', text: "it's awesome. Wow.", src: "Martin's chat with Claude, 2026-07-06" },
    ] },

    // ── World 3: power, joy, and a plan to slow down ─────────────────────────
    { world: 'w3', at: '2026-07-07T12:00', props: [{ id: 'fable', kind: 'visitor', x: 360, name: 'FABLE', col: '#a78bfa' }, { id: 'sheets', kind: 'box', x: 450, label: 'SHEETS' }], steps: [
      { who: 'system', act: 'pop:sheets', text: 'FABLE, a new Claude model, visits and builds SHEETS: a spreadsheet inside a block.' },
    ] },
    { world: 'w3', at: '2026-07-08T19:23', props: [{ id: 'panes', kind: 'panes', x: 420, label: 'SPLIT PANES' }], steps: [
      { who: 'martin', act: 'split:panes', src: "Martin's chat with Claude, 2026-07-08",
        text: "I'm getting excited, I think people will like this", more: 'Split panes: two pages side by side, and as many as you like.' },
    ] },
    { world: 'w3', at: '2026-07-10T09:45:50', props: [], steps: [
      { who: 'narrator', text: 'Eleven days later:' },
      { who: 'martin', mode: 'aha', gain: { lv: 'ADRs' }, text: 'Write it up as ADR, Design 1.', src: "Martin's chat with Claude, 2026-07-10" },
    ] },
    { world: 'w3', at: '2026-07-19T21:32', props: [], steps: [
      { who: 'martin', text: 'the plan is to stay with 0.6.x for a while ... No rush to 0.7.', src: "Martin's chat with Claude, 2026-07-19" },
    ] },
    { world: 'w3', at: '2026-07-22T15:40', props: [{ id: 'f3', kind: 'flags', x: 260, flags: [], big: '0.6.5' }], steps: [
      { who: 'narrator', act: 'raise:f3', text: 'July 22: v0.6.5, a calm release. Remember it.' },
    ] },

    // ── World 4: a big idea grows too big ────────────────────────────────────
    { world: 'w4', at: '2026-07-22T19:00', props: [{ id: 'm', kind: 'machine', x: 420 }], steps: [
      { who: 'narrator', text: 'The next wish: sync between devices, guaranteed to agree.' },
      { who: 'martin', act: 'build:m', src: "Martin's chat with Claude, 2026-07-22",
        text: "I thought I 'just' want sync. But ... I am building a lot of db-like infra" },
      { who: 'narrator', act: 'build:m', text: 'Managed storage: a database under the notes, with the Markdown files rebuilt from it.' },
    ] },
    { world: 'w4', at: '2026-07-29T21:19', props: [{ id: 'm2', kind: 'machine', x: 420, built: true }], steps: [
      { who: 'martin', gain: { tower: 250000 }, text: 'what strikes me is the brutal size: 250kLOC?', src: "Martin's chat with Claude, 2026-07-29",
        more: 'Managed storage was meant to take about a day. A week later the Rust code had grown to four times its size.' },
    ] },
    { world: 'w4', at: '2026-08-12T18:25', props: [{ id: 'm3', kind: 'machine', x: 420, built: true, shaky: true }], steps: [
      { who: 'martin', act: 'wobble', src: "Martin's chat with Claude, 2026-08-12",
        text: 'I feel tempted to cut it out and start over ... It feels very shaky' },
    ] },
    { world: 'w4', at: '2026-09-01T10:18', props: [{ id: 'q2', kind: 'qblock', x: 174, pop: 'INVARIANTS' }], steps: [
      { who: 'martin', mode: 'ask', act: 'hit:q2', text: 'What should be the invariants?', src: "Martin's chat with Claude, 2026-09-01" },
      { who: 'narrator', text: 'Invariants: rules that must hold, whatever the code does.' },
    ] },
    { world: 'w4', at: '2026-09-14T08:30', props: [{ id: 'disk', kind: 'disk', x: 450, label: 'ONE EDIT' }], steps: [
      { who: 'narrator', act: 'swell:disk', text: 'September 14. One edit on a big journal page writes 100 to 200 KB, kept forever.' },
      { who: 'martin', mode: 'aha', gain: { lv: 'INVARIANTS' }, src: "Martin's chat with Claude, 2026-09-14",
        text: 'the obvious invariant (how was this missed?!) is that the size per edit should be proportional to the edit',
        more: 'Eight minutes later: "Edit size proportional to page is an insane design that was seen by you, Opus, Sol and Astra and no one made any complaint. ... I\'m not looking for someone to blame but how I could improve the process." Since then, every change that adds stored data states its bytes per edit, measured.' },
    ] },

    // ── World 5: pull it out, then start again ───────────────────────────────
    { world: 'w5', at: '2026-09-15T10:23', props: [{ id: 'lever', kind: 'lever', x: 218 }], steps: [
      { who: 'narrator', text: 'Next morning. Managed storage (MS) never worked, and nobody needs it yet.' },
      { who: 'martin', act: 'pull:lever', gain: { tower: 176036 }, src: "Martin's chat with Claude, 2026-09-15",
        text: "OK, I'm fed up ... my proposal is to yank out MS completely" },
      { who: 'narrator', text: 'Plain Markdown files are the only storage again.' },
    ] },
    { world: 'w5', at: '2026-09-24T18:06', props: [{ id: 'wall', kind: 'wall', x: 370, label: 'COMPLEXITY' }], steps: [
      { who: 'narrator', text: 'Bugs keep coming. One takes fifteen review rounds in 36 hours.' },
      { who: 'martin', act: 'bump:wall', src: "Martin's chat with Claude, 2026-09-24",
        text: 'It feels like there is a codebase complexity threshold that was crossed' },
    ] },
    { world: 'w5', at: '2026-09-27T09:22', props: [{ id: 'wall2', kind: 'wall', x: 370, label: 'COMPLEXITY' }, { id: 'pipe', kind: 'pipe', x: 272 }], steps: [
      { who: 'martin', text: 'I have decided to try to start again from 0.6.5', src: "Martin's chat with Claude, 2026-09-27" },
      { act: 'enter:pipe', dur: 1.8 },
      { who: 'narrator', act: 'exit', gain: { tower: 54013, calm: true }, text: 'Back to 0.6.5, the last calm release, keeping everything learned on the way.' },
    ] },
    { world: 'w5', at: '2026-09-27T18:27', props: [], steps: [
      { who: 'narrator', text: 'The new storage design has just seven operations.' },
      { who: 'martin', text: '7 ops is something you can fit in your head and reason about.', src: "Martin's chat with Claude, 2026-09-27" },
    ] },

    // ── World 6: can we be sure this time? ───────────────────────────────────
    { world: 'w6', at: '2026-09-30T19:28:17', props: [], steps: [
      { who: 'narrator', text: 'Two resets later, one question: how can we be sure the new storage is right?' },
      { who: 'martin', mode: 'ask', src: "Martin's chat with Claude, 2026-09-30",
        text: 'Actually I know just as buzzwords that you can now prove a lot about code formally' },
    ] },
    { world: 'w6', at: '2026-10-02T09:45:40', props: [{ id: 'q4', kind: 'qblock', x: 174, pop: 'MODEL CHECKER' }], steps: [
      { who: 'martin', mode: 'ask', act: 'hit:q4', text: 'What is TLC?', src: "Martin's chat with Claude, 2026-10-02" },
      { who: 'narrator', text: 'A model checker: it tries every state the design can reach, looking for a broken rule.' },
      { who: 'martin', mode: 'aha', at: '2026-10-02T10:51:03', gain: { lv: 'MODEL CHECKING' }, src: "Martin's chat with Claude, 2026-10-02",
        text: "I don't want to weaken the guarantees we get from QUint, that's the whole point.",
        more: 'An hour after asking what TLC is. Five days later his own field joins in: "TLC sounds very brute-forcey. ... From my background I know finite model theory, Cooper\'s algorithm etc."' },
    ] },
    { world: 'w6', at: '2026-10-08T11:16', props: [{ id: 'f6', kind: 'flags', x: 260, flags: [], big: '0.7.0' }], steps: [
      { who: 'narrator', act: 'raise:f6', text: 'A tester calls Beta 2 the best release so far. It becomes 0.7.0.' },
    ] },
    { world: 'w6', at: '2026-10-08T22:12', props: [], steps: [
      { who: 'narrator', text: 'That evening the storage model is checked by machine and formalised in Lean.' },
      { who: 'martin', text: 'This is kinda awesome, no? :)', src: "Martin's chat with Claude, 2026-10-08" },
    ] },

    // ── World 7: keep building without shaking what people rely on ───────────
    { world: 'w7', at: '2026-10-08T16:06', props: [{ id: 'lines', kind: 'lines', x: 330, labels: ['STABLE', 'BETA', 'DEVELOPMENT'] }], steps: [
      { who: 'narrator', text: 'People rely on Tine 0.7.0 now, and Martin wants to keep building.' },
      { who: 'martin', act: 'fork:lines', src: "Martin's chat with Claude, 2026-10-08",
        text: 'keep releasing it as beta, and when things wind down / stabilize again merge to master',
        more: 'The whole sentence: "I want to continue the work in og and keep releasing it as beta, and when things wind down / stabilize again merge to master, if that makes sense?" og is the development line.' },
      { who: 'narrator', text: 'Three lines: stable for everyone, Beta for testers, development for new work.' },
    ] },
    { world: 'w7', at: '2026-10-13T12:00', props: [{ id: 'f7', kind: 'flags', x: 300, flags: ['CHINESE', 'SHARE', 'CLI'], big: 'BETA 1' }], steps: [
      { who: 'narrator', act: 'raise:f7', text: 'The first 0.8 Beta: Tine speaks Chinese, takes shares from phones, and gets a command line.' },
    ] },
    { world: 'w7', at: '2026-10-17T20:26', props: [{ id: 'lost', kind: 'box', x: 430, label: 'MARGIN COMMENTS', buried: true }], steps: [
      { who: 'narrator', text: 'Many agents work at once now. A finished feature turns up, never merged.' },
      { who: 'martin', act: 'pop:lost', src: "Martin's chat with Claude, 2026-10-17",
        text: "since you found something and apparently it wasn't merged I find that pretty scary" },
      { who: 'system', text: 'The PARTY learns: every job gets a card the moment it starts.' },
    ] },
    { world: 'w7', at: '2026-10-21T08:01', props: [{ id: 'bots', kind: 'bots', x: 400, n: 4, keep: 2 }], steps: [
      { who: 'martin', act: 'crash:bots', src: "Martin's chat with Claude, 2026-10-21",
        text: 'This session crashed, probably because of memory ... now work only 2 lanes in parallel.' },
    ] },
    { world: 'w7', at: '2026-10-21T16:02', props: [{ id: 'claim', kind: 'sign', x: 430, label: 'PROVEN STORAGE', later: 'IN 0.9' }], steps: [
      { who: 'martin', src: "Martin's chat with Claude, 2026-10-21",
        text: 'I wanted to ship 0.8 as the one where "we have a proven storage model" I cannot claim that before integrating this, right?' },
      { who: 'narrator', act: 'defer:claim', text: 'So 0.8 claims only what is checked. The full claim waits for 0.9.' },
    ] },
    { world: 'w7', final: true, at: '2026-10-26T06:29:05', props: [{ id: 'f8', kind: 'flags', x: 300, flags: ['12 BETAS', 'TESTED'], big: '0.8.0' }], steps: [
      { who: 'narrator', text: 'Twelve Betas in eleven days. Nothing severe is left in the reports.',
        more: 'Tine 0.8.0 is Beta 12 (October 24) plus one fix on top: on macOS, command-line commands could not reach the running app (GH #771).' },
      { who: 'martin', text: 'Ok, yeah, cut 0.8.0 as you described.', src: "Martin's chat with Claude, 2026-10-26" },
      { who: 'narrator', act: 'raise:f8', at: '2026-10-26T11:08:29', text: 'October 26: Tine 0.8.0 is stable, for everyone.' },
      { who: 'system', act: 'cheer', text: 'STAGE CLEAR! The PARTY celebrates. Then back to work: 0.9 is next.' },
    ] },
  ],
};
