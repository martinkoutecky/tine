icon:: 🎨

- # Customize Tine's look
  - Change colors, fonts, widths, bullets, and embed shading without a plugin: write a few lines of CSS in your graph's `logseq/custom.css`. This page shows where the file is, which names Tine promises to keep stable (the `--tine-*` tokens), ten copy-paste recipes, and how to find out what styles any element.
- ## 1. Start from a color scheme
  - 1. Open Settings (**t s**) → **Appearance** and look at **Color scheme**. Default, Nord, Solarized, Gruvbox, and **Soft** are built in. Soft is a medium-brightness palette: warm gray in light mode and lifted slate gray in dark mode, with warm highlights and quiet active tabs. Its page, sidebars, menus, and code surfaces move together while text stays readable.
  - 2. Pick one. It applies at once and is remembered on this device.
  - 3. Precedence, lowest to highest: Tine's stock palette, then the color scheme you picked, then the **Fine-tuning** choices below, then device-wide `global.css`, then your graph's `logseq/custom.css`. Your CSS always has the last word, and an installed theme package never edits it. A **Style** preset such as editorial reading type keeps its own look, but the font and size tokens below still override its fonts and text size.
  - 4. Open **Advanced** in Appearance for **Fine-tuning**, a few visual details that are not worth a theme of their own. **Embedded blocks** shows embeds as a shaded box (the default) or with only the accent line and dot. **Links to the web or a file** is a solid underline (the default), a dotted one, or none; links to pages never have one, so the two kinds read differently. **Links to pages that do not exist yet** is the dimmed link color (the default), a stronger one, or the same color as other links. Every choice starts at Tine's default look, takes effect at once, and has its own **Reset**. Settings search also finds these controls and opens Advanced.
  - **Arrows and dashes** (`->`, `<-`, `=>`, `--`, `---`) show exactly as you typed them, as in Logseq. To see them as glyphs (→, ⇒, –, —), set Settings → **Appearance** → **Typographic replacements** to **On (while reading)**, which changes only what you see and leaves your Markdown as typed, or **On (while typing)**, which rewrites the text itself as you type. It is **Off** by default; a device that used the earlier default, where arrows were drawn as glyphs, needs to pick **On (while reading)** once.
  - Fine-tuning choices are saved on this device (under `style.knobs.v1` in Tine's device settings), never in your graph, so they do not follow the graph to another computer. **Disable custom CSS** switches them off for the session along with your stylesheet, and your `custom.css` outranks them.
  - **Block embed breadcrumbs** is off by default. Turn it on to show a muted source page and parent trail above block embeds.
  - What you should see: the whole window recolors together. Anything you then set in `custom.css` overrides only what you set. A Fine-tuning choice changes its one detail at once and **Reset** puts it back.
- ## 2. Open your custom.css
  - 1. Settings → **Appearance** → **Advanced** → **Custom CSS** → **Edit custom.css** opens `logseq/custom.css` in Tine's text editor, including on mobile. Or press **Ctrl+K** (**Cmd+K** on macOS), type `css` or `custom.css`, and choose **Edit custom.css**. A missing file starts empty and is created only when you save.
  - 2. Press **Save** to apply changes immediately, or **Save and close** to return to your notes. If another editor or a sync tool changed the file, Tine refuses to overwrite it and keeps your text open. Copy your edits before choosing **Reload from disk**, then combine them with the latest file and save again.
    - Switching apps or backgrounding Tine starts saving CSS alongside your pages. If a conflict refuses the save, reopening the same CSS file restores the recoverable draft with its original revision; copy it before reloading from disk.
  - 3. **Edit global CSS** opens `global.css` in Tine's per-user config directory, outside every graph. It applies to every graph on this device and loads before each graph's `custom.css`, so the graph can override it. The quick switcher also finds **Edit global CSS (global.css)**. Saving applies it across open graph windows. This device file does not travel with a synced graph.
  - What you should see: a rule such as `:root { --tine-bullet-color: orange; }` recolors every bullet as soon as you save.
  - Something went wrong? **Settings → Appearance → Advanced → Custom CSS → Disable custom CSS** ignores both CSS files and Fine-tuning for this session only. It is never saved, so restarting Tine always brings custom CSS back; fix the file, then switch it back on. If the stylesheet hides Settings itself, open the command palette (**Mod+Shift+P**) and run **Toggle custom CSS (safe mode, this session)**.
- ## 3. The supported names: `--tine-*` tokens
  - A token is a named setting. Each starts unset, which means "use the theme's default", so nothing changes until you set one. Set tokens on `:root` or `html`, like the recipes below; any selector you write wins over Tine's own, because Tine declares them with no specificity. These sixteen names are the ones Tine promises to keep: if one is ever removed or renamed, the changelog says so.
  - | Token | Controls | Default |
    | --- | --- | --- |
    | `--tine-danger-color` | Error and destructive-action text and error borders (also the asset trash button's hover fill) | A contrast-checked red for the theme's light or dark mode |
    | `--tine-embed-bg` | Background of an embedded block or page | The theme's secondary background |
    | `--tine-embed-accent` | The accent line and bullet of an embedded block | 35% accent mixed into the bullet color |
    | `--tine-bullet-color` | Color of the block bullet dot | The theme's bullet color |
    | `--tine-bullet-size` | Diameter of the block bullet dot | 6px |
    | `--tine-tag-color` | Color of `#tags` and tag chips | The theme's tag color |
    | `--tine-highlight-bg` | Background of `^^highlighted^^` text | The theme's highlight color |
    | `--tine-missing-page-color` | Color of a link to a page that does not exist yet | A dimmed tint of the link color |
    | `--tine-external-link-underline` | The line under a link to a web address or file | `1px solid` (use `none` to remove it) |
    | `--tine-content-width` | Maximum width of the page column | 810px |
    | `--tine-content-width-wide` | Maximum width of the column in Wide mode | 100% |
    | `--tine-page-title-size` | Font size of a page title | 28px |
    | `--tine-font-size` | Base font size of page text | 16px |
    | `--tine-content-font` | Font family of the interface and page text | Inter, then your system fonts |
    | `--tine-mono-font` | Font family of code and other monospace text | MonoLisa, then your system mono fonts |
    | `--tine-editable-font` | Font family of text while you edit it | Inter, then the emoji font, then your system fonts |
  - The code and formula editors always edit in the monospace face, so `--tine-editable-font` does not reach them; `--tine-mono-font` does.
  - Colors beyond these still use Logseq's `--ls-*` variables (for example `--ls-primary-background-color`), exactly as in Logseq, so a Logseq snippet you already have keeps working.
  - The page widths you set in Settings → Appearance → Advanced (**Standard page width**, **Wide page width**) outrank `--tine-content-width` and `--tine-content-width-wide`. Reset those two fields if a token seems to have no effect.
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
  - A different reading font, for display and for editing (it also overrides the editorial reading style). Keep `var(--tine-editable-emoji-font)` in the editing list; it protects emoji from crashing some Linux systems:
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
  - 1. Settings → **Appearance** → **Advanced** → **Custom CSS** → **Developer tools** (desktop), or press **Ctrl+Shift+J** (**Cmd+Shift+J** on macOS), opens the inspector. It is part of every desktop build, release builds included. Use its element picker, then read the **Styles** pane.
  - 2. Look for a `var(--tine-…)` or `var(--ls-…)` in the rule that draws what you want to change. Setting that variable in `custom.css` is the stable way to change it.
  - 3. When no variable exists, a class selector such as `.embed-block` works today, but class names and the structure of the page can change between releases, so a rule written that way may need a touch-up after an update. Prefer a token.
  - What you should see: the inspector opens and highlights the element you point at, with its rules listed in the **Styles** pane.
- ## 6. If it does not look right
  - A token seems to do nothing: check that you set it on `:root` or `html` (or `html[data-theme="dark"]`) and that the page-width Settings fields are reset for the two width tokens.
  - The whole window became unreadable: **Disable custom CSS** in Settings, or restart Tine with the file edited.
  - If a **custom.css could not be read** message appears, fix the file's permissions or size with an external text editor, then try **Edit custom.css** again.
- ## Where next
  - [[Workflows/Extend Tine]] covers theme packages and plugins.
  - [[Reference/Files, external edits, and backups]] explains what else lives in your graph's `logseq/` folder.
