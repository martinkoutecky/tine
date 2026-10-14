import { Rich } from "../i18n/Rich";
import { msg, t, render, renderEnglish, type Msg } from "../i18n";
import { For, Show, createEffect, createMemo, onCleanup, type JSX } from "solid-js";
import { openExternal } from "./primitives";
import { closeHelpPopup, helpPopupOpen, openSettings, openWelcome, toggleHelpPopup } from "../ui";
import { graphMeta } from "../graphSession";
import { BUILTIN_KEYS, type BuiltinKeyDef, type ShortcutScope } from "../keybindings";
import { EmojiText } from "../render/emoji";
import { openGuide } from "../guide";
import { dismissOnOutsidePointer, registerTransientLayer } from "../transientLayers";
import "../styles/help.css";

const REPO = "https://github.com/martinkoutecky/tine";

type HelpItem =
  | { label: Msg; detail: Msg; run: () => void }
  | { label: Msg; detail: Msg; href: string };

export const HELP_ITEMS: HelpItem[] = [
  {
    label: msg("help-guide"),
    detail: msg("help-open-the-in-app-how-to-guide"),
    run: () => void openGuide(),
  },
  {
    label: msg("help-keyboard-shortcuts"),
    detail: msg("help-open-settings-on-the-shortcuts-tab"),
    run: () => openSettings("shortcuts"),
  },
  {
    label: msg("help-features-documentation"),
    detail: msg("help-open-docs-features-md-on-github"),
    href: `${REPO}/blob/main/docs/FEATURES.md`,
  },
  {
    label: msg("help-report-a-bug-request-a-feature"),
    detail: msg("help-open-the-github-issue-templates"),
    href: `${REPO}/issues/new/choose`,
  },
  {
    label: msg("help-release-notes"),
    detail: msg("help-open-the-github-releases-page"),
    href: `${REPO}/releases`,
  },
  {
    label: msg("help-welcome-screen"),
    detail: msg("help-show-first-run-onboarding-again"),
    run: openWelcome,
  },
];



