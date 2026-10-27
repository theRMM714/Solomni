//! 提示词渲染层：{{key}} 占位替换，纯逻辑。
//! 册子文本来自 PromptSource（文件机制在适配层）；缺键/缺变量报错，不静默。
//! 指令文案改 prompts/；运行时回执由产生它的代码返回，不进册子。

use serde::Deserialize;

/// 渲染时的变量表。
pub type Vars<'a> = &'a [(&'a str, String)];

/// 渲染单段提示词：替换 {{name}}；遇到未提供的变量 = 错误（如实，不静默）。
pub fn render(template: &str, vars: Vars) -> Result<String, String> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'{' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            if let Some(end_rel) = template[i + 2..].find("}}") {
                let key = template[i + 2..i + 2 + end_rel].trim();
                let value = vars
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, v)| v.clone())
                    .ok_or_else(|| format!("提示词变量缺失：{}", key))?;
                out.push_str(&value);
                i += 2 + end_rel + 2;
                continue;
            }
        }
        // 字面 {{（非占位）按原样保留；JSON 示例中的花括号不受影响（单括号）。
        let ch_len = template[i..]
            .chars()
            .next()
            .map(|c| c.len_utf8())
            .unwrap_or(1);
        out.push_str(&template[i..i + ch_len]);
        i += ch_len;
    }
    Ok(out)
}

/// 装配输入的**内存形态**：**只有提示词文本**（`prompts/`）。
/// 工具总表与角色表是**另一个能力的东西**（`systools/`，见 `capabilities/tools/`）：
/// 挂进这里就等于让提示词能力反过来依赖工具能力，两边成环。
///
/// **持有者只有本能力**（`service.rs` 一处，R4）。运行时回执与清单行归各产生它们的代码，
/// 不再从这里共享；这里只有提示词段。
#[derive(Debug, Clone)]
pub struct Prompts {
    /// 按名字取的那些段。**布局只有本能力知道**。
    pub(crate) core: CoreTexts,
}

/// **一段核心提示词的名字**：别的能力按名字取段，不点册子内部结构。
///
/// 加/改一段提示词的步骤因此固定成两步：`prompts/` 里加键 → 这里加一个变体（缺了编译不过）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    ChatProtocol,
    DiscussOpener,
    DiscussStep,
    DiscussAutonomyNote,
    SynthesizeSystem,
    SynthesizeUser,
    ExecuteUser,
    ReviewSystem,
    ReviewUser,
    NodeReviewSystem,
    NodeReviewUser,
    SlateSystem,
    SlateUser,
    SlateModeSingle,
    SlateModeCollab,
    VerdictSystem,
    VerdictUser,
    /// 核心代理（core_proxy）的身份提示词：在用户授予的任务级授权范围内代用户决定与转达。
    ProxySystem,
    AgentSystem,
    /// 目的：agent 的人设块（只有登记了人设时才渲染；空人设不产生空标题）。
    AgentChara,
    ToolCallingEnvelope,
    ToolCallingNative,
    PatchGuide,
    /// 路径写法的唯一说明（env_block 把它插进每个环境块）。
    PathRules,
    /// 本回合工具说明块的标题（变量：tools）。
    ToolsThisTurn,
    /// 压缩回合给模型的指令。
    CompactPrompt,
    /// 讨论轮次里“上一轮没表态”的提醒。
    DiscussReminder,
    ModuleToolParamsHeader,
}

