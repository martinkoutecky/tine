# 简体中文翻译约定

本文件是译者说明，不参与消息加载。以自然、准确、易懂的产品界面中文为准；用户的图谱内容始终保留原文。

术语依据：Logseq 0.10.9 的 `src/resources/dicts/zh-cn.edn`（只读参考）。页面、块、日记、图谱、属性、标签、别名、命名空间、查询、模板、白板、插件、同步等沿用其用语。

| English | 中文 | 说明 |
| --- | --- | --- |
| page | 页面 | 用户页面名称不翻译 |
| block / child block / parent block | 块 / 子块 / 父块 | 沿用 `:help/block-reference` 的“块” |
| journal | 日记 | 沿用 `:right-side-bar/show-journals` |
| reference / block reference / page reference | 引用 / 块引用 / 页面引用 | 沿用 `:help/block-reference`、`:help/reference-autocomplete` |
| linked reference / unlinked reference | 链接引用 / 未链接引用 | 与 Logseq 的“块引用”“页面引用”保持一致；该词典没有对应的独立标题条目 |
| property | 属性 | 属性键及值作为图谱内容保留原文 |
| graph | 图谱 | 沿用 `:on-boarding/add-graph`；不译作数据库或知识库 |
| tag / alias / namespace | 标签 / 别名 / 命名空间 | 标签内容与别名不翻译 |
| query | 查询 | 查询关键字与语法不翻译 |
| embed | 嵌入 | 区分复制引用与复制嵌入 |
| template | 模板 | 模板变量与插入语法保留原文 |
| task marker | 任务标记 | TODO、DOING、DONE、NOW、LATER、WAITING、CANCELED 等保持字面值 |
| favorites | 收藏 | Logseq 的区域标题为“收藏页面”，操作为“添加收藏”；Tine 也可收藏块和分组，故标题缩为“收藏” |
| sidebar / right sidebar | 侧边栏 / 右侧边栏 | 沿用 Logseq |
| pane | 窗格 | Tine 的拆分区域；不同于侧边栏与独立窗口 |
| whiteboard | 白板 | 沿用 `:search-item/whiteboard` |
| flashcard | 记忆卡片 | 沿用 `:settings-page/enable-flashcards`，不使用“闪卡” |
| plugin | 插件 | 插件作者提供的文本不翻译 |
| sync / conflict / backup | 同步 / 冲突 / 备份 | 合并冲突与未保存草稿分别说明 |
| Direct Files | Direct Files（直接文件模式） | 产品模式名保留英文，说明中可加中文；不同于同步服务 |
| outline / outliner | 大纲 / 大纲笔记工具 | 短标签用“大纲”，介绍用“大纲笔记工具” |
| sheet / grid / table | 表格视图 / 网格 / 表格 | Tine 专有查询表格视图没有对应 Logseq 术语，避免“工作表”暗示电子表格文件 |
| trash / recoverable trash | 回收站 / 可恢复的回收站 | 文件路径 `.tine-trash` 保留原文 |
| snapshot | 快照 | 区分完整快照与逐页备份 |
| carry unfinished tasks | 延续未完成任务 | 沿用现有中文命令，明确移动与复制的差别 |
| focus mode | 专注模式 | 窗格 focus 动作用“聚焦” |

品牌词 `{ -brand }`、Tine、Logseq 及其他产品名保持原文。反引号、`<code>`、`<kbd>` 中的语法、文件名与键名不翻译。保留 Fluent 变量、引用、选择器、分支与标记标签；中文数量分支可用同一措辞，但仍保留英文分支结构。

正文用中文全角标点。为与已有中文目录一致，中英文之间保留空格（例如“自定义 CSS”“Tine 的菜单”），键位组合、代码和路径内部保持半角。简短按钮不加句号。说明面向用户，用“你”，避免逐字照搬英文。

可直接核对的词典条目还包括 `:page/open-backup-directory`（备份）、`:file-rn/filename-desc-4`（命名空间）、`:context-menu/make-a-template`（模板）、`:content/copy-block-ref`（块引用）、`:whiteboard/references`（引用）、`:settings-page/sync`（同步）。没有独立条目的词采用常见中文产品用语，不声称其为该词典的逐字翻译。

与 Logseq 的有意差异：favorites 用“收藏”（包含块和分组），pane 用“窗格”（Tine 拆分功能），sheet 用“表格视图”（Tine 查询显示模式），Direct Files 保留模式名（产品名边界），carry 用“延续”（沿用已发布中文命令）。这些不是修改图谱中的任何语法或名称。
