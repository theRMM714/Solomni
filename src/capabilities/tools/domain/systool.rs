//! 核心自带的内置工具：read / write / edit / search。
//! 策略在 conductor（名字固定、放行、寻址、根内校验、参数校验、改动前的"读过"证据、回执文案）；机制在 SysIo 端口（适配层）。
//! **参数契约不在本文件里**：声明在 systools/tools.yaml 的 tools，本文件只按声明校验、取缺省值与拼回执。
//! 存在的理由：读盘落盘不经过任何外部进程，编码问题不进本程序——模型自己看内容自己决定。
//! 路径一律是真实绝对路径（根目录经提示词册如实告知）；模块声明的外部工具与内置工具用同一套路径。

use crate::capabilities::prompt::api::{Prompt, Segment, ToolTexts};
use crate::capabilities::tools::api::{ArgFault, ToolSchema};
use crate::capabilities::workspace::api::Sandbox;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 一次工具执行结果的事实类型：**归 kernel 共享**——内置、模块与核心自有工具同一形状（R6）。
pub use crate::kernel::api::ToolOutcome;

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const EDIT: &str = "edit";
pub const PATCH: &str = "patch";
pub const LIST: &str = "list";
pub const SEARCH: &str = "search";

/// 单次读取回传的字符上限（超出如实截断，并在回执里给出接着读的 offset）。
pub const MAX_READ_CHARS: usize = 60_000;
/// 回执里单行的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_READ_LINE_CHARS: usize = 2_000;
/// 单次 search 回传的命中行数上限（超出如实截断；命中总数照实报）。
pub const MAX_SEARCH_HITS: usize = 200;
/// 单条命中行回传的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_SEARCH_LINE_CHARS: usize = 300;

/// 写进模块目录的标记：回执里带上它，给模型看；工具轨迹据此给用户一句可见提示（同一常量，两处共用）。
pub const MODULE_WRITE_MARK: &str = "[模块目录]";

/// 执行席的**回报**工具：不碰文件，只把"做完了什么"承载成一次工具调用。
/// 为什么是工具而不是正文 JSON：回报会驱动核心（判节点完成），属于核心操作（见 tools-and-roles.md）。
pub const REPORT: &str = "submit_report";

/// 内置工具名（保留名）。
/// 与 systools/tools.yaml 的 tools 是同一份名单，测试「builtin_tool_book_is_the_one_source_of_names_and_paths」锁死两者一致。
pub fn is_builtin(name: &str) -> bool {
    name == READ
        || name == WRITE
        || name == EDIT
        || name == PATCH
        || name == LIST
        || name == SEARCH
        || name == REPORT
}

/// 内置工具名清单（拼错误提示用）。
pub fn names() -> Vec<String> {
    vec![
        READ.to_string(),
        WRITE.to_string(),
        EDIT.to_string(),
        PATCH.to_string(),
        LIST.to_string(),
        SEARCH.to_string(),
        REPORT.to_string(),
    ]
}

/// patch 在**原生通道**上的声明：参数只有一个 body。
/// 原生协议要求参数是 JSON 对象，所以补丁正文当字符串值传——转义交给供应商的解码器，
/// 模型不必自己写转义（这正是原生通道相对手写信封的收益）。
pub fn patch_decl() -> crate::capabilities::llm::api::ToolDecl {
    crate::capabilities::llm::api::ToolDecl {
        name: PATCH.to_string(),
        description:
            "用一段补丁文本改文件（*** Add File: 路径 / *** Update File: 路径 … *** End File）"
                .to_string(),
        parameters: serde_json::json!({
            "type": "object",
            "properties": {
                "body": { "type": "string", "description": "补丁正文（原样写，不用转义换行）" }
            },
            "required": ["body"],
            "additionalProperties": false,
        }),
    }
}

/// 自由格式工具：输入不是 JSON 参数，而是**信封之后的那段原样文本**（不必转义）。
/// 存在的理由：把大段内容塞进 JSON 字符串要转义换行/引号，是真实会话里反复出事的点。
pub fn is_freeform(name: &str) -> bool {
    name == PATCH
}

/// 工具说明块的**素材**（装配期算一次，随回合注入）：patch 语法、模块工具清单、模块工具参数。
/// 为什么在这里算：它们只与这个 agent 的沙箱与模块有关、与回合无关；而回合执行路径上拿不到提示词册。
#[derive(Debug, Clone, Default)]
pub struct ToolNotes {
    pub patch_guide: String,
    pub module_tools: String,
    pub module_tool_params: String,
}

pub fn tool_notes(
    prompt: &dyn Prompt,
    sb: &Sandbox,
    modules: &[crate::capabilities::workspace::api::Module],
) -> ToolNotes {
    ToolNotes {
        patch_guide: prompt.render(
            Segment::PatchGuide,
            &[
                ("work_root", crate::kernel::api::slash(&sb.shared)),
                ("sandbox_root", crate::kernel::api::slash(&sb.private)),
            ],
        ),
        module_tools: crate::capabilities::tools::domain::module_tools::module_tools(
            prompt, modules,
        ),
        module_tool_params: crate::capabilities::tools::domain::module_tools::module_tool_params(
            prompt, modules,
        ),
    }
}