/// 把册子的多个文件合并成内存形态：各文件的**顶层键**合并后就是 `core:` 的内容。
///
/// **别的能力不按字段路径读册子**：它们按名字取一段（`Prompt::text` / `Prompt::render`）。
/// 所以"文件怎么分"与"某一段落在结构体的哪一格"都只有本能力知道——改版式不再牵动别的能力。
///
/// 三条如实报错（不静默）：**键在两个文件里重复**（拆分时最可能犯的错）、**缺键**、**类型不对**。
pub fn merge_book(docs: &[String]) -> Result<Prompts, String> {
    let mut merged = yaml_serde::Mapping::new();
    for doc in docs {
        let value: yaml_serde::Value =
            yaml_serde::from_str(doc).map_err(|e| format!("提示词册非法：{}", e))?;
        let map = match value {
            yaml_serde::Value::Mapping(m) => m,
            _ => return Err("提示词册非法：每个文件的最外层必须是一张键表".to_string()),
        };
        for (key, value) in map {
            if merged.insert(key.clone(), value).is_some() {
                return Err(format!("提示词册非法：键 {:?} 在两个文件里重复", key));
            }
        }
    }
    // 剩下的键就是"核心段"（`core:` 的内容）。
    let core: CoreTexts = yaml_serde::from_value(yaml_serde::Value::Mapping(merged))
        .map_err(|e| format!("提示词册缺键或类型不对：{}", e))?;
    // 机制册自检：条目挂的**会话使用类型**必须在全表里——缺了就是装配错误，不静默漏发一份机制。
    for m in &core.mechanisms {
        if !core.session_kinds.iter().any(|k| k == &m.session) {
            return Err(format!(
                "机制册非法：会话使用类型不在 session_kinds 里：{}",
                m.session
            ));
        }
    }
    // 环境册自检：每条只写 host 或 guest 之一、值在全表里；全表里每个值都要有条目（不静默漏发一份环境）。
    for e in &core.envs {
        match (&e.host, &e.guest) {
            (Some(h), None) => {
                if !core.env_hosts.iter().any(|x| x == h) {
                    return Err(format!("环境册非法：宿主平台不在 env_hosts 里：{}", h));
                }
            }
            (None, Some(g)) => {
                if !core.env_guests.iter().any(|x| x == g) {
                    return Err(format!("环境册非法：guest os 不在 env_guests 里：{}", g));
                }
            }
            _ => return Err("环境册非法：每条只写 host 或 guest 之一".to_string()),
        }
    }
    for h in &core.env_hosts {
        if !core
            .envs
            .iter()
            .any(|e| e.host.as_deref() == Some(h.as_str()))
        {
            return Err(format!("环境册非法：宿主平台没有对应的工作环境：{}", h));
        }
    }
    for g in &core.env_guests {
        if !core
            .envs
            .iter()
            .any(|e| e.guest.as_deref() == Some(g.as_str()))
        {
            return Err(format!("环境册非法：guest os 没有对应的工作环境：{}", g));
        }
    }
    Ok(Prompts { core })
}

