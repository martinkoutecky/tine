const e=`editor-references-linked-reference = { $count ->
    [one] { NUMBER($count, useGrouping: 0) } 条链接引用
   *[other] { NUMBER($count, useGrouping: 0) } 条链接引用
    }

editor-references-reference-page-groups = 引用页面分组

editor-references-collapse-all = 全部折叠

editor-references-expand-all = 全部展开

editor-references-references-from = { $action ->
    [expand] 展开来自 { $page } 的引用
   *[collapse] 折叠来自 { $page } 的引用
    }
`;export{e as default};
