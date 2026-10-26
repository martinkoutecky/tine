const n=`command-go-search = 搜索 / 快速切换

navigation-sidebar-title = 切换侧边栏（t l）

navigation-search-title = 搜索（Ctrl+K）

navigation-search = 搜索

command-go-search-current-page = 在当前页面中搜索块

command-guide-open = 打开指南

command-go-find-in-page = 在页面中查找

command-editor-follow-link = 打开光标处的链接

command-editor-open-link-in-sidebar = 在侧边栏中打开光标处的链接

command-command-palette-toggle = 命令面板

command-ui-toggle-devtools = 切换开发者工具

command-ui-toggle-custom-css = 切换自定义 CSS（安全模式，仅本次会话）

command-go-journals = 前往日记

command-go-home = 前往首页

command-go-journal-next = 前往下一天的日记

command-go-journal-prev = 前往上一天的日记

command-go-keyboard-shortcuts = 前往键盘快捷键

command-go-backward = 后退

command-go-forward = 前进

command-tab-close = 关闭当前标签页

command-tab-reopen-closed = 重新打开已关闭的标签页

command-tab-next = 下一个标签页

command-tab-previous = 上一个标签页

command-tab-activate-n = 切换到标签页 { $index }

command-tab-activate-last = 切换到最后一个标签页

command-editor-edit-selected-block = 编辑所选块

command-go-search-pages = 搜索页面名称

command-go-search-content = 搜索块内容

command-pane-split-right = 向右拆分

command-pane-split-down = 向下拆分

command-pane-close = 关闭窗格

command-pane-toggle-maximize = 切换最大化当前窗格

command-window-open-in-new-window = 在新窗口中打开当前页面

command-sidebar-grow-width = 加宽左侧边栏

command-sidebar-shrink-width = 收窄左侧边栏

command-right-sidebar-grow-width = 加宽右侧边栏

command-right-sidebar-shrink-width = 收窄右侧边栏

command-go-search-tab = 打开搜索标签页

command-pane-grow-width = 加宽当前窗格

command-pane-shrink-width = 收窄当前窗格

command-pane-grow-height = 增高当前窗格

command-pane-shrink-height = 降低当前窗格

command-pane-select-mode = 窗格选择模式（方向键移动，Enter 打开/拆分）

command-pane-focus-left = 聚焦左侧窗格

command-pane-focus-right = 聚焦右侧窗格

command-pane-focus-up = 聚焦上方窗格

command-pane-focus-down = 聚焦下方窗格

command-pane-move-tab-left = 将标签页移到左侧窗格

command-pane-move-tab-right = 将标签页移到右侧窗格

command-pane-move-tab-up = 将标签页移到上方窗格

command-pane-move-tab-down = 将标签页移到下方窗格

command-ui-toggle-theme = 切换深色 / 浅色

command-ui-reset-zoom = 重置界面缩放

command-ui-toggle-brackets = 切换引用括号

command-ui-toggle-left-sidebar = 切换左侧边栏

command-ui-toggle-right-sidebar = 切换右侧边栏

command-ui-open-settings = 打开设置

command-page-export-pdf = 将当前页面导出为 PDF…

command-ui-toggle-wide-mode = 切换宽屏模式

command-ui-toggle-document-mode = 切换文档模式

command-ui-toggle-focus-mode = 切换专注模式

command-ui-toggle-dim-blocks = 切换淡化非活动块

command-ui-toggle-help = 切换帮助

command-workspace-new = 新建工作区…

command-task-carry-n = 延续未完成任务：最近 N 天（见设置）

command-editor-undo = 撤销

command-editor-redo = 重做

command-editor-toggle-undo-redo-mode = 切换撤销/重做模式

command-editor-indent = 缩进块

command-editor-outdent = 取消缩进块

command-editor-move-block-up = 上移块

command-editor-move-block-down = 下移块

command-editor-collapse = 折叠块

command-editor-expand = 展开块

command-editor-select-block-up = 向上选择块

command-editor-select-block-down = 向下选择块

command-editor-select-all = 选择块 / 扩大选择

command-editor-cycle-todo = 循环切换 TODO / DOING / DONE

command-editor-quick-capture-file = 快速记录：归档到今天的日记

command-editor-bold = 粗体

command-editor-italics = 斜体

command-editor-strike-through = 删除线

command-editor-highlight = 高亮

command-editor-insert-link = 插入链接

command-editor-copy-embed = 未选中文字时复制块嵌入

command-editor-clear-block = 清空块内容

command-editor-kill-line-before = 删除至行首

command-editor-kill-line-after = 删除至行尾

command-editor-backward-word = 光标移到上一个词

command-editor-forward-word = 光标移到下一个词

command-editor-backward-kill-word = 删除光标前的一个词

command-editor-forward-kill-word = 删除光标后的一个词

command-pane-focus-n = 聚焦窗格 { $index }

command-plugin = 插件：{ $title }

command-go-search-selection = 搜索选中的文本

command-select-copy-block-ref = 复制所选块的块引用

command-editor-duplicate-block = 在下方复制块（含子块）

command-editor-duplicate-block-above = 在上方复制块（含子块）

command-editor-insert-child-block = 插入子块

command-editor-select-embedding-block = 选择嵌入所在块

command-editor-collapse-children = 折叠（一层）

command-editor-expand-children = 展开（一层）

command-editor-paste-as-link = 粘贴为链接

commands-open-a-page-first-to-export-it = 请先打开页面，再导出为 PDF

commands-undo-redo-mode = 撤销/重做模式：{ $mode }

commands-select-text-in-a-block-then-use = 请先选中块内文本，再使用“粘贴为链接”

command-builtin-global-escape = 关闭浮层或退出专注模式

commands-closes-search-settings-first-outside-overlays = 优先关闭搜索或设置；没有浮层时，依次退出块选择和专注模式。

command-builtin-editor-enter = 新建块（文档模式下为换行）

commands-normally-splits-the-current-block-continues-an = 通常会拆分当前块、延续块内列表，或在 PDF 批注下添加同级笔记。文档模式下会插入换行，除非文档模式的 Enter 设置保留此结构操作。

command-builtin-editor-soft-newline = 在块内换行（文档模式下为新建块）

command-builtin-editor-escape = 退出编辑并选中块

command-builtin-editor-backspace-start = 在块开头合并

commands-at-the-start-of-a-block-merges = 在块开头，与前一个块合并，或移除空块及列表标记。

command-builtin-editor-arrow-cross-block = 在可见行边缘跨块移动

commands-from-the-first-or-last-visual-row = 在第一行或最后一行，将光标移到上一个或下一个可见块。

command-builtin-editor-copy-block-ref = 未选中文字时复制块引用

command-builtin-editor-paste-plain = 粘贴为纯文本（不转换格式）

commands-inserts-the-clipboard-s-plain-text-as-typed = 将剪贴板中的纯文本原样插入当前块：忽略 HTML 结构，不将多行文本拆成块。普通粘贴仍会转换复制的网页内容和大纲。

command-builtin-editor-ac-nav = 自动补全：移动选中项

command-builtin-editor-ac-accept = 自动补全：确认选中项

command-builtin-editor-ac-close = 自动补全：关闭弹窗

command-builtin-select-escape = 清除块选择

command-builtin-select-move = 移动所选块范围

command-builtin-select-extend = 扩大块选择范围

command-builtin-select-delete = 删除所选块

command-builtin-select-copy = 复制所选大纲

command-builtin-select-cut = 剪切所选大纲

command-builtin-select-edit = 编辑最后一个所选块

command-builtin-sheet-select-cell = 选择表格视图单元格

commands-double-click-enter-or-f2-edits-the-selected = 双击、Enter 或 F2 可编辑所选单元格。

command-builtin-sheet-move = 移动表格视图中的单元格选择

commands-esc-leaves-a-sheet-typing-replaces-the = Esc 退出表格视图；输入会替换聚焦单元格的内容。

command-builtin-sheet-range = 选择表格视图范围

command-builtin-sheet-copy-cut = 复制、剪切或粘贴表格视图单元格

command-builtin-sheet-fill = 向下或向右填充所选表格视图范围

commands-plugin-command-failed = 插件命令失败：{ $error }

commands-couldn-t-copy-selection-clipboard-write-failed = 无法复制所选内容：写入剪贴板失败。

commands-couldn-t-cut-selection-clipboard-write-failed = 无法剪切所选内容：写入剪贴板失败。
`;export{n as default};
