## Today's journal sections (GH #605): config.edn queries, the agenda, the sections page
journal-sections-customize = Customize today's sections…
journal-sections-explanation = This page controls what shows with today's journal: query blocks, the agenda, { "{{upcoming}}" }.
# $page (string): the page Tine tried to create; $error (string): why it failed.
journal-sections-create-failed = Couldn't create the page "{ $page }": { $error }
# $page (string): the sections page in use; $others (string): the other marked pages, comma-separated.
journal-sections-ambiguous = Several pages are marked tine/journal-sections; using "{ $page }". Also marked: { $others }.
# $message (string): a config.edn diagnostic, in English from the config reader.
journal-config-diagnostic = config.edn: { $message }
# $title (string): the query's title; $error (string): why it failed.
journal-query-error = { $title }: { $error }
journal-query-untitled = Query
journal-agenda-title = Scheduled & Deadline
# $page (string): the sections page.
settings-sections-page = Today's sections are defined on the page "{ $page }". Edit the agenda query there.
settings-sections-open = Open page

journals-prose-tasks-today = { NUMBER($count, useGrouping: 0) } { $count ->
        [one] task
       *[other] tasks
    } today
journals-prose-in-progress = { NUMBER($count, useGrouping: 0) } in progress
journals-prose-task-summary = { $tasks }, { $progress }