#[derive(Debug, Clone, Deserialize)]
pub struct CoreTexts {
    /// 会话使用类型的**全表**：机制册的校验口径之一（另一处是角色表）。
    pub session_kinds: Vec<String>,
    /// **机制说明册**：每条按（会话使用类型 × 适用角色）分发，见 `mechanisms_for`。
    /// AI 不知道机制就只会写散文——所以哪一类会话由哪个角色拿哪一段，是**数据**，不是代码分支。
    pub mechanisms: Vec<Mechanism>,
    pub chat_protocol: String,
    pub discuss: DiscussPrompts,
    pub synthesize: SynthPrompts,
    pub execute: ExecutePrompts,
    pub review: ReviewPrompts,
    /// 节点级验收（任务链：逐节点核对当前目标）。
    pub node_review: NodeReviewPrompts,
    pub slate: SlatePrompts,
    /// 判定用户对裁决的回应是否明确到可以开工/放行。
    pub verdict: VerdictPrompts,
    /// 核心代理（core_proxy）的身份提示词。
    pub proxy: ProxyPrompts,
    /// 一个 agent 的职责提示词（由它的模块合成为一份能力包）。
    pub agent: AgentPrompts,
    /// 工具调用约定：手写信封（envelope 形态）。
    pub tool_calling_envelope: String,
    /// 工具调用约定：原生工具调用（native 形态）；与上一条**互斥**，一个通道只用一套。
    pub tool_calling_native: String,
    /// 目的：工作环境册的**宿主平台全表**（本机档按它匹配与校验）。
    pub env_hosts: Vec<String>,
    /// 目的：工作环境册的 **guest os 全表**（虚拟机档按它匹配与校验）。
    pub env_guests: Vec<String>,
    /// 目的：**工作环境册**——每条按宿主平台或 guest 的 os 分发（见 `env_for`）；变量 work_name / agent / work_root / sandbox_root / module_roots。
    pub envs: Vec<EnvBlock>,
    /// patch 通道的写法说明（模型侧）；变量：work_root, sandbox_root
    pub patch_guide: String,
    /// 路径写法的唯一说明；变量：sandbox_root。
    pub path_rules: String,
    /// 本回合工具说明块的标题；变量：tools。
    pub tools_this_turn: String,
    /// 压缩回合给模型的指令。
    pub compact_prompt: String,
    /// 讨论轮次里“上一轮没表态”的提醒。
    pub discuss_reminder: String,
    /// 模块工具参数段的小标题（module_tool_params 的取值）。
    pub module_tool_params_header: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiscussPrompts {
    /// opener 变量：protocol, task
    pub opener: String,
    /// step 变量：transcript
    pub step: String,
    pub autonomy_note: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VerdictPrompts {
    pub system: String,
    /// user 变量：kind, payload, text
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SynthPrompts {
    pub system: String,
    /// user 变量：transcript
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProxyPrompts {
    /// 核心代理的身份提示词（能用哪些工具由角色表按回合注入，不在这里列清单）。
    pub system: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ExecutePrompts {
    /// user 变量：tasks
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReviewPrompts {
    pub system: String,
    /// user 变量：plan, reports
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NodeReviewPrompts {
    pub system: String,
    /// user 变量：nodes
    pub user: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SlatePrompts {
    pub system: String,
    /// user 变量：mode, agents, modules, models, task
    pub user: String,
    /// 形态描述（{{mode}} 的取值）：单 agent。
    pub mode_single: String,
    /// 形态描述（{{mode}} 的取值）：协作。
    pub mode_collab: String,
}

/// 目的：一条工作环境说明——`host`（宿主平台）或 `guest`（guest 的 os）恰有其一（见 `CoreTexts::env_for`）。
#[derive(Debug, Clone, Deserialize)]
pub struct EnvBlock {
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub guest: Option<String>,
    pub text: String,
}

/// 一条机制说明：**会话使用类型与适用角色同时对上**才注入（见 `CoreTexts::mechanisms_for`）。
#[derive(Debug, Clone, Deserialize)]
pub struct Mechanism {
    /// 会话使用类型（必须在 `session_kinds` 里）。
    pub session: String,
    /// 适用角色（必须在角色表里存在——悬空引用由装配期 `SystemTools::problems()` 挡下）。
    pub roles: Vec<String>,
    pub text: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentPrompts {
    /// system 变量：agent, chara_block, modules, mechanism, env, tool_calling
    pub system: String,
    /// 目的：人设块模板；变量 chara（只有人设非空时才渲染并插入 system 的 chara_block）。
    pub chara: String,
}

impl CoreTexts {
    /// 按（**会话使用类型**，**角色**）取这个回合的机制说明：只拿**同时**匹配的条目，
    /// 按册子里的声明顺序拼接（一条都没配 = 空串）。对应关系全部写在 YAML 里，这里只做匹配。
    pub fn mechanisms_for(&self, session: &str, role: &str) -> String {
        self.mechanisms
            .iter()
            .filter(|m| m.session == session && m.roles.iter().any(|r| r == role))
            .map(|m| m.text.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// 目的：按（宿主平台，guest 的 os）取工作环境块——虚拟机档传 guest，本机档传 None；只拿匹配的条目按声明顺序拼接（装载时校验全表覆盖）。
    pub fn env_for(&self, host: &str, guest: Option<&str>) -> String {
        self.envs
            .iter()
            .filter(|e| match guest {
                Some(g) => e.guest.as_deref() == Some(g),
                None => e.host.as_deref() == Some(host),
            })
            .map(|e| e.text.trim().to_string())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// **按名字取一段原文**：册子的布局只在这里露面（新加一段 = 这里加一支 match）。
    pub fn segment(&self, seg: Segment) -> &str {
        match seg {
            Segment::ChatProtocol => &self.chat_protocol,
            Segment::DiscussOpener => &self.discuss.opener,
            Segment::DiscussStep => &self.discuss.step,
            Segment::DiscussAutonomyNote => &self.discuss.autonomy_note,
            Segment::SynthesizeSystem => &self.synthesize.system,
            Segment::SynthesizeUser => &self.synthesize.user,
            Segment::ExecuteUser => &self.execute.user,
            Segment::ReviewSystem => &self.review.system,
            Segment::ReviewUser => &self.review.user,
            Segment::NodeReviewSystem => &self.node_review.system,
            Segment::NodeReviewUser => &self.node_review.user,
            Segment::SlateSystem => &self.slate.system,
            Segment::SlateUser => &self.slate.user,
            Segment::SlateModeSingle => &self.slate.mode_single,
            Segment::SlateModeCollab => &self.slate.mode_collab,
            Segment::VerdictSystem => &self.verdict.system,
            Segment::VerdictUser => &self.verdict.user,
            Segment::ProxySystem => &self.proxy.system,
            Segment::AgentSystem => &self.agent.system,
            Segment::AgentChara => &self.agent.chara,
            Segment::ToolCallingEnvelope => &self.tool_calling_envelope,
            Segment::ToolCallingNative => &self.tool_calling_native,
            Segment::PatchGuide => &self.patch_guide,
            Segment::PathRules => &self.path_rules,
            Segment::ToolsThisTurn => &self.tools_this_turn,
            Segment::CompactPrompt => &self.compact_prompt,
            Segment::DiscussReminder => &self.discuss_reminder,
            Segment::ModuleToolParamsHeader => &self.module_tool_params_header,
        }
    }
}
