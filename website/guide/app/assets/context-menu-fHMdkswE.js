const n=`context-menu-include-parent-block = 包含父块

context-menu-file-open-failed = { $action ->
    [show] 无法显示页面文件。（{ $error }）
   *[open] 无法打开页面文件。（{ $error }）
    }

context-menu-file-not-saved = 此页面尚无磁盘文件。请先输入内容，等待 Tine 保存。

context-menu-child-view = { $active ->
    [yes] { "✓ " }
   *[no] { "" }
    }{ $view ->
    [grid] 网格
    [table] 表格
   *[outline] 大纲
    }

context-menu-delete-page-label = { $kind ->
    [journal] 删除日记
   *[page] 删除页面
    }

context-menu-ui-couldn-t-copy-clipboard-write-failed = 无法复制：写入剪贴板失败。

context-menu-ui-page-actions = 页面操作

context-menu-ui-show-children-as = 子块显示方式 →

context-menu-ui-automatic-heading = 自动标题

context-menu-ui-auto = 自动

context-menu-ui-heading = 标题 { $h }

context-menu-ui-remove-heading = 移除标题

context-menu-ui-no-background = 无背景

context-menu-ui-add-child-bullet = 添加子块

context-menu-ui-zoom-into-cell = 聚焦单元格

context-menu-ui-delete-row = 删除行

context-menu-ui-delete-column = 删除列

context-menu-ui-open-as-full-page = 作为完整页面打开

context-menu-ui-add-formula = 添加公式…

context-menu-ui-edit-filter = 编辑筛选条件…

context-menu-ui-group-by = 分组依据 →

context-menu-ui-no-grouping = { $choice1 ->
    [yes] { "✓ " }
   *[no] { "" }
    }不分组

context-menu-ui-no-structural-actions = 无结构操作

context-menu-ui-convert-to-pipe-table = 转换为竖线表格

context-menu-ui-hierarchify-into-columns = 按列建立层级

context-menu-ui-hierarchify-by = 建立层级的依据 →

context-menu-ui-flatten = 展平

context-menu-ui-open-in-sidebar = 在侧边栏打开

context-menu-ui-go-to-block = 前往块

context-menu-ui-copy-link = 复制链接
context-menu-ui-copy-as = 复制为

context-menu-ui-copy-block-ref = 复制块引用

context-menu-ui-copied-block-ref = 已复制块引用

context-menu-ui-copy-block-embed = 复制块嵌入

context-menu-ui-copied-block-embed = 已复制块嵌入

context-menu-ui-a-template-named-already-exists = 名为“{ $title }”的模板已存在。

context-menu-ui-template-created = 已创建模板“{ $title }”。

context-menu-ui-make-a-template = 创建模板…

context-menu-ui-template-name = 模板名称

context-menu-ui-create-template = 创建模板

context-menu-ui-tip-in-the-template-expand-when-it-s-inserted = 提示：模板中的 <code><% today %></code>、<code><% current page %></code>、<code><% date: +3d %></code> 会在插入时展开（也可使用 <code>/Template var</code>）。

context-menu-ui-this-page-target-changed-reopen-the-page-actions-menu = 目标页面已更改，请重新打开页面操作菜单。

context-menu-ui-couldn-t-save-its-on-disk-file-was-not = 无法保存“{ $name }”：尚未打开其磁盘文件。

context-menu-ui-has-an-unresolved-save-conflict-this-is-the-file = “{ $name }”有未解决的保存冲突。此处显示的是磁盘文件的当前内容，未保存的更改会保留在 Tine 中，直到你解决冲突。

context-menu-ui-delete-the-file-moves-to-the-graph-s-tine = { $unsaved ->
    [discard] 删除“{ $name }”吗？文件将移入图谱的 .tine-trash 文件夹。未保存的更改也会被丢弃。
   *[none] 删除“{ $name }”吗？文件将移入图谱的 .tine-trash 文件夹。
    }

context-menu-ui-deleted = 已删除“{ $name }”

context-menu-ui-open = 打开

context-menu-ui-open-in-new-tab = 在新标签页打开

context-menu-ui-open-in-new-window = 在新窗口打开

context-menu-ui-remove-from-favorites = 取消收藏

context-menu-ui-add-to-favorites = 添加收藏

context-menu-ui-copy-page-ref = 复制页面引用

context-menu-ui-copied-page-ref = 已复制页面引用

context-menu-ui-copy-export-as = 复制 / 导出为…

context-menu-ui-copy-page-as-markdown = 将页面复制为 Markdown

context-menu-ui-copied-page-as-markdown = 已将页面复制为 Markdown

context-menu-ui-couldn-t-copy-page-as-markdown = 无法将页面复制为 Markdown。

context-menu-ui-export-to-pdf = 导出为 PDF…

context-menu-ui-show-in-folder = 在文件夹中显示

context-menu-ui-open-with-default-app = 用默认应用打开

context-menu-ui-find-in-page = 在页面中查找

context-menu-ui-page-properties = 页面属性…

context-menu-ui-carry-unfinished-tasks-today = 将未完成任务延续到今天

context-menu-rename-success = { $outcome ->
    [merged] 已合并到“{ $next }”
   *[renamed] 已重命名为“{ $next }”
    }

context-menu-ui-rename-failed = 重命名失败：{ $value1 }

context-menu-ui-rename-page = 重命名页面…

context-menu-ui-copied = { $count ->
    [one] 已复制块
   *[other] 已复制块
    }

context-menu-ui-zoom-into-block = 聚焦块

context-menu-ui-copy = { $count ->
    [one] 复制块
   *[other] 复制块
    }

context-menu-ui-properties = 属性…

context-menu-ui-insert-block-above = 在上方插入块

context-menu-ui-copy-block-refs = 复制块引用

context-menu-ui-copy-block-embeds = 复制块嵌入

context-menu-ui-convert-to-grid = 转换为网格

context-menu-ui-cut = { $count ->
    [one] 剪切块
   *[other] 剪切块
    }

context-menu-ui-couldn-t-cut-clipboard-write-failed = { $count ->
    [one] 无法剪切块：写入剪贴板失败。
   *[other] 无法剪切块：写入剪贴板失败。
    }

context-menu-ui-remove-numbered-list = 移除有序列表

context-menu-ui-numbered-list = 有序列表

context-menu-ui-collapse-all = 全部折叠

context-menu-ui-expand-all = 全部展开

context-menu-ui-used-for-new-journals = ✓ 用于新日记

context-menu-ui-use-for-new-journals = 用于新日记

context-menu-ui-delete = { $count ->
    [one] 删除块
   *[other] 删除块
    }
`;export{n as default};
