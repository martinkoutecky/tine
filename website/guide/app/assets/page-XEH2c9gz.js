const e=`page-open-failed = 无法打开此页面：<code>{ $error }</code>

page-no-journals = 此图谱中没有日记。

page-no-journals-hint = 请确认打开的是 Logseq 图谱（包含 <code>journals/</code> 和 <code>pages/</code> 的文件夹）。使用 <a>{ page-ui-open-graph }</a>

page-guide-read-only = 内置指南页面，仅供阅读，不会写入你的图谱。

page-ui-could-not-load-journal-feed-it-will-retry-when = 无法加载日记列表。视图刷新时会重试。

page-ui-tine-did-not-modify-the-file-try-reopening-or = Tine 未修改文件。请尝试重新打开，或检查磁盘文件。

page-ui-loading-page = 正在加载页面…

page-ui-open-graph = 打开图谱…

page-ui-rename-failed = 重命名失败：{ $value1 }

page-ui-bundled-guide-page = 内置指南页面

page-ui-double-click-to-rename-shift-click-sidebar-ctrl-middle = 双击重命名（Shift 点击 → 侧边栏，Ctrl/中键点击 → 新标签页，Alt 点击 → 另一窗格）

page-ui-shift-click-to-open-in-sidebar-ctrl-middle-click = Shift 点击在侧边栏打开，Ctrl/中键点击 → 新标签页，Alt 点击 → 另一窗格

page-ui-copy-the-guide-into-your-graph = 将指南复制到你的图谱

page-ui-page-actions = 页面操作

page-ui-unfavorite = 取消收藏

page-ui-add-to-favorites = 添加收藏

page-ui-also-known-as-other-names-that-link-here = 别名：可链接到此页面的其他名称

page-ui-aka = 别名

page-ui-tine-can-t-reproduce-this-org-file-byte-for = Tine 无法逐字节还原此 .org 文件，因此以只读方式显示，避免损坏。请在 Logseq/Emacs 中编辑。

page-ui-read-only-this-org-file-uses-a-structure-tine = 只读：此 <code>.org</code> 文件使用了 Tine 尚无法安全读写的结构，因此无法在此编辑。

page-ui-click-the-text-to-turn-it-into-an-editable = 点击文本，将其变为可编辑的块

page-ui-focus-or-create-a-trailing-block = 聚焦或创建末尾块

page-ui-click-to-continue-writing-below-this-page = 点击以在页面下方继续写作

page-ui-add-block = + 添加块

page-ui-hide-tag-table = 隐藏标签表格

page-ui-show-tagged-blocks-as-a-table = 将带标签的块显示为表格

page-ui-table = ⊞ 表格

page-ui-tag-table-couldn-t-load = 无法加载标签表格：{ $error }

page-ui-add-row = 添加 { $value1 } 行

page-ui-move-this-day-s-unfinished-tasks-to-today = 将当天的未完成任务移到今天

page-ui-carry-unfinished-tasks-today = 将未完成任务延续到今天

page-ui-to-today = 移到今天

page-ui-pull-unfinished-tasks-from-the-most-recent-day-that = 从最近有内容的一天移入未完成任务

page-ui-carry-from-previous-day = 从前一天延续

page-ui-previous = 前一天

page-ui-pull-unfinished-tasks-from-the-last-days-change-n = 从最近 { $value1 } 天移入未完成任务（可在设置中更改 N）

page-ui-carry-last-days = 延续最近 { $value1 } 天

page-ui-last-d = 最近 { $value1 } 天

page-prose-key-grammar = 键只能包含字母、数字、_、.、/ 或 -，以便 Tine 的所有读取功能都能识别。
page-prose-key-managed = “{ $key }”由 Tine 管理，无法在此设置。
page-prose-key-exists = “{ $key }”已存在，请编辑上方对应的行。
`;export{e as default};
