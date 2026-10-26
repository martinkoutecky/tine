const n=`pdf-copied-highlight-ref = 已复制高亮引用

pdf-couldn-t-load-pdf-annotations = 无法加载 PDF 批注。（{ $detail }）

pdf-couldn-t-capture-that-region-try-again = 无法截取该区域，请重试。

pdf-couldn-t-save-the-area-image-try-again = 无法保存区域图片，请重试。（{ $detail }）

pdf-finish-the-highlight-decision-or-area-image-cleanup-before-editing = 请先完成高亮处理或区域图片清理，再继续编辑。

pdf-resolve-the-highlight-conflict-before-editing-more-highlights = 请先解决高亮冲突，再编辑更多高亮。

pdf-cannot-close-or-switch-graphs-while-has-an-area-image-awaiting-cleanup-choose-retry-cleanup = { $name } 有待清理的区域图片，无法关闭或切换图谱。请选择“重试清理”。

pdf-cannot-close-or-switch-graphs-while-has-a-highlight-conflict-choose-keep-mine-or-use-disk-version-or-discard-my-changes = { $name } 有高亮冲突，无法关闭或切换图谱。请选择“保留我的版本”“使用磁盘版本”或“丢弃我的更改”。

pdf-couldn-t-retire-the-unused-area-image-retry-cleanup = 无法清理未使用的区域图片，请重试清理。（{ $detail }）

pdf-couldn-t-save-notes-highlight-not-written-resolve-the-conflict-and-retry = 无法保存笔记，高亮尚未写入。请解决冲突后重试。

pdf-highlight-saved-but-an-unused-area-image-couldn-t-move-to-trash = 高亮已保存，但未使用的区域图片无法移入回收站。（{ $detail }）

pdf-highlight-conflict-choose-keep-mine-or-use-disk-version = 高亮冲突，请选择“保留我的版本”或“使用磁盘版本”。

pdf-couldn-t-save-highlight-it-remains-unsaved-in-the-pdf = 无法保存高亮，它仍在 PDF 中处于未保存状态。（{ $detail }）

pdf-highlight-saved-but-notes-couldn-t-reload = 高亮已保存，但无法重新加载笔记。（{ $detail }）

pdf-using-disk-highlights-local-changes-were-discarded = 已使用磁盘高亮，本地更改已丢弃。

pdf-couldn-t-use-disk-highlights-or-retire-their-area-image = 无法使用磁盘高亮或清理其区域图片。（{ $detail }）

pdf-couldn-t-reload-disk-highlights-for-keep-mine = 无法为“保留我的版本”重新加载磁盘高亮。（{ $detail }）

pdf-discard-your-local-pdf-highlight-changes-the-sidecar-will-be-left-untouched-any-unreferenced-local-crop-may-remain-on-disk = 丢弃本地 PDF 高亮更改吗？批注数据文件保持原样。未被引用的本地裁剪图片可能仍保留在磁盘上。

pdf-local-highlight-changes-discarded-disk-sidecar-was-left-untouched = 本地高亮更改已丢弃，磁盘批注数据文件保持原样。

pdf-couldn-t-confirm-highlight-discard = 无法确认丢弃高亮。（{ $detail }）

pdf-untitled = 无标题

pdf-area-image-cleanup-pending = 区域图片待清理

pdf-unsaved-highlight = 未保存的高亮

pdf-previous-page = 上一页

pdf-page-type-a-number-and-press-enter-to-jump = 页码：输入数字并按 Enter 跳转

pdf-next-page = 下一页

pdf-find-in-document-ctrl-f = 在文档中查找（Ctrl+F）

pdf-zoom-out = 缩小

pdf-zoom-in = 放大

pdf-fit-width = 适应宽度

pdf-fit-height = 适应高度

pdf-area-highlight-drag-a-rectangle-to-capture-a-region-as-an-image = 区域高亮（{ $shortcut }）：拖出矩形，将区域截取为图片

pdf-open-highlights-notes-page = 打开高亮与笔记页面

pdf-notes = 笔记

pdf-outline = 大纲

pdf-more-settings = 更多设置

pdf-close-pdf = 关闭 PDF

pdf-highlight-conflict-resolve-it-before-editing-more-highlights = 高亮冲突。请先解决，再编辑更多高亮。

pdf-keep-mine = 保留我的版本

pdf-use-disk-version = 使用磁盘版本

pdf-discard-my-changes = 丢弃我的更改

pdf-area-image-cleanup-is-pending-retry-before-closing-or-switching-graphs = 有区域图片待清理。关闭或切换图谱前请重试。

pdf-retry-cleanup = 重试清理

pdf-pdf-settings = PDF 设置

pdf-reader-tools = 阅读工具

pdf-area-highlight = 区域高亮

pdf-theme = 主题

pdf-theme-options = PDF 主题

pdf-theme-label = { $theme ->
    [light] 浅色
    [warm] 暖色
   *[dark] 深色
    }

pdf-theme-description = { $theme ->
    [light] 浅色 PDF 主题
    [warm] 暖色 PDF 主题
   *[dark] 深色 PDF 主题
    }

pdf-document-outline = 文档大纲

pdf-loading-outline = 正在加载大纲…

pdf-outline-too-large-partially-shown = 大纲过大，仅显示部分内容。

pdf-no-outlines = 没有大纲

pdf-find-in-document = 在文档中查找

pdf-find-count = { $index } / { $count }{ $truncatedSuffix }

pdf-no-results-in-scanned-text = 已扫描文本中无结果+

pdf-no-results = 无结果

pdf-previous-match-shift-enter = 上一个匹配（Shift+Enter）

pdf-next-match-enter = 下一个匹配（Enter）

pdf-close-esc = 关闭（Esc）

pdf-couldn-t-open-this-pdf = { "无法打开此 PDF：" }

pdf-copy-ref = 复制引用

pdf-linked-references = 链接引用

pdf-remove-highlight = 移除高亮
pdf-outline-disclosure = { $state ->
    [collapse] 折叠 { $label }
   *[expand] 展开 { $label }
    }
`;export{n as default};
