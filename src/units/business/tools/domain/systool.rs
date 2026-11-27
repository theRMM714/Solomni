//! 目的：核心自带内置工具（read / write / edit / patch / delete / list / grep）的纯规则——参数校验、取缺省值、拼回执、观察账本。
//! 管：按 `systools/tools.yaml` 的声明校验参数、取缺省值与拼回执；**参数契约不在本文件里**（声明在工具表，本文件只按声明行事）。
//! 不管：执行策略（名字固定、放行、寻址、根内校验、改动前的"读过"证据）在 runtime；读写机制在 `SysIo` 端口（适配层）。
//! 联动：由 `service/systool.rs` 调用；路径一律真实绝对路径（根目录经提示词册如实告知），模块声明的外部工具与内置工具同一套路径。

use crate::units::business::prompt::api::{Prompt, Segment};
use crate::units::business::tools::api::{ArgFault, ToolSchema};
use crate::units::business::workspace::api::Sandbox;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 目的：一次工具执行结果的形状：**归 kernel 共享**——内置、模块与核心自有工具同一形状（R6）。
pub use crate::kernel::api::types::ToolOutcome;

pub const READ: &str = "read";
pub const WRITE: &str = "write";
pub const EDIT: &str = "edit";
pub const PATCH: &str = "patch";
pub const DELETE: &str = "delete";
pub const LIST: &str = "list";
pub const GREP: &str = "grep";
/// 目的：执行类系统工具——一次性 shell（机制在 kernel 的 ProcessRunner / guest，不在 SysIo）。
pub const SHELL: &str = "shell";

/// 目的：单次读取回传的字符上限（超出如实截断，并在回执里给出接着读的 offset）。
/// 约束：必须低于 budget::MAX_RESULT_CHARS，给文件头与“接着读”提示留余量——否则发送前还会被
///   cap_result 从中间截断，模型看到的就不是我们记成“全文”的那份内容了。
pub const MAX_READ_CHARS: usize = crate::kernel::api::budget::MAX_RESULT_CHARS - 2_000;
/// 目的：回执里单行的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_READ_LINE_CHARS: usize = 2_000;
/// 目的：单次 read 最多返回的行数；也是 `limit` 的缺省值与上限（与 systools/tools.yaml 一致）。
/// 约束：机制层按这个窗口取行，但会扫完整份文件统计行数与版本——>1MB 的文件也能直接读后半段。
pub const MAX_READ_LINES: usize = 2_000;
/// 目的：单次 grep 回传的命中行数上限（`max` 的缺省值；到上限就停止扫描并如实报）。
pub const MAX_GREP_HITS: usize = 200;
/// 目的：`max` 参数的上限（再多就该缩小 path / glob，而不是往上下文里灌内容）。
pub const MAX_GREP_MAX: usize = 500;
/// 目的：单条命中行回传的字符上限（长行截断，避免一行吃掉整个上下文）。
pub const MAX_GREP_LINE_CHARS: usize = 300;
/// 目的：单个文件最多报多少条命中（其余只计数，提示缩小 pattern 或单独搜这个文件）。
pub const MAX_GREP_PER_FILE: usize = 50;
/// 目的：`context` 参数的上限（前后各最多几行）。
pub const MAX_GREP_CONTEXT: usize = 10;
/// 目的：一次目录递归最多扫多少个文件（再多就如实提示缩小范围）。
pub const MAX_GREP_FILES: usize = 5_000;
/// 目的：grep 回执正文的字符上限（`max` 条 × 单行上限之后仍要落在结果预算内）。
pub const MAX_GREP_CHARS: usize = crate::kernel::api::budget::MAX_RESULT_CHARS - 2_000;

/// 目的：执行席的**回报**工具：不碰文件，只把"做完了什么"承载成一次工具调用。
///    为什么是工具而不是正文 JSON：回报会驱动核心（判节点完成），属于核心操作（见 tools-and-roles.md）。
pub const REPORT: &str = "submit_report";

