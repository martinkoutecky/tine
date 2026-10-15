app-choose-no-to-review-retry-saving = { app-close-explanation }{ "\u000A" }{ "\u000A" }{ app-close-inventory }{ "\u000A" }{ "\u000A" }选择“否”可检查草稿、重试保存或复制内容。仍要关闭此窗口并丢失这些内容吗？

app-unsaved-changes = 未保存的更改

app-couldn-t-save-pending-pdf-changes-the-graph = 无法保存待处理的 PDF 更改。图谱仍保持打开。

app-still-saving-your-changes-closing-in = 仍在保存更改，稍后将关闭。

app-couldn-t-confirm-closing-the-window-your-unsaved = 无法确认关闭窗口。未保存的更改仍保留在窗口中。

app-tine-couldn-t-close-the-android-activity-tap = Tine 无法关闭 Android 活动。请点击返回键重试。

app-close-tab = 关闭标签页

app-pane-select = 选择窗格

app-move-onto-seams-edges = <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> 移动（可移到分隔线和边缘）· <kbd>Enter</kbd> 进入窗格 · <kbd>Del</kbd> 关闭窗格

app-ctrl-k-open-a-page-in-this-pane = <kbd>Ctrl+K</kbd> 在此窗格打开页面 · <kbd>Esc</kbd> 退出

app-captured-to = 已记录到“{ $title }”

app-captured-to-today-s-journal = 已记录到今天的日记

app-capture-couldn-t-be-saved-its-text = 无法保存快速记录，文本仍保留在快速记录窗口中

app-tine-was-renamed-under-the-hood-so = Tine 的内部名称已更改，我们已迁移你的设置和备份。少数应用偏好（如快捷键）可能需要重新设置，抱歉给你带来不便！

app-tine-could-not-write-its-usual-application-data = Tine 无法写入通常使用的应用数据文件夹，本次会话的设置和备份将保存在 { $fallbackDirectory }。修复原文件夹的权限后即可恢复正常位置。

app-quick-capture-could-not-connect-to-this = 快速记录无法连接到此图谱窗口。

app-the-block-renderer-failed-to-load = 块渲染器加载失败，文本将以未格式化形式显示。请重新加载 Tine；如问题持续，请报告。

app-finishing-graph-operation = 正在完成图谱操作…

app-close = 关闭

app-close-navigation-sidebar = 关闭导航侧边栏

app-journals = 日记

app-toggle-theme-t-t = 切换主题（t t）

app-toggle-right-sidebar-t-r = 切换右侧边栏（t r）

app-settings-t-s = 设置（t s）

app-refreshing-changes-from-disk = 正在读取磁盘上的更改…

app-exit-focus-esc = 退出专注模式（Esc）

app-close-explanation = { $reason ->
    [still-saving] Tine 仍在写入更改，耗时比预期更长。较慢的磁盘或网络驱动器可能导致这种情况。
   *[other] Tine 有无法保存的更改（存在冲突或保存停滞）。
    }

app-close-inventory = { $inventory ->
    [empty] 有待处理的附件或存储操作，未发现页面草稿。
   *[other] { $inventory }
    }

app-pane-split-instruction = { $kind ->
    [edge] <kbd>Enter</kbd> <span>拆分<em-hint>整个窗口</em-hint></span> · <em-hint>输入页面名称</em-hint>（或按 <kbd>Ctrl+K</kbd>）在新窗格打开页面
    [pane-edge] <kbd>Enter</kbd> <span>拆分<em-hint>此窗格</em-hint></span> · <em-hint>输入页面名称</em-hint>（或按 <kbd>Ctrl+K</kbd>）在新窗格打开页面
   *[seam] <kbd>Enter</kbd> <span>在此处拆分（复制窗格内容）</span> · <em-hint>输入页面名称</em-hint>（或按 <kbd>Ctrl+K</kbd>）在新窗格打开页面
    }

app-pane-split-navigation = { $kind ->
    [pane-edge] <span>再次向外移动可扩大拆分范围 · </span><kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> 移动 · <kbd>Esc</kbd> 退出
   *[other] <kbd>←</kbd><kbd>→</kbd><kbd>↑</kbd><kbd>↓</kbd> 移动 · <kbd>Esc</kbd> 退出
    }

# Helper-return and component-prop interface text (GH #107).
app-prose-navigation-sidebar = 导航侧边栏
app-prose-unexpected-failure = 发生意外错误。
