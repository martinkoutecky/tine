pages-all-title = 所有页面

page-icon-set = 设置页面图标…

page-icon-remove = 移除页面图标

page-icon-label = 页面图标

page-icon-save = 保存图标

page-icon-hint = 输入表情或短文本。Windows：Win + .；macOS：Control + Command + Space。

page-icon-stale = 页面已更改，请重新打开页面操作菜单以编辑图标。

pages-all-filter = 筛选页面名称

pages-all-show-journals = 显示日记

pages-all-loading = 正在加载…

pages-all-column-page = 页面

pages-all-column-updated = 更新时间

pages-all-column-journal-date = 日记日期

pages-all-no-matches = 没有匹配的页面名称。

pages-all-empty = 尚无页面。

pages-all-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 个页面
   *[other] { NUMBER($count, useGrouping: 0) } 个页面
    }

pages-all-filtered-count = { NUMBER($count, useGrouping: 0) } / { NUMBER($total, useGrouping: 0) } 个页面

config-query-title = 查询

config-query-unsupported = Tine 暂时无法运行 config.edn 中的此查询。

config-query-details = 技术详情

sidebar-namespaces-title = 命名空间

pages-namespaces-open-full-view = 打开完整视图…

pages-namespaces-filter = 筛选命名空间和页面

pages-namespaces-expand-all = 全部展开

pages-namespaces-collapse-all = 全部折叠

pages-namespaces-column-namespace = 命名空间

pages-namespaces-column-pages = 页面

pages-namespaces-no-matches = 没有匹配的命名空间或页面名称。

pages-namespaces-empty = 尚无命名空间页面。将页面命名为 Project/Roadmap 这样即可创建。

pages-namespaces-expand = 展开 { $name }

pages-namespaces-collapse = 折叠 { $name }

pages-namespaces-matches = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 个匹配
   *[other] { NUMBER($count, useGrouping: 0) } 个匹配
    }

pages-namespaces-count = { NUMBER($pages, useGrouping: 0) } 个页面，{ NUMBER($namespaces, useGrouping: 0) } 个命名空间
rename-refused-own-child = 无法将“{ $old }”重命名为“{ $new }”：“{ $new }”已是“{ $old }”下的页面或别名，而“{ $old }”下的所有页面都会随之移动。
rename-refused-file-taken = 无法重命名：文件 { $file } 属于页面“{ $page }”。请先重命名或合并该页面。
rename-refused-not-a-page = 无法重命名“{ $name }”：没有页面或别名使用这个名称。
rename-refused-day = 无法重命名“{ $name }”：这是一个日记日期，日期不会改名。
rename-refused-alias-held = 无法重命名：“{ $alias }”是页面“{ $page }”的别名。请换一个名称，或先从“{ $page }”中移除该别名。
rename-refused-conflict-markers = 无法重命名：{ $file } 含有未解决的冲突标记，移动它需要修改内容。请先解决冲突。
rename-refused-capture = 无法重命名：“{ $name }”会变成“{ $image }”，而它已是页面“{ $page }”。请先重命名或合并该页面。
rename-refused-contested = 无法重命名：多个文件都声明了“{ $name }”（{ $files }）。请先将其中一个合并到另一个，或移除其中一个。
rename-refused-stale-merge = 无法合并：{ $file } 已不再使用你选择的名称。请重新开始重命名，选择要保留的页面。
rename-refused-unindexed = 无法重命名或合并：Tine 无法安全读取 { $file }。请在其他编辑器中修复其编码或简化文件，再重试。
rename-refused-read-only-org = 无法重命名或合并：Tine 无法在不改变内容的情况下保存 { $file }。请在其他 Org 编辑器中编辑它，或将笔记移到新页面，再重试。
rename-refused-different-formats = 无法合并：{ $source } 和 { $target } 使用不同格式。请先将笔记移到格式相同的页面，再重试。
rename-refused-merge-conflict-markers = 无法合并：请先解决 { $file } 中的冲突标记，再重试。未合并任何页面。
rename-refused-same-file = 无法合并：{ $file } 同时是来源文件和要保留的文件。请选择两个不同的文件。
rename-refused-stale-source = 无法重命名：{ $file } 已不再对应你选择的页面。请重新打开页面，再开始重命名。
rename-refused-title-identity = 无法重命名或合并：修改 { $file } 标题中的引用会让页面使用另一个名称。请先编辑标题，让其中的引用不再指向正在重命名的页面，再重试。未重命名或合并任何页面。
rename-refused-body-layout = 无法合并：合并文件时，Tine 无法保留 { $file } 的块结构。请在该文件中将顶层块与左边缘对齐，保存后重试。未合并任何页面。
rename-refused-indented-body = 无法合并：{ $file } 的第一个块有缩进，因此合并文件时 Tine 无法将其块保留在顶层。请在该文件中将顶层块与左边缘对齐，保存后重试。未合并任何页面。
rename-refused-directive-escape = 无法合并：{ $file } 在引用块之外含有 #+END_QUOTE 行，会提前结束合并页面中的引用块。请在该文件中将此行移入引用块或删除此行，然后重试。未合并任何页面。
rename-refused-alias-spelling = 无法合并：Tine 无法在保留原有写法的情况下移动 { $file } 中的别名声明“{ $alias }”。请在该文件中将该声明改为用逗号分隔的普通别名名称，保存后重试。未合并任何页面。
rename-refused-duplicate-files = 无法重命名：这些文件声明了同一个页面：{ $files }。请先将其中一个合并到另一个，或移除其中一个，再重试。
