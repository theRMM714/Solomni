# render 模块

把 harvest 模块产出的 `corpus.jsonl` 渲染成一份**自包含、可离线打开**的 HTML 报告。
只消费语料、只写成品：不抓取网络、不改写语料、不补充语料之外的事实。

## 工具：report

```yaml
command: node tools/report.js
```

工具是独立可执行程序：参数从 **stdin** 收一个 JSON 对象，结果写 **stdout**，失败写 stderr 并以非零退出码结束。
进程的工作目录 = 本模块根目录，所以 `command` 与参数里的路径都按模块根写相对路径。
只用 node 标准库（`fs` / `path` / `readline`），没有任何第三方依赖。

### 参数

| 参数 | 必填 | 说明 |
| --- | --- | --- |
| `corpus` | 是 | `corpus.jsonl` 的路径（相对模块根） |
| `out` | 是 | HTML 产物路径（相对模块根） |
| `index` | 否 | 同时写一份轻量 JSON 清单 |
| `title` | 否 | 报告标题，默认 `资料报告` |

### 用法

```powershell
# args.json 里写参数对象，再用管道喂给 stdin（PowerShell）
Get-Content args.json | node modules/render/tools/report.js
```

`args.json` 示例：

```json
{
  "corpus": "target/test-scratch/corpus.jsonl",
  "out": "target/test-scratch/report.html",
  "index": "target/test-scratch/index.json",
  "title": "资料报告"
}
```

## 输入：corpus.jsonl

每行一个 JSON 对象、UTF-8、非 ASCII 不转义，字段见 `MODULE_SPEC.md` 与 harvest 模块：

```json
{"rel":"相对共享区的路径","path":"绝对路径","kind":"md|txt|html|json","bytes":N,"chars":N,"lines":N,"headings":["..."],"links":["..."],"text":"抽出的纯文本（保留换行）"}
```

解析方式：`fs.createReadStream` + `readline` 逐行流式处理，正文只保留前 200 字用于摘要，不把整份语料驻留内存。

## 输出

### out（HTML 报告）

单文件、内联样式，包含：

- **概览**：文档数、总字数、相对链接数、坏链数、跳过的行数；按 `kind` 的计数表 + 一张**内联 SVG** 条形图
- **目录**：每份资料一张卡片，含 `rel`、`kind`、chars/lines/bytes、标题大纲、正文前 200 字摘要
- **链接检查**：逐条坏链列出「来源文档 / 链接 / 解析目标」
- **生成时间**：`new Date().toISOString()` 得到的 ISO 8601 时间

### index（可选轻量 JSON 清单）

一个 JSON 数组，每项 `{"rel","kind","chars","headings"}`，便于其它模块或脚本快速消费。

## 无外部资源的保证

HTML 产物满足以下不变量（可离线双击打开）：

- 没有任何 `script` 元素、没有 `link` 元素、没有 `img` 元素
- 没有任何指向 http/https 的 `src` / `href`；产物里出现的 http/https 只可能落在**被转义后的链接文字**里
- 样式全部写在 `<style>` 里；条形图是内联 `<svg>`（不写 `xmlns`，因此产物里不出现任何 SVG 命名空间 URL）
- 语料内容一律先 `& < > " '` 全转义再进文本节点，`&lt;script&gt;` 这类片段不会被当成标签执行

## 链接检查规则

- 检查对象：**相对引用**（RFC 3986）——不以 `#` 开头、不带 URI scheme（`http:` / `https:` / `mailto:` / `data:` …）、不以 `//` 开头
- 文档内的 `#锚点` 一律不判坏；`#fragment` 与 `?query` 在比对前剥离
- 相对链接先按「所在文档的目录」解析，也接受直接以 `rel` 为目标的写法；两种写法都没命中语料里的 `rel` 才算坏链
- 坏链只报告、不猜测、不修正

## 退出码与错误

| 情况 | 行为 |
| --- | --- |
| 缺必填参数 / 参数不是非空字符串 / stdin 不是 JSON 对象 | stderr 报错（附用法），退出码 2 |
| 语料打不开 / 读失败 | stderr 报错，退出码 1 |
| 产物写不进（目录不存在、无权限） | stderr 报错，退出码 1 |
| 单行 JSON 非法或缺少 `rel` | 跳过该行，计数并如实报「跳过了 N 行」，退出码仍为 0 |

stdout 是给模型看的紧凑摘要（不超过 20 行）：文档数、总字数、按 kind、跳过的行数、坏链数量与前几条、产物路径。

## 手工验证

`target/test-scratch/` 下放一份 4~6 行的小 `corpus.jsonl`（含中文、一条故意指向不存在目标的相对链接、一段需要转义的 HTML 片段），然后：

```powershell
Get-Content target/test-scratch/args.json | node modules/render/tools/report.js
```

验证点：报告生成；坏链被揪出；HTML 里 http/https 只出现在链接文字里；`& < > " '` 被正确转义。
