//! 目的：会话的系统提示拼装：身份块、工具面、本回合提示、表态约定与回合号。
//! 管：agent 身份与席位身份两条入口共用一份装配；模块提示与回合参数的类型也在这里。
//! 不管：会话状态与转录（`domain/session.rs`）、提示词模板与落盘（`prompt` 能力）。
//! 联动：`domain/session.rs` 重导出，`api.rs` 对外给呈现层；模板在 `units/business/prompt`。

use crate::units::business::llm::api::Msg;

/// 目的：**身份块的系统提示**：把 `{{mechanism}}` / `{{env}}` / `{{tool_calling}}` 三件事按同一口径填进模板。 模块只是能力包（没有"发言"这回事）；发言席是 agent，所以这份 system 按 agent 成文。 `brief` 是能力摘要，`system` 是工具用法；两者都来自模块声明。 env 由 tools 的 systool 按该 agent 的沙箱渲染后传入。 **工具清单不在这里**：本回合能用哪些工具随回合注入（见 collab 的 engine::tools_block）。
#[allow(clippy::too_many_arguments)]
pub fn agent_system(
    prompt: &dyn crate::units::business::prompt::api::Prompt,
    agent: &str,
    chara: &str,
    modules: &[ModulePrompt],
    env: &str,
    mode: crate::units::pure::registry_model::api::ToolMode,
    seat: &str,
) -> String {
    render_system(
        prompt,
        crate::units::business::prompt::api::Segment::AgentSystem,
        agent,
        chara,
        modules,
        env,
        mode,
        seat,
    )
}

/// 目的：**席位身份块**（不是 agent 的核心身份）：渲染席位提示词段 + 环境 + 调用约定。 代理会话（core_proxy）用它：它的身份是席位提示词（`prompts/roles/core_proxy.yaml`）， 不是"某个 agent 的模块能力包"；机制 / 环境 / 调用约定与 agent 身份同一份口径。
#[allow(clippy::too_many_arguments)]
pub fn seat_system(
    prompt: &dyn crate::units::business::prompt::api::Prompt,
    segment: crate::units::business::prompt::api::Segment,
    agent: &str,
    env: &str,
    mode: crate::units::pure::registry_model::api::ToolMode,
    seat: &str,
) -> String {
    render_system(prompt, segment, agent, "", &[], env, mode, seat)
}

/// 两处身份（agent / 席位）**共用这一份装配**：模板不同，变量与取值口径完全相同——
/// 各写一份必然漂移。
#[allow(clippy::too_many_arguments)]
fn render_system(
    prompt: &dyn crate::units::business::prompt::api::Prompt,
    segment: crate::units::business::prompt::api::Segment,
    agent: &str,
    chara: &str,
    modules: &[ModulePrompt],
    env: &str,
    mode: crate::units::pure::registry_model::api::ToolMode,
    seat: &str,
) -> String {
    use crate::units::business::prompt::api::Segment;
    let parts = modules
        .iter()
        .map(|m| {
            let brief = m.brief.trim();
            if brief.is_empty() {
                format!("\n== {} ==\n{}", m.id, m.system.trim())
            } else {
                format!("\n== {} ==\n能力：{}\n{}", m.id, brief, m.system.trim())
            }
        })
        .collect::<Vec<_>>()
        .join("");
    // 机制说明**按席位**从册子里取：对应关系是数据，这里只做匹配。
    //（AI 不知道机制就只会写散文。）
    let mechanism = prompt.mechanism(seat);
    // 人设块是**独立指令段**：空人设不产生空标题；模板文案在册子里，不在代码里。
    let chara_block = if chara.trim().is_empty() {
        String::new()
    } else {
        prompt.render(Segment::AgentChara, &[("chara", chara.trim().to_string())])
    };
    prompt.render(
        segment,
        &[
            ("agent", agent.to_string()),
            ("chara_block", chara_block),
            ("modules", parts),
            ("mechanism", mechanism),
            ("env", env.to_string()),
            // 两套调用约定**互斥**：一个通道只用一套（同时教会让模型在正文里讲解参数而被误判成调用）
            (
                "tool_calling",
                match mode {
                    crate::units::pure::registry_model::api::ToolMode::Native => {
                        prompt.text(Segment::ToolCallingNative).to_string()
                    }
                    crate::units::pure::registry_model::api::ToolMode::Envelope => {
                        prompt.text(Segment::ToolCallingEnvelope).to_string()
                    }
                },
            ),
        ],
    )
}

/// 目的：一个模块进身份块所需的文本：能力摘要 + 工具用法。
/// 约束：`brief` 只写能做什么（一行），`system` 只写怎么用（参数 / 路径 / 输入输出 / 错误 / 前置条件）。
#[derive(Debug, Clone)]
pub struct ModulePrompt {
    /// 目的：模块 id（身份块小节的标题）。
    pub id: String,
    /// 目的：能力摘要（身份块里的一行“能力：”）。
    pub brief: String,
    /// 目的：工具用法（参数 / 路径 / 输入输出 / 错误 / 前置条件）。
    pub system: String,
}

