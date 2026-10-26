const n=`about-copy-version = 复制版本信息

about-copied = 已复制！

about-copy-failed = 复制失败

about-version-label = { $build ->
    [development] { $product } 开发版
   *[other] { $product } { $version }
    }

about-update-current = 你正在使用最新版本（{ $version }）。

about-update-available = { $mode ->
    [manual] { $product } { $version } 已发布，请在通知中选择“手动下载”。
   *[automatic] { $product } { $version } 已发布，请在通知中选择“安装更新”。
    }

about-update-unavailable = 暂时无法检查更新，请查看发布页面。

about-a-fast-local-first-logseq-compatible-outliner = 快速、本地优先且兼容 Logseq 的大纲笔记工具。

about-development-build = 开发版

about-checking = 正在检查…

about-check-for-updates = 检查更新

about-check-for-updates-automatically = 自动检查更新

about-applies-to-this-device-manual-checks-remain = 仅适用于本设备。关闭后仍可手动检查更新。

about-updates-arrive-through-your-app-s-distribution-channel = 更新通过应用的分发渠道提供。

about-releases = 发布版本

about-built = 构建时间：{ $buildStamp }

about-website = 网站

about-source-code = 源代码

about-support-tine = 支持 Tine

about-changelog = 更新日志

about-report-an-issue = 报告问题

about-privacy = 隐私

about-email-support = 邮件支持

about-license-agpl-3-0-only = 许可证：AGPL-3.0-only

about-direction-design-and-authorship = 方向、设计与创作

about-engineering-and-analysis = 工程与分析
`;export{n as default};
