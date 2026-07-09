import { For, Show, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { backend } from "../backend";
import {
  closeHelpPopup,
  graphMeta,
  helpPopupOpen,
  openSettings,
  openWelcome,
  toggleHelpPopup,
} from "../ui";
import { BUILTIN_KEYS, type BuiltinKeyDef, type ShortcutScope } from "../keybindings";
import { EmojiText } from "../render/emoji";
import { openGuide } from "../guide";
import "../styles/help.css";

const REPO = "https://github.com/martinkoutecky/tine";

type HelpItem =
  | { label: string; detail: string; run: () => void }
  | { label: string; detail: string; href: string };

export const HELP_ITEMS: HelpItem[] = [
  {
    label: "Guide",
    detail: "Open the in-app how-to guide",
    run: () => void openGuide(),
  },
  {
    label: "Keyboard shortcuts",
    detail: "Open Settings on the shortcuts tab",
    run: () => openSettings("shortcuts"),
  },
  {
    label: "Features documentation",
    detail: "Open docs/FEATURES.md on GitHub",
    href: `${REPO}/blob/main/docs/FEATURES.md`,
  },
  {
    label: "Report a bug / Request a feature",
    detail: "Open the GitHub issue templates",
    href: `${REPO}/issues/new/choose`,
  },
  {
    label: "Release notes",
    detail: "Open the GitHub releases page",
    href: `${REPO}/releases`,
  },
  {
    label: "Welcome screen",
    detail: "Show first-run onboarding again",
    run: openWelcome,
  },
];

function openExternal(url: string) {
  void backend().openExternal(url).catch(() => {});
}

export function HelpPopup(): JSX.Element {
  let root: HTMLDivElement | undefined;

  createEffect(() => {
    if (!helpPopupOpen()) return;

    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as Node | null;
      if (root && target && !root.contains(target)) closeHelpPopup();
    };
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.preventDefault();
      e.stopPropagation();
      closeHelpPopup();
    };

    window.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("keydown", onKeyDown, true);
    onCleanup(() => {
      window.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("keydown", onKeyDown, true);
    });
  });

  const run = (item: HelpItem) => {
    closeHelpPopup();
    if ("href" in item) openExternal(item.href);
    else item.run();
  };

  return (
    <div class="help-corner" ref={root}>
      <Show when={helpPopupOpen()}>
        <div class="help-menu" role="menu" aria-label="Help">
          <For each={HELP_ITEMS}>
            {(item) => (
              <button class="help-menu-item" role="menuitem" onClick={() => run(item)}>
                <span class="help-menu-label">
                  <EmojiText text={item.label} />
                </span>
                <span class="help-menu-detail">
                  <EmojiText text={item.detail} />
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <button
        class="help-corner-btn"
        title="Help (?)"
        aria-label="Help"
        aria-haspopup="menu"
        aria-expanded={helpPopupOpen()}
        onClick={toggleHelpPopup}
      >
        ?
      </button>
    </div>
  );
}

export interface ShortcutSettingRow {
  id: string;
  label: string;
  binding: string;
  effective: string;
  overridden: boolean;
  scope: ShortcutScope;
}

export interface ShortcutPaneSection {
  scope: ShortcutScope;
  title: string;
  description: string;
  commands: ShortcutSettingRow[];
  builtins: BuiltinKeyDef[];
}

const SHORTCUT_GROUPS: { scope: ShortcutScope; title: string; description: string }[] = [
  {
    scope: "global",
    title: "Global",
    description: "Available across the app. Modifier chords can also fire while a block is being edited.",
  },
  {
    scope: "select",
    title: "Select mode",
    description: "Available when no text editor is focused, including block selection and navigation sequences.",
  },
  {
    scope: "editor",
    title: "Editor",
    description: "Available while editing a block.",
  },
];

export function buildShortcutPaneData(shortcuts: ShortcutSettingRow[]): ShortcutPaneSection[] {
  return SHORTCUT_GROUPS.map((group) => ({
    ...group,
    commands: shortcuts.filter((s) => s.scope === group.scope),
    builtins: BUILTIN_KEYS.filter((s) => s.scope === group.scope),
  }));
}

export function shortcutPaneCommandIds(sections: ShortcutPaneSection[]): string[] {
  return sections.flatMap((section) => section.commands.map((row) => row.id));
}

function displayBinding(binding: string): string {
  const b = binding.trim();
  if (!b) return "Unbound";
  if (b === "false") return "Disabled";
  return b;
}

function bindingFor(shortcuts: ShortcutSettingRow[], id: string, fallback: string): string {
  return displayBinding(shortcuts.find((s) => s.id === id)?.effective ?? fallback);
}

function TriggerTable(props: { shortcuts: ShortcutSettingRow[] }): JSX.Element {
  const rows = createMemo(() => [
    {
      trigger: "/",
      label: "Slash commands",
      detail: "Commands, templates, dates, uploads, queries, and callouts inside the block editor.",
    },
    {
      trigger: bindingFor(props.shortcuts, "go/search", "mod+k"),
      label: "Search / quick switch",
      detail: "Jump to pages and blocks from the switcher.",
    },
    {
      trigger: "[[ ]]",
      label: "Page reference autocomplete",
      detail: "Search or create pages while typing a page reference.",
    },
    {
      trigger: "(( ))",
      label: "Block reference autocomplete",
      detail: "Search blocks and insert a durable block reference.",
    },
    {
      trigger: "shift+click",
      label: "Open reference in sidebar",
      detail: "Works on pages, block references, bullets, and reference-count badges.",
    },
    {
      trigger: "right-click",
      label: "Context menu",
      detail: "Open block, page, reference, color, copy, template, and delete actions.",
    },
  ]);

  return (
    <section class="help-settings-section">
      <div class="help-section-head">
        <h3>Triggers</h3>
      </div>
      <table class="help-table">
        <thead>
          <tr>
            <th>Action</th>
            <th>Trigger</th>
            <th>Notes</th>
          </tr>
        </thead>
        <tbody>
          <For each={rows()}>
            {(row) => (
              <tr>
                <td>{row.label}</td>
                <td><code>{row.trigger}</code></td>
                <td>{row.detail}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </section>
  );
}

type SyntaxFormat = "md" | "org";
type SyntaxRow = {
  label: string;
  md: string;
  org: string;
  result: () => JSX.Element;
};

const SYNTAX_ROWS: SyntaxRow[] = [
  { label: "Bold", md: "**Bold**", org: "*Bold*", result: () => <strong>Bold</strong> },
  { label: "Italic", md: "*Italic*", org: "/Italic/", result: () => <em>Italic</em> },
  { label: "Strikethrough", md: "~~Strike~~", org: "+Strike+", result: () => <del>Strike</del> },
  { label: "Highlight", md: "==Highlight==", org: "^^Highlight^^", result: () => <mark>Highlight</mark> },
  { label: "LaTeX", md: "$$E = mc^2$$", org: "$$E = mc^2$$", result: () => <span>E = mc^2</span> },
  { label: "Inline code", md: "`code`", org: "~code~", result: () => <code>code</code> },
  {
    label: "Code block",
    md: "```clojure\n(println \"Hello\")\n```",
    org: "#+BEGIN_SRC clojure\n(println \"Hello\")\n#+END_SRC",
    result: () => <pre><code>(println "Hello")</code></pre>,
  },
  {
    label: "Link",
    md: "[Link](https://example.com)",
    org: "[[https://example.com][Link]]",
    result: () => <a>Link</a>,
  },
  {
    label: "Image",
    md: "![image](../assets/example.png)",
    org: "[[../assets/example.png][image]]",
    result: () => <span class="help-image-chip">image</span>,
  },
];

function SyntaxTable(): JSX.Element {
  const format = createMemo<SyntaxFormat>(() => graphMeta()?.preferred_format === "org" ? "org" : "md");
  const learnMore = () =>
    format() === "org"
      ? "https://orgmode.org/worg/dev/org-syntax.html"
      : "https://www.markdownguide.org/basic-syntax";

  return (
    <section class="help-settings-section">
      <div class="help-section-head">
        <h3>{format() === "org" ? "Org syntax" : "Markdown syntax"}</h3>
        <button class="help-link-button" onClick={() => openExternal(learnMore())}>
          Learn more
        </button>
      </div>
      <table class="help-table help-syntax-table">
        <thead>
          <tr>
            <th>Style</th>
            <th>Syntax</th>
            <th>Result</th>
          </tr>
        </thead>
        <tbody>
          <For each={SYNTAX_ROWS}>
            {(row) => (
              <tr>
                <td>{row.label}</td>
                <td><code>{format() === "org" ? row.org : row.md}</code></td>
                <td class="help-rendered">{row.result()}</td>
              </tr>
            )}
          </For>
        </tbody>
      </table>
    </section>
  );
}

function ShortcutRow(props: {
  row: ShortcutSettingRow;
  recording: string | null;
  onRecord: (id: string) => void;
  onReset: (id: string) => void;
}): JSX.Element {
  const recording = () => props.recording === props.row.id;
  return (
    <div class="help-shortcut-row">
      <span class="help-shortcut-label">{props.row.label}</span>
      <button
        class="help-keycap help-keycap-button"
        classList={{ recording: recording(), overridden: props.row.overridden, disabled: props.row.effective === "false" }}
        onClick={() => props.onRecord(props.row.id)}
        title="Click to remap"
      >
        {recording() ? "Press keys..." : displayBinding(props.row.effective)}
      </button>
      <span class="help-shortcut-tail">
        <Show when={props.row.overridden}>
          <button
            class="help-reset"
            title="Reset to default"
            onClick={() => props.onReset(props.row.id)}
          >
            Reset
          </button>
        </Show>
        <span class="mono help-shortcut-id">{props.row.id}</span>
      </span>
    </div>
  );
}

function BuiltinRow(props: { row: BuiltinKeyDef }): JSX.Element {
  return (
    <div class="help-shortcut-row help-builtin-row">
      <span class="help-shortcut-label">
        {props.row.label}
        <Show when={props.row.details}>
          <span class="help-shortcut-detail">{props.row.details}</span>
        </Show>
      </span>
      <span class="help-keycap help-keycap-static">{props.row.binding}</span>
      <span class="help-shortcut-tail">
        <span class="help-builtin-pill">Built-in</span>
        <span class="mono help-shortcut-id">{props.row.id}</span>
      </span>
    </div>
  );
}

export function ShortcutsSettingsPane(props: {
  shortcuts: ShortcutSettingRow[];
  recording: string | null;
  onRecord: (id: string) => void;
  onReset: (id: string) => void;
}): JSX.Element {
  const sections = createMemo(() => buildShortcutPaneData(props.shortcuts));

  return (
    <div class="help-shortcuts-pane">
      <div class="settings-hint settings-block">
        Click a binding to record new keys (<code>mod</code> = Ctrl, or Cmd on macOS). Esc cancels.
        Overrides are saved locally on top of <code>config.edn</code>.
      </div>

      <TriggerTable shortcuts={props.shortcuts} />
      <SyntaxTable />

      <For each={sections()}>
        {(section) => (
          <section class="help-settings-section help-shortcut-group">
            <div class="help-section-head help-shortcut-group-head">
              <div>
                <h3>{section.title}</h3>
                <p>{section.description}</p>
              </div>
            </div>

            <Show when={section.commands.length}>
              <div class="help-shortcut-subhead">Remappable commands</div>
              <div class="help-shortcut-list">
                <For each={section.commands}>
                  {(row) => (
                    <ShortcutRow
                      row={row}
                      recording={props.recording}
                      onRecord={props.onRecord}
                      onReset={props.onReset}
                    />
                  )}
                </For>
              </div>
            </Show>

            <Show when={section.builtins.length}>
              <div class="help-shortcut-subhead">Built-in keys</div>
              <div class="help-shortcut-list">
                <For each={section.builtins}>
                  {(row) => <BuiltinRow row={row} />}
                </For>
              </div>
            </Show>
          </section>
        )}
      </For>
    </div>
  );
}