/// 目的：会话参数：拼请求要的**前提**（身份、环境）。**不是对话**——不占对话列表的位置， 每次模型调用由驱动现渲染（见 `identity`）。  为什么单独有个位置：参数是**派生**的（登记处 + 提示词册 + 工作区路径）。把它渲染成 "第 0 条消息"存进会话，就等于让参数冒充对话：回档、压缩、重建都得单独照顾那一条， 参数一改还得把整个会话重建一遍。
#[derive(Debug, Clone)]
pub struct SessionParams {
    /// 目的：agent 实例名（身份块的 {{agent}}，也是重建时的命名依据）。
    pub agent: String,
    /// 目的：工作名（环境块的 {{work_name}}）。
    pub work_name: String,
    /// 目的：共享区（真实根）。
    pub shared: std::path::PathBuf,
    /// 目的：该 agent 的私有沙箱（真实根）。
    pub private: std::path::PathBuf,
    /// 目的：模块 id → 目录（环境块列出的模块目录）。
    pub module_dirs: std::collections::BTreeMap<String, std::path::PathBuf>,
    /// 目的：人设 / 身份（建会话时快照；空 = 不产生人设块）。
    pub chara: String,
    /// 目的：模块能力包：id + 能力摘要 + 工具用法（顺序即装配顺序）。
    pub modules: Vec<ModulePrompt>,
    /// 目的：**席位提示词段**：`None` = 普通 agent（模块能力包 + `AgentSystem`）； `Some(seg)` = 不是 agent 的核心身份（代理这类），身份块渲染 `seg` 那一段席位提示词。 存段名而不是渲染好的文本：身份块**每回合现渲染**（册子一改，下一次调用就生效）。
    pub seat_system: Option<crate::units::business::prompt::api::Segment>,
    /// 目的：这一回合的**角色**（solo / executor / discussant / core_proxy…）：身份块与 tool 来源按它取。
    ///   约束：这是「当前的角色」——流程换段时驱动随回合换它、跑完还原（`TurnRun.role`）。
    pub role: String,
    /// 目的：这一回合坐在哪个**流程段**（standard / discuss / execute / proxy）：loop 来源按 (段, 角色) 查角色位。
    pub loop_id: String,
    /// 目的：这场会话跑在哪种 guest 里（本机档 = None）——工作环境块按它选 guest 档，路径按 guest 拼法渲染。
    pub guest: Option<crate::units::business::sandbox::api::GuestOs>,
}

impl SessionParams {
    /// 目的：从（agent、沙箱、模块清单）装配：沙箱给真实根与模块目录，模块清单给能力包。 这是**唯一**的装配口径——会话的建立与重建都走它，参数不会两处各拼一套。
    pub fn from_workspace(
        agent: &str,
        sb: &crate::units::business::workspace::api::Sandbox,
        modules: &[crate::units::business::catalog::modules::api::Module],
        loop_id: &str,
        role: &str,
    ) -> SessionParams {
        SessionParams {
            agent: agent.to_string(),
            work_name: sb.work_name.clone(),
            shared: sb.shared.clone(),
            private: sb.private.clone(),
            module_dirs: sb.modules.clone(),
            chara: String::new(),
            modules: modules
                .iter()
                .map(|m| ModulePrompt {
                    id: m.manifest.id.clone(),
                    brief: m.manifest.brief.clone(),
                    system: m.manifest.system.clone(),
                })
                .collect(),
            seat_system: None,
            role: role.to_string(),
            loop_id: loop_id.to_string(),
            guest: None,
        }
    }

    /// 目的：接上这个 agent 的人设（建会话时从快照带入；空 = 不产生人设块）。
    pub fn with_chara(mut self, chara: String) -> SessionParams {
        self.chara = chara;
        self
    }

    /// 目的：把"这场会话跑在哪种 guest 里"接上（本机档传 None）——工作环境块与路径拼法按它选。
    pub fn with_guest(
        mut self,
        guest: Option<crate::units::business::sandbox::api::GuestOs>,
    ) -> SessionParams {
        self.guest = guest;
        self
    }

    /// 目的：@ 改写要用的根——共享区 + 本席私有沙箱 + 本会话的路径翻译器。
    /// 约束：与工作环境块同一套渲染——虚拟机档用 `PathBridge::for_session`，本机档用视图盘的路径码；
    ///   视图盘没开时给恒等桥（宿主写法，与旧行为一致），@ 根始终带翻译器。
    pub(crate) fn ref_roots(&self) -> crate::units::business::prompt::api::RefRoots {
        let path_codec: Option<std::sync::Arc<dyn crate::kernel::ports::PathCodec>> = match self
            .guest
        {
            Some(g) => Some(std::sync::Arc::new(
                crate::kernel::api::path_bridge::PathBridge::for_session(
                    std::env::consts::OS,
                    Some(g.dialect()),
                ),
            )),
            None => crate::kernel::api::view_drive::view_codec().or_else(|| {
                Some(
                    std::sync::Arc::new(crate::kernel::api::path_bridge::PathBridge::for_session(
                        std::env::consts::OS,
                        None,
                    )) as std::sync::Arc<dyn crate::kernel::ports::PathCodec>,
                )
            }),
        };
        crate::units::business::prompt::api::RefRoots {
            work: self.shared.clone(),
            private: Some(self.private.clone()),
            path_codec,
        }
    }

