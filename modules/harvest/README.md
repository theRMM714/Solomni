# harvest

扫一个目录，把文本资料抽成**语料清单** `corpus.jsonl`：每行一个 JSON 对象，正文抽成纯文本，供后续模块消费。
只读原件、只写 `out` 指定的那一份清单；不联网、不采集、不改动原文件。

实现：单个 python 脚本 `tools/scan.py`，只用标准库。

## 工具

    harvest.scan    command: python tools/scan.py
    stdin: {"root":"<要扫的目录>","out":"<corpus.jsonl 路径>","include":"md,txt,html,json"}

- `root` / `out` 必填；`include` 是扩展名白名单，缺省 `md,txt,html,json`（`.markdown` 归 md，`.htm` 归 html）。
- 递归扫 `root`，**跳过**隐藏目录（`.` 开头）与 `userdata/`（模块自己的跨任务私有区）。
- 单份超过 8 MB、读不了、或不在白名单里的文件一律跳过，跳过的数量在 stdout 末尾如实报出，不静默丢。
- 失败走 stderr + 非零退出码（缺参数、`root` 不是目录、`out` 写不了）；没命中任何资料不是错误，如实报 0 份。
- stdout 只给产物路径 + 几行统计（模型看这个，不回显整份语料）。

## corpus.jsonl（跨模块契约）

每行一个对象（UTF-8、非 ASCII 原样、紧凑分隔符）；消费方是 `render` 与 `indexer`，改字段要同时改它们：

| 字段 | 含义 |
| --- | --- |
| `rel` | 相对 `root` 的路径，分隔符统一 `/` |
| `path` | 真实绝对路径（同样用 `/`） |
| `kind` | `md` / `txt` / `html` / `json` |
| `bytes` | 文件字节数 |
| `chars` | 抽出的正文码点数 |
| `lines` | 正文行数 |
| `headings` | 标题列表：markdown 取 `#`..`######` 的文字，HTML 取 `h1`..`h3`，其余为空 |
| `links` | 链接列表：markdown 取 `[文字](地址)` 的地址，HTML 取 `href`，其余为空 |
| `text` | 抽出的纯文本正文（HTML 去掉 `script`/`style`） |