/// 观察账本：**本次会话里核心见过哪些文件的什么内容**。
/// 它存在的唯一理由：整份覆盖（write）会丢掉没读到的内容，所以要求"先完整读过、且读后没被改过"。
/// 语义边界：账本随会话（AgentSession）保存；回档（rewind）清空——转录里那段读取证据被截掉了，证据随之作废；
/// 重启后按落盘重建的会话不带账本（从零开始，模型重新读一遍即可）。edit 不需要账本：old_string 本身就是要改的那段原文。
#[derive(Debug, Clone, Default)]
pub struct Observations {
    seen: BTreeMap<PathBuf, Seen>,
}

#[derive(Debug, Clone)]
enum Seen {
    /// 完整读过（或由核心写入）：当时的内容指纹。
    Known(u64),
    /// 只读到一部分（原因文案）：整份覆盖会被拒。
    Partial(String),
}

impl Observations {
    /// 回档（删除模式）时清空：转录里那段读取证据已经不存在了。
    pub fn clear(&mut self) {
        self.seen.clear();
    }

    /// 记一次读取：完整读到 = 记指纹（可覆盖）；不完整 = 记下"只读到一部分"的原因。
    /// 不完整的读取**不覆盖**已有的完整记录（读到一半不会让已知变成未知）。
    pub(crate) fn note_read(&mut self, path: &Path, complete: bool, why: String, hash: u64) {
        if complete {
            self.seen.insert(path.to_path_buf(), Seen::Known(hash));
        } else {
            self.seen
                .entry(path.to_path_buf())
                .or_insert(Seen::Partial(why));
        }
    }

    /// 记一次写入：文件内容由核心产生，所以核心确切知道它现在是什么。
    pub(crate) fn note_written(&mut self, path: &Path, hash: u64) {
        self.seen.insert(path.to_path_buf(), Seen::Known(hash));
    }

    /// 合并一个**并发分支**的账本（分支里只跑声明可并发的只读工具）。
    /// 规则与逐个记账一致：完整读到记为所见、部分读到只补空白（读到一半不会让已知变成未知）。
    /// 调用方**必须按原始调用顺序**合并——那样并发批次与串行执行的结果完全相同。
    pub fn absorb(&mut self, branch: &Observations) {
        for (path, seen) in &branch.seen {
            match seen {
                Seen::Known(h) => {
                    self.seen.insert(path.clone(), Seen::Known(*h));
                }
                Seen::Partial(why) => {
                    self.seen
                        .entry(path.clone())
                        .or_insert(Seen::Partial(why.clone()));
                }
            }
        }
    }

    fn known(&self, path: &Path) -> Option<u64> {
        match self.seen.get(path) {
            Some(Seen::Known(h)) => Some(*h),
            _ => None,
        }
    }

    fn partial_why(&self, path: &Path) -> Option<String> {
        match self.seen.get(path) {
            Some(Seen::Partial(w)) => Some(w.clone()),
            _ => None,
        }
    }
}

/// 内容指纹（FNV-1a 64 位）：只用来判"读过的和现在的是不是同一份"，不做安全用途。
pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 字节位置对应的行号（从 1 数起）。
pub(crate) fn line_of(s: &str, at: usize) -> usize {
    s[..at].matches('\n').count() + 1
}

/// 字面匹配的全部命中位置（不重叠）。
pub(crate) fn find_all(hay: &str, needle: &str) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if needle.is_empty() {
        return out;
    }
    let mut from = 0usize;
    while let Some(i) = hay[from..].find(needle) {
        let at = from + i;
        out.push(at);
        from = at + needle.len();
    }
    out
}