    /// 目的：身份块：**每次调用现渲染**（模板与文案取当前提示词册，通道形态取当前登记处）。
    pub fn identity(
        &self,
        prompt: &dyn crate::units::business::prompt::api::Prompt,
        mode: crate::units::pure::registry_model::api::ToolMode,
    ) -> String {
        let env = env_block(prompt, self);
        match self.seat_system {
            // 席位身份（代理这类不是 agent 的核心）：同一份机制 / 环境 / 调用约定口径。
            Some(seg) => seat_system(prompt, seg, &self.agent, &env, mode, &self.role),
            None => agent_system(
                prompt,
                &self.agent,
                &self.chara,
                &self.modules,
                &env,
                mode,
                &self.role,
            ),
        }
    }
}

/// 目的：把一个宿主路径渲染成这场会话的环境写法：本机档用视图盘编码（没有就是恒等），虚拟机档用桥的 guest 拼法。
/// 约束：纯函数；编码器由参数注入，两个分支都能在不碰全局视图盘的情况下验收。
pub(crate) fn env_path(
    view: Option<&dyn crate::kernel::ports::PathCodec>,
    bridge: &crate::kernel::api::path_bridge::PathBridge,
    path: &std::path::Path,
) -> String {
    match view {
        Some(codec) => crate::kernel::api::path::slash(std::path::Path::new(&codec.to_view(path))),
        None => crate::kernel::api::path::slash(std::path::Path::new(&bridge.to_guest(path))),
    }
}

/// 目的：**工作环境块**：提示词册 env 渲染（真实根目录 + 路径规矩）。**不含任何工具清单**—— 能用哪些工具由核心按这一回合的身份现渲染后随回合注入（见 collab 的 `MemberTools::tools_block`）。  归属：它渲染的就是 `SessionParams`，所以随会话走（留在 `tools` 会让 `tools → session` 成环）。
pub fn env_block(
    prompt: &dyn crate::units::business::prompt::api::Prompt,
    p: &SessionParams,
) -> String {
    use crate::units::business::prompt::api::Segment;
    // 路径按**这场会话的环境**渲染：本机档 = 视图盘打开时的视图写法（宿主路径由路径码翻译），否则宿主拼法；
    // 虚拟机档 = guest 拼法（与挂载、命令装配同一座桥）。
    let view = if p.guest.is_none() {
        crate::kernel::api::view_drive::view_codec()
    } else {
        None
    };
    let bridge = crate::kernel::api::path_bridge::PathBridge::for_session(
        std::env::consts::OS,
        p.guest.map(|g| g.dialect()),
    );
    let in_env = |path: &std::path::Path| -> String { env_path(view.as_deref(), &bridge, path) };
    let module_roots = if p.module_dirs.is_empty() {
        "成员模块目录：无".to_string()
    } else {
        format!(
            "成员模块目录（只读）：\n{}",
            p.module_dirs
                .iter()
                .map(|(id, root)| format!("- {}：{}", id, in_env(root)))
                .collect::<Vec<_>>()
                .join("\n")
        )
    };
    let sandbox_root = in_env(&p.private);
    // 路径写法只有一处（册子里的 path_rules）：它自己也要用真实根渲染出例子，再插进环境块。
    let path_rules = prompt.render(
        Segment::PathRules,
        &[("sandbox_root", sandbox_root.clone())],
    );
    // 工作环境按**环境**分发：本机档按宿主平台选；虚拟机档按 guest 的 os 选。
    prompt.env(
        std::env::consts::OS,
        p.guest.map(|g| g.as_str()),
        &[
            ("work_name", p.work_name.clone()),
            ("agent", p.agent.clone()),
            ("work_root", in_env(&p.shared)),
            ("sandbox_root", sandbox_root),
            ("module_roots", module_roots),
            ("path_rules", path_rules),
        ],
    )
}

/// 目的：跑**一个回合**要的那几样（单 agent / 节点 / 讨论成员共用同一条轮循环，差别只在这里）： 身份块（谁）、席位（能调什么）、本回合提示（说什么）、表态约定、回合号（行按它对上）。
pub struct TurnRun<'a> {
    /// 目的：本回合的身份块（驱动按当前提示词册现渲染；不进对话）。
    pub identity: &'a str,
    /// 目的：本回合坐的流程段与角色 id（工具面按它们现拼）——**每回合必给**。
    ///   讨论席 / 节点按回合给；单 agent 给会话自己的那一段与角色。
    pub loop_id: &'a str,
    pub role: &'a str,
    /// 目的：本回合的提示（讨论席的开场/轮转词；执行席常为空——它的指令在派发行里）。
    pub turn: Vec<Msg>,
    /// 目的：本回合认不认**协作表态**（讨论席认；执行席不认）。
    pub verbs: bool,
    /// 目的：本回合的回合号（转录行按它对上）；None = 用该轮自己的回复号（单 agent 的每轮各成回合）。
    pub turn_id: Option<u64>,
}
