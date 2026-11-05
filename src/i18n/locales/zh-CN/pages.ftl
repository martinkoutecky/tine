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

pages-all-open-full-list = 打开完整列表…

pages-all-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 个页面
   *[other] { NUMBER($count, useGrouping: 0) } 个页面
    }

pages-all-filtered-count = { NUMBER($count, useGrouping: 0) } / { NUMBER($total, useGrouping: 0) } 个页面

pages-all-open-more = 还有 { NUMBER($count, useGrouping: 0) } 个，打开完整列表…

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
rename-alias-choice-title = “{ $to }”是另一个页面的别名
rename-alias-choice-body = “{ $to }”是“{ $owner }”的别名。要把“{ $from }”合并到“{ $owner }”，还是让“{ $to }”成为独立页面？独立页面仍是“{ $owner }”的别名，因此指向“{ $to }”的链接仍会显示在“{ $owner }”的链接引用中。
rename-alias-choice-no-merge = 不提供合并：“{ $owner }”位于“{ $from }”之下，会随它一起移动。
rename-alias-choice-merge = 合并到“{ $owner }”
rename-alias-choice-own-page = 让“{ $to }”成为独立页面（仍是“{ $owner }”的别名）
rename-refused-own-child = 无法将“{ $old }”重命名为“{ $new }”：“{ $new }”已是“{ $old }”下的页面或别名，而“{ $old }”下的所有页面都会随之移动。
rename-refused-file-taken = 无法重命名：文件 { $file } 属于页面“{ $page }”。请先重命名或合并该页面。
rename-alias-choice-stale-what = 重命名