/// 目的：**人的命令面**——呈现层（CLI / Web 动作目录）对外暴露的那些动作 id。
/// 约束：只回答「这条动作有没有人的入口」，不是授权：会话里的工具面由来源各自交出（见 session_model
///   的 ToolSource），同一 id 可以既是人的动作、又是某个来源在会话里交出的工具（read / create_session 就是）。
///   与 systools/tools.yaml 的一致性由 src/tests/commands.rs 锁死：表里每条动作要么在这张单子，
///   要么由某个会话来源交出，孤儿即红。
pub const HUMAN_ACTIONS: &[&str] = &[
    READ,
    LIST,
    GREP,
    SHELL,
    "compact",
    "create_session",
    "control_session",
    "activate_session",
    "guest_diagnose",
    "explain_tools",
    "set_secret",
    "clear_secret",
    "control_resident",
    "set_module_enabled",
    "send_message",
    "set_task",
    "answer_card",
    "withdraw",
    "rewind",
    "update_task",
    "set_permission_profile",
    "set_thinking",
    "set_context_freshness",
    "edit_session",
    "upload",
    "restore_work_commit",
    "upsert_provider",
    "remove_provider",
    "discover_models",
    "upsert_model",
    "remove_model",
    "set_core_model",
    "probe_model_tools",
    "probe_replay_shape",
    "upsert_agent",
    "remove_agent",
    "set_settings",
];

/// 目的：这条动作有没有人的入口（呈现层能不能调它）。
pub fn is_human_action(id: &str) -> bool {
    HUMAN_ACTIONS.contains(&id)
}
/// 目的：内置工具名（保留名）。
///    与 systools/tools.yaml 的 tools 是同一份名单，测试「builtin_tool_book_is_the_one_source_of_names_and_paths」锁死两者一致。
pub fn is_builtin(name: &str) -> bool {
    name == READ
        || name == WRITE
        || name == EDIT
        || name == PATCH
        || name == DELETE
        || name == LIST
        || name == GREP
        || name == REPORT
}

/// 目的：这个系统工具名是不是执行类（shell）——不是 SysIo 内置文件工具，但同样按系统工具面发放与校验。
pub fn is_exec(name: &str) -> bool {
    name == SHELL
}

/// 目的：系统工具保留名清单（拼错误提示、模块工具不得占用）。
pub fn names() -> Vec<String> {
    vec![
        READ.to_string(),
        WRITE.to_string(),
        EDIT.to_string(),
        PATCH.to_string(),
        DELETE.to_string(),
        LIST.to_string(),
        GREP.to_string(),
        REPORT.to_string(),
        SHELL.to_string(),
    ]
}

