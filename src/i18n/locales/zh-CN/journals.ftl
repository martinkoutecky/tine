journal-sections-customize = 自定义今天的区域…
journal-sections-explanation = 此页面控制今天日记中显示的内容：查询块、日程和 { "{{upcoming}}" }。

journal-sections-create-failed = 无法创建页面“{ $page }”：{ $error }

journal-sections-ambiguous = 多个页面标记为 tine.journal-sections，将使用“{ $page }”。其他已标记页面：{ $others }。

journal-config-diagnostic = config.edn：{ $message }

journal-query-error = { $title }：{ $error }

journal-query-untitled = 查询

journal-agenda-title = 计划与截止

settings-sections-page = 今天的区域由页面“{ $page }”定义。请在该页面编辑日程查询。

settings-sections-open = 打开页面

journals-prose-tasks-today = 今天有 { $count } 项任务
journals-prose-in-progress = { $count } 项进行中
journals-prose-task-summary = { $tasks }，{ $progress }

journal-title-format-refused-missing-year = 日记标题使用“{ $effective }”，而不是 config.edn 中的“{ $configured }”：该格式不写年份，一个标题会指向每一年的同一日期。你的文件没有改动。

journal-title-format-refused-missing-month = 日记标题使用“{ $effective }”，而不是 config.edn 中的“{ $configured }”：该格式不写月份，一个标题会指向多个日期。你的文件没有改动。

journal-title-format-refused-missing-day = 日记标题使用“{ $effective }”，而不是 config.edn 中的“{ $configured }”：该格式不写日，一个标题会指向多个日期。你的文件没有改动。

journal-title-format-refused-misread = 日记标题使用“{ $effective }”，而不是 config.edn 中的“{ $configured }”：该格式把 { $day } 写成“{ $written }”，而这个标题打开的是 { $readAs }。你的文件没有改动。

journal-title-format-refused-unread = 日记标题使用“{ $effective }”，而不是 config.edn 中的“{ $configured }”：该格式把 { $day } 写成“{ $written }”，而这个标题不打开任何日记日期。你的文件没有改动。