/// "是不是只差空白"：按行比较（每行内空白折叠）。命中则给（行号, 该行原文）——模型据此改对缩进。
pub(crate) fn near_match(hay: &str, old: &str) -> Option<(usize, String)> {
    fn flat(line: &str) -> String {
        line.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    let want: Vec<String> = old.lines().map(flat).collect();
    if want.is_empty() {
        return None;
    }
    let lines: Vec<&str> = hay.lines().collect();
    if want.len() > lines.len() {
        return None;
    }
    for i in 0..=(lines.len() - want.len()) {
        let got: Vec<String> = lines[i..i + want.len()].iter().map(|l| flat(l)).collect();
        if got == want {
            return Some((i + 1, lines[i].to_string()));
        }
    }
    None
}

/// 整份覆盖已存在文件的证据检查：本次会话里**完整读过**它、且读后没被改过（write 与 patch 的 Add 共用）。
pub(crate) fn overwrite_check(
    texts: &ToolTexts,
    obs: &Observations,
    spec: &str,
    path: &Path,
    current: &str,
) -> Result<(), String> {
    match obs.known(path) {
        Some(seen) if seen == fnv1a(current) => Ok(()),
        Some(_) => Err(texts.render(&texts.write_stale, &[("path", spec.to_string())])),
        None => Err(match obs.partial_why(path) {
            Some(w) => texts.render(
                &texts.write_partial,
                &[("path", spec.to_string()), ("why", w)],
            ),
            None => texts.render(&texts.write_need_read, &[("path", spec.to_string())]),
        }),
    }
}

/// 整份覆盖已有文件时，内容按原文件的行尾风格写（免得把 CRLF 文件改成 LF）。
pub(crate) fn align_eol(content: &str, current: &str) -> String {
    if current.contains("\r\n") {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    }
}

/// 某一块不成立的回执（整体不写盘，所以措辞要直说这一点）。
pub(crate) fn block_fault(texts: &ToolTexts, n: usize, why: String) -> String {
    texts.render(
        &texts.patch_block_fault,
        &[("n", n.to_string()), ("why", why)],
    )
}

/// patch 解析失败的回执（每一类都说清事实）。
pub(crate) fn patch_fault(texts: &ToolTexts, f: &crate::capabilities::tools::api::Fault) -> String {
    use crate::capabilities::tools::api::Fault as F;
    match f {
        F::NoBlocks => texts.patch_no_blocks.clone(),
        F::UnknownMarker { line, text } => texts.render(
            &texts.patch_unknown_marker,
            &[("line", line.to_string()), ("text", text.clone())],
        ),
        F::MissingPath { line, marker } => texts.render(
            &texts.patch_missing_path,
            &[("line", line.to_string()), ("marker", marker.clone())],
        ),
        F::EmptyAdd { line } => texts.render(&texts.patch_empty_add, &[("line", line.to_string())]),
        F::EmptyUpdate { line } => {
            texts.render(&texts.patch_empty_update, &[("line", line.to_string())])
        }
        F::EmptySearch { line } => {
            texts.render(&texts.patch_empty_search, &[("line", line.to_string())])
        }
        F::MissingEnd { line } => {
            texts.render(&texts.patch_missing_end, &[("line", line.to_string())])
        }
        F::SearchWithoutReplace { line } => texts.render(
            &texts.patch_search_no_replace,
            &[("line", line.to_string())],
        ),
        F::ReplaceWithoutSearch { line } => texts.render(
            &texts.patch_replace_no_search,
            &[("line", line.to_string())],
        ),
    }
}

/// 一处改动失败的原因句（外面再套"第 N 块第 k 处"）。
pub(crate) fn edit_fault_reason(
    texts: &ToolTexts,
    f: &crate::capabilities::tools::api::EditFault,
) -> String {
    use crate::capabilities::tools::api::EditFault as E;
    match f {
        E::NotFound { total } => {
            texts.render(&texts.patch_not_found, &[("total", total.to_string())])
        }
        E::Multiple { lines } => texts.render(
            &texts.patch_multiple,
            &[
                ("n", lines.len().to_string()),
                (
                    "lines",
                    lines
                        .iter()
                        .map(|l| l.to_string())
                        .collect::<Vec<_>>()
                        .join("、"),
                ),
            ],
        ),
        E::NearMiss { line, actual } => texts.render(
            &texts.patch_near,
            &[("line", line.to_string()), ("actual", actual.clone())],
        ),
    }
}

/// 参数不符的回执：说清是哪一条不合（由声明判定），再把工具签名原样发回去。
pub fn arg_fault_text(
    texts: &ToolTexts,
    name: &str,
    schema: &ToolSchema,
    fault: &ArgFault,
) -> String {
    let why = match fault {
        ArgFault::NotObject => texts.arg_not_object.clone(),
        ArgFault::Missing(n) => texts.render(&texts.arg_missing, &[("name", n.clone())]),
        ArgFault::WrongType { name, want } => texts.render(
            &texts.arg_wrong_type,
            &[("name", name.clone()), ("want", want.to_string())],
        ),
        ArgFault::Empty(n) => texts.render(&texts.arg_empty, &[("name", n.clone())]),
        ArgFault::TooSmall { name, min } => texts.render(
            &texts.arg_too_small,
            &[("name", name.clone()), ("min", min.clone())],
        ),
        ArgFault::TooBig { name, max } => texts.render(
            &texts.arg_too_big,
            &[("name", name.clone()), ("max", max.clone())],
        ),
        ArgFault::Unknown(n) => texts.render(&texts.arg_unknown, &[("name", n.clone())]),
    };
    texts.render(
        &texts.arg_fault,
        &[
            ("why", why),
            (
                "signature",
                format!("{}\n{}", name, schema.render_for_prompt()),
            ),
        ],
    )
}

/// 越权调用：这个席位没有这个工具（角色表决定工具面）——**如实拒绝**，不执行。
pub fn refuse(texts: &ToolTexts, name: &str) -> ToolOutcome {
    fail(texts.render(&texts.tool_not_allowed, &[("name", name.to_string())]))
}

pub(crate) fn fail(msg: String) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: msg,
    }
}

/// 按字符截断（不劈开 UTF-8）；返回（文本, 是否截断）。
pub(crate) fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}