/// 目的：patch 在**原生通道**上的声明：参数只有一个 body。
///    原生协议要求参数是 JSON 对象，所以补丁正文当字符串值传——转义交给供应商的解码器，
///    模型不必自己写转义（这正是原生通道相对手写信封的收益）。
pub fn patch_decl() -> crate::units::business::llm::api::ToolDecl {
    crate::units::business::llm::api::ToolDecl {
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

/// 目的：自由格式工具：输入不是 JSON 参数，而是**信封之后的那段原样文本**（不必转义）。
///    存在的理由：把大段内容塞进 JSON 字符串要转义换行/引号，是真实会话里反复出事的点。
pub fn is_freeform(name: &str) -> bool {
    name == PATCH
}

/// 目的：工具说明块的**素材**（装配期算一次，随回合注入）：patch 语法、模块工具清单、模块工具参数。
///    为什么在这里算：它们只与这个 agent 的沙箱与模块有关、与回合无关；而回合执行路径上拿不到提示词册。
#[derive(Debug, Clone, Default)]
pub struct ToolNotes {
    pub patch_guide: String,
    pub module_tools: String,
    /// 目的：模块工具与**已就绪服务操作的参数合并成一段**；标题只出现一次（服务行由装配方传入）。
    pub module_tool_params: String,
}

pub fn tool_notes(
    prompt: &dyn Prompt,
    sb: &Sandbox,
    modules: &[crate::units::business::catalog::modules::api::Module],
    service_param_sections: &[String],
) -> ToolNotes {
    // 示例路径必须与【工作环境】同一种写法：虚拟机档给 guest 写法，宿主盘符在 guest 里不存在。
    // 翻译器与 env_block 同源（沙箱的 path_codec）——两份提示词各用一套路径会把模型带偏。
    let view = |path: &std::path::Path| -> String {
        match sb.path_codec.as_ref() {
            Some(codec) => {
                crate::kernel::api::path::slash(std::path::Path::new(&codec.to_view(path)))
            }
            None => crate::kernel::api::path::slash(path),
        }
    };
    ToolNotes {
        patch_guide: prompt.render(
            Segment::PatchGuide,
            &[
                ("work_root", view(&sb.shared)),
                ("sandbox_root", view(&sb.private)),
            ],
        ),
        module_tools: crate::units::business::tools::domain::module_tools::module_tools(modules),
        module_tool_params: crate::units::business::tools::domain::module_tools::module_tool_params(
            prompt,
            modules,
            service_param_sections,
        ),
    }
}

/// 目的：观察账本：**本次会话里核心见过哪些文件的什么内容**。
///    它存在的唯一理由：整份覆盖（write）会丢掉没读到的内容，所以要求"先完整读过、且读后没被改过"。
///    语义边界：账本随会话（AgentSession）保存；回档（rewind）清空——转录里那段读取证据被截掉了，证据随之作废；
///    重启后按落盘重建的会话不带账本（从零开始，模型重新读一遍即可）。edit 不需要账本：old_string 本身就是要改的那段原文。
#[derive(Debug, Clone, Default)]
pub struct Observations {
    seen: BTreeMap<PathBuf, Seen>,
    /// 目的：本次调用里由核心**写过**的路径（批次末的存档快照与 work_commit 据此知道该收哪些文件）。
    /// 约束：只活在一次调用的分支副本里——循环在调用前清空、调用后取走；不参与 absorb。
    written: std::collections::BTreeSet<PathBuf>,
    /// 目的：本次调用里由核心**删掉**的路径（自动提交的 `deletes` 由此而来）。
    removed: std::collections::BTreeSet<PathBuf>,
    /// 目的：本次调用产生的观察事实（读、写、改、patch）；门禁按"路径 + 整份版本 + 覆盖"授权。
    /// 约束：挂在工具行上随转录落盘；不像 `seen` 那样只记 FNv1a，也不把"改过"当"读过全文"。
    facts: Vec<crate::units::business::tools::api::ObservationFact>,
    /// 当前正在执行的模型回复号；新产生的观察事实记上它，压缩时按它丢弃失效观察。
    reply: u64,
}

#[derive(Debug, Clone)]
enum Seen {
    /// 完整读过（或由核心写入）：当时的内容指纹。
    Known(u64),
    /// 只读到一部分（原因文案）：整份覆盖会被拒。
    Partial(String),
}

impl Observations {
    /// 目的：回档（删除模式）时清空：转录里那段读取证据已经不存在了。
    pub fn clear(&mut self) {
        self.seen.clear();
        self.written.clear();
        self.removed.clear();
        self.facts.clear();
    }

    /// 目的：告诉账本"接下来是第几次回复在干活"；事实据此带上回复号。
    pub(crate) fn set_reply(&mut self, reply: u64) {
        self.reply = reply;
    }

    /// 目的：当前回复号（构造观察事实时用）。
    pub fn reply(&self) -> u64 {
        self.reply
    }

    /// 目的：当前账本里的全部观察事实（门禁只读投影）。
    pub fn facts(&self) -> &[crate::units::business::tools::api::ObservationFact] {
        &self.facts
    }

    /// 目的：按落盘转录回填观察事实（重启 / 回档后门禁仍能看到活动窗口里的读）。
    /// 约束：先清空再装——调用方给的已经是活动窗口的投影，不叠加进程里那一份。
    pub fn restore_facts(
        &mut self,
        facts: Vec<crate::units::business::tools::api::ObservationFact>,
    ) {
        self.facts = facts;
    }

    /// 目的：压缩发送视图后，丢掉起点早于 `up_to` 的观察事实——那段内容模型已经看不到了。
    /// 约束：回复号就是该回复第一行的行 id，所以 `reply < up_to` 等价于"这次观察已被压缩掉"。
    pub fn retain_from(&mut self, up_to: u64) {
        self.facts.retain(|f| f.reply >= up_to);
    }

    /// 目的：取走"这次调用写过哪些路径"（取走即清空，下一次调用从零开始）。
    pub fn take_written(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.written).into_iter().collect()
    }

    /// 目的：取走"这次调用删掉了哪些路径"（取走即清空，下一次调用从零开始）。
    pub fn take_removed(&mut self) -> Vec<PathBuf> {
        std::mem::take(&mut self.removed).into_iter().collect()
    }

    /// 目的：记一条观察事实（门禁的授权证据）。
    pub(crate) fn note_fact(
        &mut self,
        mut fact: crate::units::business::tools::api::ObservationFact,
    ) {
        fact.coverage = fact.coverage.normalized();
        self.facts.push(fact);
    }

    /// 目的：记一次删除——work_commit 要把它写成 `deletes`，否则沙箱删了、工作线里还留着。
    pub(crate) fn note_removed(&mut self, path: &Path) {
        self.removed.insert(path.to_path_buf());
    }

    /// 目的：记一次读取：完整读到 = 记指纹（可覆盖）；不完整 = 记下"只读到一部分"的原因。
    ///    不完整的读取**不覆盖**已有的完整记录（读到一半不会让已知变成未知）。
    pub(crate) fn note_read(&mut self, path: &Path, complete: bool, why: String, hash: u64) {
        if complete {
            self.seen.insert(path.to_path_buf(), Seen::Known(hash));
        } else {
            self.seen
                .entry(path.to_path_buf())
                .or_insert(Seen::Partial(why));
        }
    }

    /// 目的：记一次写入：文件内容由核心产生，所以核心确切知道它现在是什么。
    pub(crate) fn note_written(&mut self, path: &Path, hash: u64) {
        self.seen.insert(path.to_path_buf(), Seen::Known(hash));
        self.written.insert(path.to_path_buf());
    }

    /// 目的：只记"这个路径被这次调用改过"（给存档线自动提交用），不冒充"整份读过"。
    pub(crate) fn note_touched(&mut self, path: &Path) {
        self.written.insert(path.to_path_buf());
    }

    /// 目的：合并一个**并发分支**的账本（分支里只跑声明可并发的只读工具）。
    ///    规则与逐个记账一致：完整读到记为所见、部分读到只补空白（读到一半不会让已知变成未知）。
    ///    调用方**必须按原始调用顺序**合并——那样并发批次与串行执行的结果完全相同。
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
        self.facts.extend(branch.facts.iter().cloned());
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

/// 目的：内容指纹（FNV-1a 64 位）：只用来判"读过的和现在的是不是同一份"，不做安全用途。
pub(crate) fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// 目的：字节位置对应的行号（从 1 数起）。
pub(crate) fn line_of(s: &str, at: usize) -> usize {
    s[..at].matches('\n').count() + 1
}

/// 目的：字面匹配的全部命中位置（不重叠）。
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

/// 目的：字面替换不成立的原因（机制层在锁内复算时用；文案由调用方决定）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum LiteralFault {
    /// 目的：一处也没找到。
    NotFound,
    /// 目的：命中多处，而这次要求唯一；附带每处所在行号。
    Ambiguous { lines: Vec<usize> },
}

/// 目的：字面替换的结果：新内容、改动行区间（1 起）、替换处数。
pub(crate) type LiteralEdit = (String, Vec<(u32, u32)>, usize);

/// 目的：按字面替换并算出新内容里被改到的行区间；`all=false` 要求恰好一处命中。
/// 约束：返回的行区间已 clamp 到新内容行数、排序并合并；`old` 为空一律视为找不到。
pub(crate) fn apply_literal_edit(
    hay: &str,
    old: &str,
    new: &str,
    all: bool,
) -> Result<LiteralEdit, LiteralFault> {
    let hits = find_all(hay, old);
    if hits.is_empty() {
        return Err(LiteralFault::NotFound);
    }
    if hits.len() > 1 && !all {
        return Err(LiteralFault::Ambiguous {
            lines: hits.iter().map(|i| line_of(hay, *i)).collect(),
        });
    }
    let chosen: Vec<usize> = if all { hits.clone() } else { vec![hits[0]] };
    let mut out = String::with_capacity(hay.len() + new.len());
    let mut last = 0usize;
    let mut changed: Vec<(u32, u32)> = Vec::new();
    for at in &chosen {
        out.push_str(&hay[last..*at]);
        let start = line_of(&out, out.len()) as u32;
        out.push_str(new);
        let end = start + new.matches('\n').count() as u32;
        changed.push((start, end));
        last = at + old.len();
    }
    out.push_str(&hay[last..]);
    let total_lines = out.lines().count();
    let changed: Vec<(u32, u32)> = changed
        .into_iter()
        .map(|(from, to)| {
            let from = from.min(total_lines.max(1) as u32);
            (from, to.min(total_lines.max(1) as u32).max(from))
        })
        .collect();
    let n = chosen.len();
    Ok((out, changed, n))
}

/// 目的："是不是只差空白"：按行比较（每行内空白折叠）。命中则给（行号, 该行原文）——模型据此改对缩进。
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

/// 目的：整份覆盖已存在文件的证据检查：本次会话里**完整读过**它、且读后没被改过（write 与 patch 的 Add 共用）。
pub(crate) fn overwrite_check(
    obs: &Observations,
    spec: &str,
    path: &Path,
    current: &str,
) -> Result<(), String> {
    // 读账本的指纹与 read 回执同口径：按 `lines()` 去掉行尾换行与末尾 `\r` 再连回去。
    // 不这样做，末尾换行或 CRLF 的差异会被当成"读后又被改过"。
    let normalized = current.lines().collect::<Vec<_>>().join("\n");
    match obs.known(path) {
        Some(seen) if seen == fnv1a(&normalized) => Ok(()),
        Some(_) => Err(format!(
            "{} 在你读过之后又被改动过（内容与当时不同）：请重新 read 一遍再决定怎么写，不要凭记忆覆盖。",
            spec
        )),
        None => Err(match obs.partial_why(path) {
            Some(w) => format!(
                "{} 你只读到一部分：{}。整份覆盖会丢掉没读到的内容，请改用 edit 改确定的片段。",
                spec, w
            ),
            None => format!(
                "要整份覆盖已存在的文件 {}，必须在本次会话里先**完整读过**它（只读一段不算）。请先 read 全文，或改用 edit 只改那一处。",
                spec
            ),
        }),
    }
}

/// 目的：整份覆盖已有文件时，内容按原文件的行尾风格写（免得把 CRLF 文件改成 LF）。
pub(crate) fn align_eol(content: &str, current: &str) -> String {
    if current.contains("\r\n") {
        content.replace('\n', "\r\n")
    } else {
        content.to_string()
    }
}

/// 目的：某一块不成立的回执（整体不写盘，所以措辞要直说这一点）。
pub(crate) fn block_fault(n: usize, why: String) -> String {
    format!("[patch] 第 {} 块不成立，**没有任何文件被写入**：{}", n, why)
}

/// 目的：patch 解析失败的回执（每一类都说清事实）。
pub(crate) fn patch_fault(f: &crate::units::business::tools::api::Fault) -> String {
    use crate::units::business::tools::api::Fault as F;
    match f {
        F::NoBlocks => {
            "没认出任何补丁标记（要 *** Add File: 路径 或 *** Update File: 路径）".to_string()
        }
        F::UnknownMarker { line, text } => format!(
            "第 {} 行看不懂的标记 *** {}（只认 Add File / Update File / SEARCH / REPLACE / End File）",
            line, text
        ),
        F::MissingPath { line, marker } => {
            format!("第 {} 行的 {} 后面没写路径", line, marker)
        }
        F::EmptyAdd { line } => format!("第 {} 行的 Add File 后面没有内容", line),
        F::EmptyUpdate { line } => {
            format!("第 {} 行的 Update File 后面没有 SEARCH / REPLACE", line)
        }
        F::EmptySearch { line } => format!(
            "第 {} 行的 SEARCH 是空的（要在文件开头插入，就把开头那几行一起写进 SEARCH）",
            line
        ),
        F::MissingEnd { line } => format!(
            "第 {} 行起的这一块没有 *** End File 收尾（它的后面可能还有别的话，核心不猜哪里是结尾）：请给每一块都补上 *** End File。",
            line
        ),
        F::SearchWithoutReplace { line } => {
            format!("第 {} 行的 SEARCH 后面没有 REPLACE", line)
        }
        F::ReplaceWithoutSearch { line } => {
            format!("第 {} 行的 REPLACE 前面没有 SEARCH", line)
        }
    }
}

/// 目的：一处改动失败的原因句（外面再套"第 N 块第 k 处"）。
pub(crate) fn edit_fault_reason(f: &crate::units::business::tools::api::EditFault) -> String {
    use crate::units::business::tools::api::EditFault as E;
    match f {
        E::NotFound { total } => {
            format!("SEARCH 的原文按**整行**匹配找不到（该文件共 {} 行）", total)
        }
        E::Multiple { lines } => format!(
            "SEARCH 命中了 {} 处（第 {} 行）：把上下文补足，让它唯一",
            lines.len(),
            lines
                .iter()
                .map(|l| l.to_string())
                .collect::<Vec<_>>()
                .join("、")
        ),
        E::NearMiss { line, actual } => {
            format!("第 {} 行与它只差空白：{}", line, actual)
        }
    }
}

/// 目的：参数不符的回执：说清是哪一条不合（由声明判定），再把工具签名原样发回去。
pub fn arg_fault_text(name: &str, schema: &ToolSchema, fault: &ArgFault) -> String {
    let why = match fault {
        ArgFault::NotObject => "args 必须是一个参数对象".to_string(),
        ArgFault::Missing(n) => format!("缺少必填参数 {}", n),
        ArgFault::WrongType { name, want } => format!("参数 {} 需要 {}", name, want),
        ArgFault::Empty(n) => format!("参数 {} 不能是空字符串", n),
        ArgFault::TooSmall { name, min } => format!("参数 {} 不能小于 {}", name, min),
        ArgFault::TooBig { name, max } => format!("参数 {} 不能大于 {}", name, max),
        ArgFault::Unknown(n) => format!("没有参数 {}", n),
        ArgFault::NotInValues { name, values } => {
            format!("参数 {} 只能是 {}", name, values.join(" / "))
        }
        ArgFault::UnknownFlag(f) => format!("没有旗语 {}", f),
        ArgFault::FlagNeedsValue(f) => format!("旗语 {} 缺值", f),
        ArgFault::Duplicate(n) => format!("参数 {} 给重了", n),
        ArgFault::ExtraOperand(n) => format!("多了声明没接收的参数 {}", n),
    };
    let signature = format!("{}\n{}", name, schema.render_for_prompt());
    format!("{}。{}", why, signature)
}

/// 目的：越权调用：这个席位没有这个工具（机制席位决定工具面）——**如实拒绝**，不执行。
pub fn refuse(name: &str) -> ToolOutcome {
    fail(format!(
        "这个席位没有工具 {}：机制席位没发给它（越权调用一律不执行）。",
        name
    ))
}

pub(crate) fn fail(msg: String) -> ToolOutcome {
    ToolOutcome::new(false, msg)
}

/// 目的：按字符截断（不劈开 UTF-8）；返回（文本, 是否截断）。
pub(crate) fn truncate_chars(s: &str, max: usize) -> (String, bool) {
    if s.chars().count() <= max {
        return (s.to_string(), false);
    }
    (s.chars().take(max).collect(), true)
}
