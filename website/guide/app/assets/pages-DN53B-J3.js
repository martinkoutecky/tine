const n=`pages-all-title = 所有页面

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
`;export{n as default};
