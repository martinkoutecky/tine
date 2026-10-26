const n=`calendar-jump-go-to-date = 前往日期

calendar-jump-previous-month = 上个月

calendar-jump-next-month = 下个月

calendar-jump-has-an-entry = 有内容

calendar-jump-empty = 空白

calendar-jump-today = 今天

namespace-macro-label = 命名空间

namespace-hierarchy = 层级

quick-switcher-run-a-command = 运行命令…

quick-switcher-search-blocks-in-current-page = 搜索当前页面中的块…

quick-switcher-jump-to-page-search-or-run-a-command = 跳转到页面、搜索或运行命令…

quick-switcher-load-more-results = 加载更多结果…

quick-switcher-more-matches-exist-narrow-your-query-to-see = 还有更多匹配，请缩小查询范围以查看。

quick-switcher-searching = 正在搜索…

quick-switcher-no-matched-results = 没有匹配结果

quick-switcher-search-syntax = 搜索语法

quick-switcher-navigate = { " " }导航

quick-switcher-open = { " " }打开

quick-switcher-commands = { " " }命令

quick-switcher-open-search-tab = 打开搜索标签页

quick-switcher-journal = 日记

quick-switcher-page = 页面

quick-switcher-new = 新建

quick-switcher-cmd = 命令

right-sidebar-reference-sidebar = 引用侧边栏

right-sidebar-sidebar = 侧边栏

right-sidebar-sidebar-item-actions = 侧边栏条目操作

right-sidebar-collapse-all = 全部折叠

right-sidebar-expand-all = 全部展开

right-sidebar-close-all = 全部关闭

right-sidebar-close-sidebar-t-r = 关闭侧边栏（t r）

right-sidebar-nothing-open-shift-click-a-page-or-block = 尚未打开任何内容。按住 Shift 点击页面或块即可在此打开。

right-sidebar-more-actions = 更多操作

right-sidebar-close = 关闭

right-sidebar-close-others = 关闭其他

right-sidebar-collapse = 折叠

right-sidebar-expand = 展开

right-sidebar-collapse-others = 折叠其他

right-sidebar-expand-sidebar-item = 展开侧边栏条目

right-sidebar-collapse-sidebar-item = 折叠侧边栏条目

right-sidebar-on = 位于 { $page }

right-sidebar-this-block-is-no-longer-available = 此块已不可用。

search-result-row-block = 块

sidebar-tine = Tine

sidebar-journals = 日记

sidebar-favorites = 收藏

sidebar-recent = 最近

sidebar-namespaces = 命名空间

sidebar-new-page = + 新页面

sidebar-remove-from-this-list = 从此列表移除 { $name }

sidebar-open-graph = 打开图谱…

sidebar-new-graph = 新建图谱…

sidebar-review-the-pages-that-need-a-decision = 检查需要你决定如何处理的页面

sidebar-favorites-expand = 展开

sidebar-favorites-collapse = 折叠

sidebar-favorites-rename-group = 重命名分组 { $groupName }

sidebar-favorites-delete-this-group-what-it-holds-moves-up = 删除此分组（其中内容上移一级）

sidebar-favorites-delete-group = 删除分组 { $groupName }

sidebar-favorites-new-group = + 新分组

tab-bar-pinned-sticky-links-open-in-a-new-tab = 已固定：链接会在新标签页打开。双击取消固定

tab-bar-double-click-to-pin-sticky = 双击固定标签页

tab-bar-close-tab = 关闭标签页

tab-bar-show-all-tabs = 显示所有标签页

tab-bar-open-tabs = 已打开的标签页

tab-bar-reorder = 调整 { $title } 的顺序

tab-bar-drag-to-reorder-alt-up-down-moves-the = 拖动调整顺序；Alt+Up/Down 移动聚焦的标签页

tab-bar-pinned = 已固定

tab-bar-close = 关闭 { $title }

tagged-pages-some-tagged-pages-are-omitted-because-the-query = 已达到查询结果上限，部分带标签的页面未显示。

topbar-overflow-menu-more-toolbar-actions = 更多工具栏操作

topbar-overflow-menu-calendar = 日历

topbar-overflow-menu-journals = 日记

topbar-overflow-menu-toggle-theme = 切换主题

topbar-overflow-menu-toggle-right-sidebar = 切换右侧边栏

topbar-overflow-menu-back = 后退

topbar-overflow-menu-forward = 前进

window-chrome-minimize = 最小化

window-chrome-restore = 还原

window-chrome-maximize = 最大化

window-chrome-close = 关闭

workspace-switcher-delete-workspace = 删除工作区“{ $name }”吗？

workspace-switcher-workspace = 工作区：{ $name }

workspace-switcher-quick-switch-workspace = 快速切换工作区

workspace-switcher-workspaces = 工作区

workspace-switcher-rename = 重命名 { $name }

workspace-switcher-rename-action = 重命名

workspace-switcher-delete = 删除 { $name }

workspace-switcher-delete-action = 删除

workspace-switcher-new-workspace = + 新工作区

workspace-switcher-new-workspace-name = 新工作区名称

workspace-switcher-workspace-name = 工作区名称

workspace-switcher-create = 创建

workspace-switcher-save = 保存

workspace-window-shell-search-ctrl-k = 搜索（Ctrl+K）

workspace-window-shell-search = 搜索

workspace-window-shell-go-back = 后退

workspace-window-shell-go-forward = 前进

right-sidebar-open-as-page = 作为页面打开

quick-switcher-create-named-page = 创建页面：<strong>{ $name }</strong>

quick-switcher-alias = 别名 { $alias }

sidebar-conflict-count = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 处冲突
   *[other] { NUMBER($count, useGrouping: 0) } 处冲突
    }

tagged-pages-heading = { " " }带有“{ $name }”标签的页面

# Helper-return and component-prop interface text (GH #107).
navigation-prose-no-results = 没有结果
navigation-prose-no-graph = 没有图谱
navigation-prose-journals = 日记
navigation-prose-conflicts = 冲突
navigation-prose-all-pages = 所有页面
navigation-prose-search = 搜索
navigation-prose-guide = 指南

navigation-prose-search-source = 搜索：{ $source }
`;export{n as default};
