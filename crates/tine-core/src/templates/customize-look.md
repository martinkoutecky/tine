icon:: 🎨

- # Customize Tine's look
  - Change colors, fonts, widths, bullets, and embed shading without a plugin: write a few lines of CSS in your graph's `logseq/custom.css`. This page shows where the file is, which names Tine promises to keep stable (the `--tine-*` tokens), ten copy-paste recipes, and how to find out what styles any element.
- ## 1. Start from a color scheme
  - 1. Open Settings (**t s**) → **Appearance** and look at **Color scheme**. Default, Nord, Solarized, Gruvbox, and **Soft** are built in. Soft is a gentler, medium-brightness take on Tine's own look: a warm medium-light in light mode and a neutral medium-dark in dark mode.
  - 2. Pick one. It applies at once and is remembered on this device.
  - 3. Precedence, lowest to highest: Tine's stock palette, then the color scheme you picked, then your graph's `logseq/custom.css`. Your CSS always has the last word, and an installed theme package never edits it.
  - What you should see: the whole window recolors together. Anything you then set in `custom.css` overrides only what you set.
- ## 2. Open your custom.css
  - 1. Settings → **Appearance** → **Custom CSS** → **Edit custom.css**. Tine creates `logseq/custom.css` in your graph folder with a short commented starter if the graph has none, then opens it in your system's default editor. It never overwrites a file that already exists.
  - 2. Save the file. Tine notices the change on disk and re-applies it in the open window: no restart and no reopening the graph.
  - 3. On Android there is no editor hand-off. Open `logseq/custom.css` in your graph folder with any text editor; Tine re-applies it when the file changes, and reopening the graph always does.
  - What you should see: a rule such as `:root { --tine-bullet-color: orange; }` recolors every bullet as soon as you save.
  - Something went wrong? **Settings → Appearance → Custom CSS → Disable custom CSS** ignores the file for this session only. It is never saved, so restarting Tine always brings custom CSS back; fix the file, then switch it back on.
- ## 3. The supported names: `--tine-*` tokens
  - A token is a named setting. Each starts unset, which means "do what Tine always did", so nothing changes until you set one. Set tokens on `:root`, like the recipes below. These thirteen names are the ones Tine promises to keep: if one is ever removed or renamed, the changelog says so.
  - | Token | Controls | Default |
    | --- | --- | --- |
    | `--tine-embed-bg` | Background of an embedded block or page | The theme's secondary background |
    | `--tine-embed-accent` | The accent line and bullet of an embedded block | 35% accent mixed into the bullet color |
    | `--tine-bullet-color` | Color of the block bullet dot | The theme's bullet color |
    | `--tine-bullet-size` | Diameter of the block bullet dot | 6px |
    | `--tine-tag-color` | Color of `#tags` and tag chips | The theme's tag color |
    | `--tine-highlight-bg` | Background of `^^highlighted^^` text | The theme's highlight color |
    | `--tine-content-width` | Maximum width of the page column | 810px |
    | `--tine-content-width-wide` | Maximum width of the column in Wide mode | 100% |
    | `--tine-page-title-size` | Font size of a page title | 28px |
    | `--tine-font-size` | Base font size of page text | 16px |
    | `--tine-content-font` | Font family of the interface and page text | Inter, then your system fonts |
    | `--tine-mono-font` | Font family of code and other monospace text | MonoLisa, then your system mono fonts |
    | `--tine-editable-font` | Font family of text while you edit it | Inter, then the emoji font, then your system fonts |
  - Colors beyond these still use Logseq's `--ls-*` variables (for example `--ls-primary-background-color`), exactly as in Logseq, so a Logseq snippet you already have keeps working.
  - The page widths you set in Settings → Appearance (**Standard page width**, **Wide page width**) outrank `--tine-content-width` and `--tine-content-width-wide`. Reset those two fields if a token seems to have no effect.
- ## 4. Ten copy-paste recipes
  - Paste any of these into `logseq/custom.css` and save. Combine as many as you like.
  - Remove the shade behind embedded blocks:
  - ```css
    :root { --tine-embed-bg: transparent; }
    ```
  - Give embedded blocks a warm accent instead of the theme's:
  - ```css
    :root { --tine-embed-accent: #d97706; }
    ```
  - Change the bullet color, and make bullets a little bigger:
  - ```css
    :root {
      --tine-bullet-color: #d97706;
      --tine-bullet-size: 8px;
    }
    ```
  - A wider page column, in normal and in Wide mode:
  - ```css
    :root {
      --tine-content-width: 1100px;
      --tine-content-width-wide: 1500px;
    }
    ```
  - A different reading font, for display and for editing. Keep `var(--tine-editable-emoji-font)` in the editing list; it protects emoji from crashing some Linux systems:
  - ```css
    :root {
      --tine-content-font: Georgia, "Times New Roman", serif;
      --tine-editable-font: Georgia, var(--tine-editable-emoji-font), serif;
    }
    ```
  - Larger text everywhere the base size applies:
  - ```css
    :root { --tine-font-size: 18px; }
    ```
  - A different code font:
  - ```css
    :root { --tine-mono-font: "Fira Code", Consolas, monospace; }
    ```
  - A different tag color and highlight color:
  - ```css
    :root {
      --tine-tag-color: #0d9488;
      --tine-highlight-bg: #fde68a;
    }
    ```
  - Smaller page titles:
  - ```css
    :root { --tine-page-title-size: 22px; }
    ```
  - A different value only in dark mode (Tine marks the window `light` or `dark`):
  - ```css
    html[data-theme="dark"] { --tine-bullet-color: #fbbf24; }
    ```
  - Recolor the page itself with a Logseq variable, light mode only:
  - ```css
    html[data-theme="light"] { --ls-primary-background-color: #fdf6e3; }
    ```
- ## 5. Find out what styles an element
  - 1. Settings → **Appearance** → **Custom CSS** → **Developer tools** (desktop), or press **Ctrl+Shift+J**, opens the inspector. It is part of every desktop build, release builds included. Use its element picker, then read the **Styles** pane.
  - 2. Look for a `var(--tine-…)` or `var(--ls-…)` in the rule that draws what you want to change. Setting that variable in `custom.css` is the stable way to change it.
  - 3. When no variable exists, a class selector such as `.embed-block` works today, but class names and the structure of the page can change between releases, so a rule written that way may need a touch-up after an update. Prefer a token.
  - What you should see: the inspector opens and highlights the element you point at, with its rules listed in the **Styles** pane.
- ## 6. If it does not look right
  - A token seems to do nothing: check that you set it on `:root` (or `html[data-theme="dark"]`) and that the page-width Settings fields are reset for the two width tokens.
  - The whole window became unreadable: **Disable custom CSS** in Settings, or restart Tine with the file edited.
  - If a **custom.css could not be read** message appears, fix the file's permissions or size; **Edit custom.css** opens it even then.
- ## Where next
  - [[Workflows/Extend Tine]] covers theme packages and plugins.
  - [[Reference/Files, external edits, and backups]] explains what else lives in your graph's `logseq/` folder.
