improve-tine-s-parser-lsdoc-is-a-from-scratch-reimplementation = Tine 的解析器（<strong>lsdoc</strong>）是对 Logseq 解析器（<strong>mldoc</strong>）的全新实现。此工具会完全在本机上，用两者解析<strong>你的</strong>图谱，找出结果不一致的地方，帮助 Tine 像 Logseq 一样呈现你的笔记。

improve-privacy-nothing-is-uploaded-every-divergence-snippet = <strong>隐私：</strong>不会上传任何内容。下方每个差异片段都经过<strong>匿名化</strong>（替换页面名和词语，保留 URL 协议但去除主机和路径），并<strong>重新验证</strong>其仍能复现问题。若隐私处理后差异不再出现，Tine 会省略该结果。决定分享前，请仍仔细检查每个片段。

improve-what-to-run = 运行内容

improve-both = 两者

improve-divergences = 解析差异

improve-speed = 速度

improve-include-journals = 包含日记

improve-fast-scan = 快速扫描

improve-faster-but-can-miss-or-invent-divergences = 速度更快，但可能漏报或误报差异；仅显示已确认的结果。关闭后进行完整、可靠的扫描。

improve-running = 正在运行…

improve-run-comparison = 运行对比

improve-failed = 失败：{ $error }

improve-skipped-unreadable-or-oversized-file-s = { $count ->
    [one] 已跳过 { $count } 个无法读取或过大的文件。
   *[other] 已跳过 { $count } 个无法读取或过大的文件。
    }

improve-divergence-detection-needs-the-next-lsdoc-build = 差异检测需要下一版 lsdoc（0.3.4）。此构建暂时无法在应用内用 lsdoc 解析完整文件。下方显示 Logseq 解析器的速度，更新后将启用 lsdoc 列和差异扫描。

improve-parse-speed-best-of-3 = 解析速度（3 次中最快）

improve-parser = 解析器

improve-files = 文件

improve-total = 总计

improve-p50 = p50

improve-p95 = p95

improve-max = 最大值

improve-divergences-count = 差异（{ $count }）

improve-copied = 已复制！

improve-copy-failed = 复制失败

improve-copy-all = 复制全部

improve-no-actionable-divergences-lsdoc-matched-logseq-s = 没有可处理的解析差异。排除已确认的 mldoc 基准异常后，lsdoc 在所有文件上的结果均与 Logseq 解析器一致。

improve-lines = 第 { $lineStart }-{ $lineEnd } 行

improve-found-but-couldn-t-be-anonymized-not = 发现了差异，但无法匿名化，因此不显示。

improve-anonymized-via = 匿名化方式：{ $tier }

improve-copy = 复制

improve-suppressed-mldoc-oracle-artifact-s = 已排除 { $count } 项 mldoc 基准异常

improve-divergence-location = <code>{ $path }</code> · 第 { $lineStart }-{ $lineEnd } 行 — { $detail }

improve-known-intentional-parser-difference-s-suppressed = { $count ->
    [one] 已排除 { $count } 项已知的有意解析差异
   *[other] 已排除 { $count } 项已知的有意解析差异
    }

improve-non-divergence-issue-s = 其他问题：{ $count } 项

improve-post-these-anonymized-snippets-to = 将这些匿名化片段发布到 <a>{ $issuesUrl }</a>

improve-pending-next-build = 等待下一构建版本

improve-ms = { $milliseconds } ms

improve-scan-summary = { $files ->
    [one] { $product } { $tineVersion } · lsdoc { $lsdocVersion } · 已扫描 { $files } 个文件，{ $bytes }。
   *[other] { $product } { $tineVersion } · lsdoc { $lsdocVersion } · 已扫描 { $files } 个文件，{ $bytes }。
    }

improve-phase = { $phase ->
    [scan] 正在扫描
    [verify] 正在验证
   *[bench] 正在计时
    }
