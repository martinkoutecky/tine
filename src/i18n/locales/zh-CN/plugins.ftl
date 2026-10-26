plugins-plugin-setting-could-not-be-saved = 无法保存插件设置：{ $error }

plugins-plugin-settings-could-not-be-reset = 无法重置插件设置：{ $error }

plugins-this-plugin-has-no-configurable-settings = 此插件没有可配置的设置。

plugins-reset = 重置

plugins-reset-all-settings = 重置所有设置

plugins-stored-on-this-device-only-plugin-settings = 仅保存在本设备上。插件设置不会写入图谱。

plugins-choose-both-manifest-json-and-the-plugin-s-wasm = 请同时选择 manifest.json 和插件的 .wasm 入口文件。

plugins-plugin-package-is-too-large-manifest = 插件包过大（清单 ≤ 64 KiB，.wasm ≤ 8 MiB）。

plugins-installed-disabled-review-it-then-enable = 已安装 { $name } { $version }，当前为禁用状态。请检查后在此启用。

plugins-plugin-installation-failed = 插件安装失败：{ $error }

plugins-plugin-could-not-be = { $enabled ->
    [disabled] 无法禁用插件：{ $error }
   *[enabled] 无法启用插件：{ $error }
    }

plugins-uninstall-this-removes-the-plugin-from = 卸载 { $name } { $version } 吗？{ "\u000A" }{ "\u000A" }此操作将从本设备移除插件，不会更改图谱或笔记。

plugins-uninstall-plugin = 卸载插件吗？

plugins-was-uninstalled = 已卸载 { $name } { $version }。

plugins-plugin-could-not-be-uninstalled = 无法卸载插件：{ $error }

plugins-installed-disabled-enable-it-after-reviewing = 已安装 { $name }，当前为禁用状态。请检查其权限后启用。

plugins-community-plugin-installation-failed = 社区插件安装失败：{ $error }

plugins-installed-plugins = ← 已安装插件

plugins-behavioral-port = 功能移植

plugins-source-derived-port = 基于源代码移植

plugins-from-for-original-authors = 移植自 { $ecosystem } 的 { $name }；原作者：{ $authors }。

plugins-original-source-at = 原始源代码版本：{ $revision }

plugins-settings = 设置

plugins-package = 软件包

plugins-details-screenshots = 详情与截图

plugins-uninstalling = 正在卸载…

plugins-uninstall = 卸载…

plugins-plugin-settings-sections = 插件设置区域

plugins-browse = 浏览

plugins-installed = 已安装（{ $count }）

plugins-experimental-plugin-platform = 实验性插件平台

plugins-tine-plugins-are-capability-limited-webassembly-guests-not = Tine 插件是权限受限的 WebAssembly 客体，并非 Logseq 或 Obsidian 插件。它们无法直接访问 DOM、Tauri、网络、文件、进程或图谱。插件安装后默认禁用，仅在声明的权限和入口通过验证后才能运行。

plugins-install-a-local-package = 安装本地软件包

plugins-select-its-manifest-json-and-wasm-file-together = 请同时选择 <code>manifest.json</code> 和 <code>.wasm</code> 文件。

plugins-validating = 正在验证…

plugins-choose-package = 选择软件包…

plugins-community-catalogue = 社区目录

plugins-signed-registry-automated-deterministic-and-no-tools = 签名注册表 · 自动化确定性审计与不使用工具的 AI 审计 · 不可变版本摘要。

plugins-showing-the-last-verified-offline-copy = 正在显示上次验证的离线副本。

plugins-installed-plugins-are-held-until-a-signed = 在签名目录通过验证前，已安装插件将暂停运行。

plugins-checking-the-signed-catalogue = 正在检查签名目录…

plugins-no-verified-catalogue-is-available = 没有可用的已验证目录。

plugins-retry = 重试

plugins-installed-status = 已安装

plugins-checking = 正在检查…

plugins-unavailable-on = 在 { $platform } 上不可用

plugins-verifying = 正在验证…

plugins-install = 安装

plugins-human-written = 人工编写

plugins-ai = AI-{ $developmentMethod }

plugins-checked = 检查日期：{ $date }

plugins-hide-safety-report = 隐藏安全报告

plugins-safety-report = 安全报告

plugins-verifying-the-signed-report = 正在验证签名报告…

plugins-the-report-could-not-be-fetched-and = 无法获取报告并验证摘要。

plugins-why-human-review-was-required = 为何需要人工审核

plugins-an-automated-check-found-behavior-that-tine = 自动检查发现了 Tine 要求在发布前由人工检查的行为。

plugins-this-plugin-can-edit-the-focused-block = 运行此插件的命令时，它可编辑聚焦的块。即使其他自动检查均通过，Tine 也要求所有可写入图谱的插件接受人工审核。

plugins-the-audit-also-caught-that-an-earlier = 审计还发现，早期草稿可能操作错误的聚焦块。发布的插件已将范围缩小到查询表格或看板块，并重新审核。

plugins-signed-review-record = 签名审核记录：{ $note }

plugins-severity-describes-possible-impact-not-reviewer-confidence = 严重程度描述潜在影响，而非审核者的信心。“低风险”表示问题范围有限，不太可能影响笔记；“信息”表示观察结果，而非已知危害。

plugins-raw-report = 原始报告

plugins-automated-review-is-evidence-not-a-guarantee = 自动审核提供参考依据，无法作出保证。

plugins-no-plugins-installed = 未安装任何插件。

plugins-open-settings = 设置…

plugins-details = 详情…

plugins-capabilities-list = { $kind ->
    [none] 权限：无
   *[some] 权限：{ $capabilities }
    }

plugins-audit-details = { $count ->
    [one] 源代码 <source-code>{ $source }</source-code> · 软件包 <package-code>{ $package }</package-code> · 报告 <report-code>{ $report }</report-code> · 已审核 { $count } 个区域
   *[other] 源代码 <source-code>{ $source }</source-code> · 软件包 <package-code>{ $package }</package-code> · 报告 <report-code>{ $report }</report-code> · 已审核 { $count } 个区域
    }

plugins-ui-could-not-cache-verified-plugin-registry = 无法缓存已验证的插件注册表：{ $value1 }

plugins-ui-could-not-cache-plugin-safety-report = 无法缓存插件安全报告。

# Helper-return and component-prop interface text (GH #107).
plugins-prose-information = 信息
plugins-prose-human-reviewed-before-publication = 发布前经过人工审核
plugins-prose-low-risk-automated-pass = 自动审核通过，风险较低
plugins-prose-automated-review-passed = 自动审核通过

plugins-prose-risk-finding = { $severity ->
        [low] 低风险项
        [medium] 中风险项
        [critical] 严重风险项
       *[high] 高风险项
    }
