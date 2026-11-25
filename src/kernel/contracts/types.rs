//! 目的：跨层共享的形状。
//! 管：只没有领域逻辑的事实——`SessionId` / `ToolOutcome` / `Ask`（请用户裁决的问题）/ 默认预算。
//! 不管：带领域逻辑的类型（归各自能力）；执行与作业模型（归环 1 的 exec_model）；各业务自造一份 DTO 副本（R6 禁止）。
//! 联动：由 `src/kernel/api.rs` 重导出；判据见 ARCHITECTURE.md 的「硬要求清单」R6。

use crate::kernel::contracts::note::{Note, Severity};

/// 目的：单次模型调用的默认总预算（秒）。
/// 约束：登记处（设置项）与 `llm` 的出站调用参数共享它；设置项见 `AppSettings::llm_timeout_secs`。
pub const DEFAULT_LLM_TIMEOUT_SECS: u64 = 300;

/// 目的：**能力名**的合法写法（模块 `runtimes` 与运行包 `id` 两侧共享同一份文法）。
/// 约束：小写字母或数字开头，其余小写字母、数字、`-` `_` `.`；长度 1..=64。
pub fn valid_capability(name: &str) -> bool {
    let mut it = name.chars();
    match it.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    if name.len() > 64 {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// 目的：前端唯一的会话标识 = 工作名（用户的命名，也是落盘目录名）。
pub type SessionId = String;

/// 目的：一次工具调用的事实结果——`ok` = 成功；`severity` 分轻重；`output` 是原始输出；
///   `notes` 是结构化说明（稳定码 + 参数，不带文案）。
/// 约束：内置工具、模块工具与核心自有工具共用这同一个形状，谁都不该复制第二份（R6）；
///   `output` 只放被执行者产出的原始文本，**不放**任何需要按语言渲染的句子（那些进 `notes`）。
#[derive(Debug, Clone, PartialEq)]
pub struct ToolOutcome {
    pub ok: bool,
    pub severity: Severity,
    pub output: String,
    pub notes: Vec<Note>,
}

impl ToolOutcome {
    /// 目的：按成败造一条没有说明的回执——成功是 `Info`，失败是 `Error`。
    pub fn new(ok: bool, output: impl Into<String>) -> ToolOutcome {
        ToolOutcome {
            ok,
            severity: if ok { Severity::Info } else { Severity::Error },
            output: output.into(),
            notes: Vec::new(),
        }
    }

    /// 目的：就地追加一条说明（执行层边跑边补事实时用）；分级取最重的一条。
    pub fn push_note(&mut self, note: Note) {
        self.severity = self.severity.max(note.severity);
        self.notes.push(note);
    }

    /// 目的：整批追加说明（例如回执已经把分区与收尾说明拆好）——分级同样取最重的一条。
    pub fn with_notes(mut self, notes: Vec<Note>) -> ToolOutcome {
        for note in &notes {
            self.severity = self.severity.max(note.severity);
        }
        self.notes.extend(notes);
        self
    }
}

impl From<crate::kernel::contracts::run::RunOutcome> for ToolOutcome {
    /// 目的：机制层的中性结果原样抬成工具回执——四个字段一对一，业务层不再逐个搬。
    fn from(run: crate::kernel::contracts::run::RunOutcome) -> ToolOutcome {
        ToolOutcome {
            ok: run.ok,
            severity: run.severity,
            output: run.output,
            notes: run.notes,
        }
    }
}

/// 目的：一条**请用户裁决**的问题：谁在问（信封）+ 消息三段 + 选项集——通道只管送，不解释内容。
/// 约束：请求方（工具执行层、围栏这类机制）与通道（会话的裁决队）共用这一个形状，所以它属于 kernel（R6）；
///   问什么、有几个选项、每个选项做什么都由请求方实现，通道只把选项 **id** 原样还给请求方。
///   端口见 `crate::kernel::ports::AskUser`；选项 id 与卡片的派生见 docs/session/session-model.md 的「请用户裁决」。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    /// 目的：谁在问——信封第一段（core 核心 / member 某一席 / tools 工具层）。
    pub role: String,
    /// 目的：具体是谁——信封第二段（agent 实例名或机制名）。
    pub name: String,
    /// 目的：标题（一句话说清这一问是什么）。
    pub title: String,
    /// 目的：正文（为什么问：不改这一环会怎样）。
    pub body: String,
    /// 目的：详情（展开看的东西：哪个目录、缺什么前提、怎么补）；没有就是空串。
    pub detail: String,
    /// 目的：选项集——`(id, label)`；**id 是行为契约**（回答按 id 分派、改文案不改行为），label 只给渲染。
    pub options: Vec<(String, String)>,
    /// 目的：**没人答时怎么办**——发起方声明一个**自己卡上的选项 id**（通道不解释它意味着什么）；
    ///   不写 = 没人答就交回发起方按 fail-closed 处置。它是"这一问的收场声明"，不是通道的词表：
    ///   想加"接受 / 拒绝 / 全部接受"这类选项，改的是发起方自己的选项集，不是这里。
    #[serde(default)]
    pub on_unanswered: Option<String>,
}

impl Ask {
    /// 目的：这个选项 id 是不是本卡选项集里的一条（声明与回答都按它校验，不静默改写）。
    pub fn has_option(&self, id: &str) -> bool {
        self.options.iter().any(|(o, _)| o == id)
    }
}

/// 目的：一次提问的收场——**为什么没有答案**如实分开，发起方据此处置（不必逐态 match，用 `decided`）。
/// 约束：通道只认选项 id 与这三种"没有答案"的事实，不认识"接受 / 拒绝"这类语义（那是发起方自己的选项）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskOutcome {
    /// 前端答了某个选项 id。
    Chosen(String),
    /// 没人答，按发起方声明的 `on_unanswered` 收场（**如实标为"不是用户答的"**）。
    Defaulted(String),
    /// 没人答、也没声明默认项：发起方按 fail-closed 处置（这一趟没有可回答的前端，或前端断了）。
    NoAnswer,
    /// 用户按了停止 / 会话被关闭：整队作废（= 拒绝）——**不套用默认项**，停止不能被当成放行。
    Stopped,
    /// 构不出可用选项：端口不发起裁决，改为停掉这个会话 + 落一条警告（发起方不执行）。
    NoOptions,
}

impl AskOutcome {
    /// 目的：这次**照哪个选项 id 办**——用户答的或声明的默认项都算；`None` = 这次不办。
    /// 约束：发起方只需要看这一个结果，不必逐态 match；"为什么没答案"的如实记录由端口落进会话。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn decided(&self) -> Option<&str> {
        match self {
            AskOutcome::Chosen(id) | AskOutcome::Defaulted(id) => Some(id.as_str()),
            _ => None,
        }
    }
}