export function HelpPopup(): JSX.Element {
  let root: HTMLDivElement | undefined;

  dismissOnOutsidePointer({ open: helpPopupOpen, inside: () => [root], dismiss: closeHelpPopup });
  createEffect(() => {
    if (!helpPopupOpen()) return;
    const unregister = registerTransientLayer({ id: "help", root: () => root ?? null, dismiss: () => { closeHelpPopup(); return true; } });
    onCleanup(unregister);
  });

  const run = (item: HelpItem) => {
    closeHelpPopup();
    if ("href" in item) openExternal(item.href);
    else item.run();
  };

  return (
    <div class="help-corner" ref={root}>
      <Show when={helpPopupOpen()}>
        <div class="help-menu" role="menu" aria-label={t("help-help")}>
          <For each={HELP_ITEMS}>
            {(item) => (
              <button class="help-menu-item" role="menuitem" onClick={() => run(item)}>
                <span class="help-menu-label">
                  <EmojiText text={render(item.label)} />
                </span>
                <span class="help-menu-detail">
                  <EmojiText text={render(item.detail)} />
                </span>
              </button>
            )}
          </For>
        </div>
      </Show>
      <button
        class="help-corner-btn"
        title={t("help-help-with-shortcut")}
        aria-label={t("help-help")}
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
  /** In the active UI language; `english` also matches a search (tine#107). */
  label: string;
  english?: string;
  binding: string;
  /** A second built-in chord that runs the same command. Shown beside the
   *  binding so it is discoverable, but not remappable and not its own row: it
   *  disappears the moment the user binds the command themselves. */
  alias?: string;
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

const SHORTCUT_GROUPS: { scope: ShortcutScope; title: Msg; description: Msg }[] = [
  {
    scope: "global",
    title: msg("help-global"),
    description: msg("help-available-across-the-app-modifier-chords-can"),
  },
  {
    scope: "select",
    title: msg("help-select-mode"),
    description: msg("help-available-when-no-text-editor-is-focused"),
  },
  {
    scope: "editor",
    title: msg("help-editor"),
    description: msg("help-available-while-editing-a-block"),
  },
];

/** Group shortcut rows and filter by all search terms across command labels,
 * ids and bindings. Cost: O(commands × query length); no I/O or failure. */
export function buildShortcutPaneData(shortcuts: ShortcutSettingRow[], search = ""): ShortcutPaneSection[] {
  const terms = search.trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matches = (...values: (string | undefined)[]) => {
    const text = values.filter(Boolean).join(" ").toLocaleLowerCase();
    return terms.every((term) => text.includes(term));
  };
  return SHORTCUT_GROUPS.map((group) => ({
    ...group,
    title: render(group.title),
    description: render(group.description),
    commands: shortcuts.filter((s) => s.scope === group.scope && matches(s.label, s.english, s.id, s.effective, s.binding)),
    builtins: BUILTIN_KEYS.filter((s) => s.scope === group.scope && matches(render(s.label), renderEnglish(s.label), s.id, s.binding, s.details && render(s.details), s.details && renderEnglish(s.details))),
  }));
}

export function shortcutPaneCommandIds(sections: ShortcutPaneSection[]): string[] {
  return sections.flatMap((section) => section.commands.map((row) => row.id));
}

function displayBinding(binding: string): string {
  const b = binding.trim();
  if (!b) return t("help-unbound");
  if (b === "false") return t("help-unbound");
  return b;
}

function bindingFor(shortcuts: ShortcutSettingRow[], id: string, fallback: string): string {
  return displayBinding(shortcuts.find((s) => s.id === id)?.effective ?? fallback);
}

function TriggerTable(props: { shortcuts: ShortcutSettingRow[] }): JSX.Element {
  const rows = createMemo(() => [
    {
      trigger: "/",
      label: msg("help-slash-commands"),
      detail: msg("help-commands-templates-dates-uploads-queries-and-callouts"),
    },
    {
      trigger: bindingFor(props.shortcuts, "go/search", "mod+k"),
      label: msg("help-search-quick-switch"),
      detail: msg("help-jump-to-pages-and-blocks-from-the"),
    },
    {
      trigger: "[[ ]]",
      label: msg("help-page-reference-autocomplete"),
      detail: msg("help-search-or-create-pages-while-typing-a"),
    },
    {
      trigger: "(( ))",
      label: msg("help-block-reference-autocomplete"),
      detail: msg("help-search-blocks-and-insert-a-durable-block"),
    },
    {
      trigger: "shift+click",
      label: msg("help-open-reference-in-sidebar"),
      detail: msg("help-works-on-pages-block-references-bullets-and"),
    },
    {
      trigger: "right-click",
      label: msg("help-context-menu"),
      detail: msg("help-open-block-page-reference-color-copy-template"),
    },
  ]);

  return (
    <section class="help-settings-section">
      <div class="help-section-head"><h3>{t("help-triggers")}</h3>
      </div>
      <table class="help-table">
        <thead>
          <tr><th>{t("help-action")}</th><th>{t("help-trigger")}</th><th>{t("help-notes")}</th>
          </tr>
        </thead>
        <tbody>
          <For each={rows()}>
            {(row) => (
              <tr>
                <td>{render(row.label)}</td>
                <td><code>{row.trigger}</code></td>
                <td>{render(row.detail)}</td>
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
  label: Msg;
  md: string;
  org: string;
  result: () => JSX.Element;
};

const SYNTAX_ROWS: SyntaxRow[] = [
  { label: msg("help-bold"), md: "**Bold**", org: "*Bold*", result: () => <strong>Bold</strong> },
  { label: msg("help-italic"), md: "*Italic*", org: "/Italic/", result: () => <em>Italic</em> },
  { label: msg("help-strikethrough"), md: "~~Strike~~", org: "+Strike+", result: () => <del>Strike</del> },
  { label: msg("help-highlight"), md: "==Highlight==", org: "^^Highlight^^", result: () => <mark>Highlight</mark> },
  { label: msg("help-latex"), md: "$$E = mc^2$$", org: "$$E = mc^2$$", result: () => <span>E = mc^2</span> },
  { label: msg("help-inline-code"), md: "`code`", org: "~code~", result: () => <code>code</code> },
  {
    label: msg("help-code-block"),
    md: "```clojure\n(println \"Hello\")\n```",
    org: "#+BEGIN_SRC clojure\n(println \"Hello\")\n#+END_SRC",
    result: () => <pre><code>(println "Hello")</code></pre>,
  },
  {
    label: msg("help-link"),
    md: "[Link](https://example.com)",
    org: "[[https://example.com][Link]]",
    result: () => <a>Link</a>,
  },
  {
    label: msg("help-image"),
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
      <div class="help-section-head"><h3>{format() === "org" ? t("help-org-syntax") : t("help-markdown-syntax")}</h3>
        <button class="help-link-button" onClick={() => openExternal(learnMore())}>{t("help-learn-more")}</button>
      </div>
      <table class="help-table help-syntax-table">
        <thead>
          <tr><th>{t("help-style")}</th><th>{t("help-syntax")}</th><th>{t("help-result")}</th>
          </tr>
        </thead>
        <tbody>
          <For each={SYNTAX_ROWS}>
            {(row) => (
              <tr>
                <td>{render(row.label)}</td>
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
  onUnbind: (id: string) => void;
  onReset: (id: string) => void;
}): JSX.Element {
  const recording = () => props.recording === props.row.id;
  return (
    <div class="help-shortcut-row">
      <span class="help-shortcut-label">{props.row.label}</span>
      <span class="help-shortcut-keys">
        <button
          class="help-keycap help-keycap-button"
          classList={{ recording: recording(), overridden: props.row.overridden, disabled: props.row.effective === "false" }}
          onClick={() => props.onRecord(props.row.id)}
          title={t("help-click-to-remap")}
        >{recording() ? t("help-press-keys") : displayBinding(props.row.effective)}
        </button>
        <Show when={props.row.alias && !props.row.overridden}>
          <span class="help-shortcut-alias"><Rich message={msg("help-or", { alias: displayBinding(props.row.alias!) })} slots={{ "span": (children) => <span class="help-keycap help-keycap-static">{children}</span> }} /></span>
        </Show>
      </span>
      <span class="help-shortcut-tail">
        <Show when={props.row.effective.trim() && props.row.effective !== "false"}><button class="help-reset" title={t("help-remove-this-keybinding")} onClick={() => props.onUnbind(props.row.id)}>{t("help-unbind")}</button>
        </Show>
        <Show when={props.row.overridden}>
          <button
            class="help-reset"
            title={t("help-reset-to-default")}
            onClick={() => props.onReset(props.row.id)}
          >{t("help-reset")}</button>
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
        {render(props.row.label)}
        <Show when={props.row.details}>
          <span class="help-shortcut-detail">{render(props.row.details!)}</span>
        </Show>
      </span>
      <span class="help-keycap help-keycap-static">{props.row.binding}</span>
      <span class="help-shortcut-tail"><span class="help-builtin-pill">{t("help-built-in")}</span>
        <span class="mono help-shortcut-id">{props.row.id}</span>
      </span>
    </div>
  );
}

export function ShortcutsSettingsPane(props: {
  shortcuts: ShortcutSettingRow[];
  search: string;
  recording: string | null;
  onRecord: (id: string) => void;
  onUnbind: (id: string) => void;
  onReset: (id: string) => void;
}): JSX.Element {
  const sections = createMemo(() => buildShortcutPaneData(props.shortcuts, props.search));

  return (
    <div class="help-shortcuts-pane">
      <div class="settings-hint settings-block"><Rich message={msg("help-click-a-binding-to-record-new-keys")} slots={{ "code": (children) => <code>{children}</code> }} /></div>

      <Show when={!props.search.trim()}>
        <TriggerTable shortcuts={props.shortcuts} />
        <SyntaxTable />
      </Show>
      <Show when={props.search.trim() && sections().every((section) => !section.commands.length && !section.builtins.length)}><div class="settings-search-empty">{t("help-no-matching-shortcuts")}</div>
      </Show>

      <For each={sections()}>
        {(section) => (
          <Show when={section.commands.length || section.builtins.length}>
          <section class="help-settings-section help-shortcut-group">
            <div class="help-section-head help-shortcut-group-head">
              <div>
                <h3>{section.title}</h3>
                <p>{section.description}</p>
              </div>
            </div>

            <Show when={section.commands.length}><div class="help-shortcut-subhead">{t("help-remappable-commands")}</div>
              <div class="help-shortcut-list">
                <For each={section.commands}>
                  {(row) => (
                    <ShortcutRow
                      row={row}
                      recording={props.recording}
                      onRecord={props.onRecord}
                      onUnbind={props.onUnbind}
                      onReset={props.onReset}
                    />
                  )}
                </For>
              </div>
            </Show>

            <Show when={section.builtins.length}><div class="help-shortcut-subhead">{t("help-built-in-keys")}</div>
              <div class="help-shortcut-list">
                <For each={section.builtins}>
                  {(row) => <BuiltinRow row={row} />}
                </For>
              </div>
            </Show>
          </section>
          </Show>
        )}
      </For>
    </div>
  );
}
