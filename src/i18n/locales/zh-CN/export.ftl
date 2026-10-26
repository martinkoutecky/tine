export-modal-plain-text = 纯文本

export-modal-cleaned-the-text-as-displayed-without-markup-markers = 清理格式：保留显示的文本，移除格式标记（粗体、高亮、链接）

export-modal-markdown = Markdown

export-modal-formatting-preserved-block-references-resolved-and-embeds-expanded = 保留格式：解析块引用并展开嵌入

export-modal-text = 文本

export-modal-opml = OPML

export-modal-html = HTML

export-modal-dashes = 短横线

export-modal-logseq-outline-bullets = Logseq 大纲（- 项目符号）

export-modal-spaces = 空格

export-modal-indent-no-bullets = 缩进，无项目符号

export-modal-no-indent = 无缩进

export-modal-flat-no-bullets = 平铺，无项目符号

export-modal-links-text = [[links]] → 文本

export-modal-remove-emphasis = 移除强调格式

export-modal-remove-tags = 移除 #tags

export-modal-remove-properties = 移除属性

export-modal-newline-after-block = 块后换行

export-modal-resolve-refs-fully = 完整解析引用

export-modal-copy-export = 复制 / 导出{ " " }

export-modal-format = 格式

export-modal-content = 内容

export-modal-indent = 缩进

export-modal-level = 层级 ≤

export-modal-close = 关闭

export-modal-resolving = 正在解析…

export-modal-copy = 复制

reference-evidence-jump-to-mention = { $kind ->
    [explicit] 跳转到显式提及 { $index }
   *[plain] 跳转到纯文本提及 { $index }
    }

reference-evidence-open-this-block-in = 在 { $page } 中打开此块

reference-evidence-open-this-mention-in = 在 { $page } 中打开此提及

reference-evidence-open-mention-in = 在 { $page } 中打开提及 { $span }

reference-evidence-show-excerpt = 显示摘录

reference-evidence-show-full-block = 显示完整块

reference-export-chooser-copy-export = 复制 / 导出 { $subject }

reference-export-chooser-reference-selection = 引用选择

reference-export-chooser-all = 全部

reference-export-chooser-none = 无

reference-export-chooser-entries = { $subject } 条目

reference-export-chooser-no-entries-in-this-section = 此区域没有条目。

reference-export-chooser-cancel = 取消

reference-export-chooser-open = 复制 / 导出…

export-modal-block-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 个块
   *[other] { NUMBER($count, useGrouping: 0) } 个块
    }

reference-export-chooser-count = { $count } / { $total }

reference-evidence-mention-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 处提及
   *[other] { NUMBER($count, useGrouping: 0) } 处提及
    }

reference-evidence-occurrence-label = 跳转到第 { $index } / { $total } 处提及

# Helper-return and component-prop interface text (GH #107).
export-prose-plain-text-output-has-no-markup-markers-to-remove = 纯文本输出中没有需要移除的标记
export-prose-only-applies-to-plain-text-output = 仅适用于纯文本输出
