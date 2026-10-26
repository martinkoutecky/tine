const n=`journal-sections-customize = 自定义今天的区域…
journal-sections-explanation = 此页面控制今天日记中显示的内容：查询块、日程和 { "{{upcoming}}" }。

journal-sections-create-failed = 无法创建页面“{ $page }”：{ $error }

journal-sections-ambiguous = 多个页面标记为 tine/journal-sections，将使用“{ $page }”。其他已标记页面：{ $others }。

journal-config-diagnostic = config.edn：{ $message }

journal-query-error = { $title }：{ $error }

journal-query-untitled = 查询

journal-agenda-title = 计划与截止

settings-sections-page = 今天的区域由页面“{ $page }”定义。请在该页面编辑日程查询。

settings-sections-open = 打开页面

journals-prose-tasks-today = 今天有 { $count } 项任务
journals-prose-in-progress = { $count } 项进行中
journals-prose-task-summary = { $tasks }，{ $progress }
`;export{n as default};
