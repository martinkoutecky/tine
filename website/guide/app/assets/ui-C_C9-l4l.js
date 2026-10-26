const n=`ui-sync-conflicts = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 处新的同步冲突待检查
   *[other] { NUMBER($count, useGrouping: 0) } 处新的同步冲突待检查
    }

ui-review = 检查

ui-save-preference-failed = 无法保存{ $preference ->
    [keyboard] 键盘快捷键
   *[other] 显示偏好
    }。

ui-ui-could-not-load-display-preference = 无法加载显示偏好。

ui-ui-could-not-save-journal-title-format = 无法保存日记标题格式：{ $value1 }

ui-ui-could-not-check-for-duplicate-journal-days = 无法检查同一天的重复日记：{ $value1 }

ui-ui-could-not-enter-fullscreen-focus-mode = 无法进入全屏专注模式。

ui-ui-could-not-leave-fullscreen-focus-mode = 无法退出全屏专注模式。

ui-ui-could-not-check-some-sidebar-blocks-try-again-after-the = 无法检查部分侧边栏块，请在图谱加载后重试。

ui-ui-pdf-export-needs-the-desktop-app-a-mobile-webview-cannot = PDF 导出需要桌面应用：移动端 WebView 无法打印。

ui-conflict-policy-could-not-remember-always-ask = 无法保存“始终询问”：{ $value1 }

ui-conflict-policy-could-not-load-always-ask = 无法加载“始终询问”：{ $value1 }

ui-main-plugins-unavailable = 插件不可用：{ $value1 }

ui-main-community-extensions-unavailable = 社区扩展不可用：{ $value1 }
`;export{n as default};
