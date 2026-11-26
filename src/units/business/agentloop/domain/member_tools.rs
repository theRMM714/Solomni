//! 目的：会话的成员工具面——一个成员这一回合能调什么（内置工具 + 自己模块的外部工具）。
//! 管：`MemberTools`（工具面 + 观察账本 + sandbox 根）、只含值的 `ToolFace`、核实用例的端口 `CoreVerify`、按模块汇总的工具表（`tool_table` / `service_ops_for_modules`）与 shell 现场视图。
//! 不管：工具总表与席位声明（在 tools 能力与 `session::domain::mechanism`）；工具执行（在 `units::business::tools`）。
//! 联动：由 `session.rs` 装配进会话；执行经 `crate::units::business::tools::api::ToolExec`。

use crate::kernel::api::clock;
use crate::units::business::tools::api::{ShellFacts, ToolExec};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

/// 目的：shell 最近一次运行的状态（前端状态栏卡片用）：命令、起止、退出码与回执预览。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ShellLive {
    /// 目的：命令原文。
    pub command: String,
    /// 目的：cwd（私有沙箱）。
    pub cwd: String,
    /// 目的：开始时间（Unix 秒）。
    pub started_at: u64,
    /// 目的：结束时间；None = 仍在跑。
    pub finished_at: Option<u64>,
    /// 目的：本次执行耗时（秒）；运行中为 None。实况取自结构化事实，回放也读同一份事实。
    #[serde(default)]
    pub elapsed_secs: Option<u64>,
    /// 目的：本次耗时里围栏准备占的秒数（本机档首调用写 ACL 常常是大头）；不足一秒 / 虚拟机档为 None。
    #[serde(default)]
    pub fence_prep_secs: Option<u64>,
    /// 目的：退出码（超时 / 停止 / 拿不到时为 None）。
    pub exit_code: Option<i32>,
    /// 目的：结束状态（exit / timeout / stopped / signal）；None = 运行中、没执行或没跑成。
    #[serde(default)]
    pub status: Option<String>,
    /// 目的：成败；None = 仍在跑。
    pub ok: Option<bool>,
    /// 目的：回执预览（最多 4000 字符；完整回执在转录的 shell 工具行）。
    pub output: String,
    /// 目的：true = 从转录派生（回放 / 重建），不是本次进程的运行槽：此时没有起止墙钟时间，
    ///   耗时与 cwd 取自工具行的结构化事实，不解析回执正文。
    #[serde(default)]
    pub historical: bool,
}

/// 目的：一个会话的 shell 实时状态槽——工具循环写、状态栏快照读。
#[derive(Debug, Default)]
pub struct ShellSlot(std::sync::Mutex<Option<ShellLive>>);

impl ShellSlot {
    /// 目的：开始一次 shell：覆盖上一次的状态，等到 finish 才写结束时间 / 退出码。
    pub fn start(&self, command: &str, cwd: &str) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(ShellLive {
            command: command.to_string(),
            cwd: cwd.to_string(),
            started_at: clock::now_secs(),
            finished_at: None,
            elapsed_secs: None,
            fence_prep_secs: None,
            exit_code: None,
            status: None,
            ok: None,
            output: String::new(),
            historical: false,
        });
    }

    /// 目的：收尾一次 shell：写结束时间、成败与回执预览（超过 4000 字符截断）。
    /// 参数：`facts` = 执行层当场算好的结构化事实（耗时 / 围栏准备 / 退出码 / 状态）；
    ///   `None` = 这次没走到执行层（如围栏拒绝），退出码与状态如实留空、耗时回落本次墙钟。
    pub fn finish(&self, facts: Option<&ShellFacts>, ok: bool, output: &str) {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        let Some(s) = g.as_mut() else {
            return;
        };
        let end = clock::now_secs();
        s.finished_at = Some(end);
        match facts {
            Some(facts) => {
                s.elapsed_secs = Some(facts.elapsed_secs());
                s.fence_prep_secs = facts.fence_prep_secs();
                s.exit_code = facts.exit_code;
                s.status = facts.status.clone();
            }
            None => {
                s.elapsed_secs = Some(end.saturating_sub(s.started_at));
                s.fence_prep_secs = None;
                s.exit_code = None;
                s.status = None;
            }
        }
        s.ok = Some(ok);
        s.output = output.chars().take(4000).collect();
    }

    /// 目的：整体替换快照（重建 / 回档按转录回填；None = 清掉）。
    pub fn set(&self, live: Option<ShellLive>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = live;
    }

    /// 目的：读一份快照；没有运行过 shell 就是 None。
    pub fn get(&self) -> Option<ShellLive> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

/// 目的：从转录派生最近一次 shell 的状态（回放 / 重建用）：命令、cwd、耗时、成败、退出码与回执预览。
/// 约束：只读工具行的结构化字段（tool.name / tool.args / tool.shell / tool.output），不解析正文；
///   跳过回档折叠的行；参数里没有命令（调用没解析出来）或没有结构化事实的行跳过。
pub fn last_shell(events: &[serde_json::Value]) -> Option<ShellLive> {
    let marks = super::rewind::rewind_marks(events);
    let mut found = None;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for line in lines {
            let id = line.get("id").and_then(|i| i.as_u64()).unwrap_or(0);
            if super::rewind::folded(&marks, id) {
                continue;
            }
            let Some(tool) = line.get("tool") else {
                continue;
            };
            if tool.get("name").and_then(|n| n.as_str()) != Some("shell")
                || !tool
                    .get("module")
                    .and_then(|m| m.as_str())
                    .unwrap_or("")
                    .is_empty()
            {
                continue;
            }
            let output = tool.get("output").and_then(|o| o.as_str()).unwrap_or("");
            let args: serde_json::Value =
                serde_json::from_str(tool.get("args").and_then(|a| a.as_str()).unwrap_or(""))
                    .unwrap_or_default();
            let command = args
                .get("command")
                .and_then(|c| c.as_str())
                .unwrap_or("")
                .to_string();
            if command.is_empty() {
                continue;
            }
            // 结构化事实是唯一来源：没有它的行不算「最近一次 shell」。
            let Some(facts) = tool
                .get("shell")
                .and_then(|f| serde_json::from_value::<ShellFacts>(f.clone()).ok())
            else {
                continue;
            };
            found = Some(ShellLive {
                command,
                cwd: facts.cwd.clone(),
                started_at: 0,
                finished_at: None,
                elapsed_secs: Some(facts.elapsed_secs()),
                fence_prep_secs: facts.fence_prep_secs(),
                exit_code: facts.exit_code,
                status: facts.status.clone(),
                ok: Some(tool.get("ok").and_then(|o| o.as_bool()).unwrap_or(false)),
                output: output.chars().take(4000).collect(),
                historical: true,
            });
        }
    }
    found
}

/// 目的：一个模块的外部工具环境：模块目录（外部工具进程的 cwd）+ 它声明的工具表 + 已启动常驻服务的操作。
///   cwd 必须落在声明它的模块里（命令形如 python tools/x.py，是相对模块根写的）。
#[derive(Debug, Clone)]
pub struct ModuleTools {
    pub root: PathBuf,
    /// 目的：工具名 → 启动命令（模块作者在 module.yaml 的 tools.<名字>.command 里声明）。
    pub commands: BTreeMap<String, String>,
    /// 目的：工具名 → 参数契约（只含**声明了** params 的工具；没声明的工具不校验、不进提示词）。
    pub books: BTreeMap<String, crate::units::business::tools::api::ToolSchema>,
    /// 目的：声明了 parallel 的工具名（只读、无副作用；同一回复里的多个可并发调用会真的并发跑）。
    pub parallel: BTreeSet<String>,
    /// 目的：已启动常驻服务的操作——工具名（`<服务>.<操作>`）→ 它属于哪个服务的哪个操作；
    ///   由 runtime 装配工具面时按**已就绪**的服务现填（清单里只有声明，跑起来才有操作）。
    pub services: BTreeMap<String, ServiceOp>,
}

/// 目的：一个已启动常驻服务提供的操作（模型侧按 `<服务>.<操作>` 寻址）。
#[derive(Debug, Clone)]
pub struct ServiceOp {
    /// 目的：模型侧的工具名（`<服务>.<操作>`），也是它在模块工具面里的键。
    pub tool: String,
    /// 目的：该模块里的服务名（module.yaml 的 services.<名>）。
    pub service: String,
    /// 目的：服务自己发现的操作名。
    pub op: String,
    /// 目的：给人/模型看的一句话说明（服务给的，可能为空）。
    pub description: String,
    /// 目的：参数契约——服务给的 **JSON Schema 原样**（可能为空）；核心不校验，只声明给模型。
    pub parameters: Option<serde_json::Value>,
}

/// 目的：成员工具面里**只有值**的那一半：来自 module.yaml（按模块分组的放行表）+ 本成员的沙箱与身份。
///   无端口、无锁、无共享状态——只读工具面的函数收它，装配与执行仍走 `MemberTools`。
#[derive(Debug, Clone)]
pub struct ToolFace {
    /// 目的：这条通道的工具调用形态：envelope = 手写信封（任何供应商都能用）；native = 供应商结构化槽位。
    ///   **两套互斥**：native 就不解析信封、正文里的信封也不执行（但如实记失败行）。
    pub mode: crate::units::pure::registry_model::api::ToolMode,
    /// 目的：模块 id → 该模块的（目录, 工具表）；内置 read/write 不走这里。
    pub modules: BTreeMap<String, ModuleTools>,
    /// 目的：本会话的执行档位——工具执行者据此决定在本机跑还是送进 guest。
    pub tier: crate::units::pure::exec_model::api::Tier,
    /// 目的：虚拟机档要用的 **guest 租约**（空 = 本机档 / 没有 guest）：共享 guest 时它是作用域根工作名
    ///   （proxy 树 = 代理根名）；主机围栏仍用 `fence.lease`（子会话名）。
    pub guest_lease: String,
    /// 目的：本成员的沙箱：内置文件工具的寻址与越界依据（权限收口在 runtime）。
    pub sandbox: crate::units::business::workspace::api::Sandbox,
    /// 目的：工具说明块的标题模板（来自提示词段 `tools_this_turn`；变量 `{{tools}}`）。
    /// 约束：只存指令模板；运行时回执由各自产生它们的代码返回。
    pub tools_this_turn: String,
    /// 目的：内置工具的参数契约（来自 `systools/tools.yaml` 的 tools）：说明与校验都按它来。
    ///   它属于**工具面**，不属于沙箱——沙箱只管路径。
    pub builtin_tools: crate::units::business::tools::api::ToolBook,
    /// 目的：模块 id → 它缺的运行包能力（本档位下该模块的工具不执行；空表 = 都能执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 目的：本成员工具进程的围栏（可达范围 + 断网）：策略在 runtime 派生，机制在 ProcessRunner 适配层安装。
    pub fence: crate::kernel::api::fence::FencePlan,
    /// 目的：模块 id → 该模块隐私字段的注入项（env 名 → 值）；起进程时只走环境，不进命令行。
    pub module_env: std::collections::BTreeMap<String, Vec<(String, String)>>,
    /// 目的：本机档运行包的生效 PATH（由 packages 计划给出；虚拟机档 / 没有包 = None）。
    pub runtime_path: Option<std::ffi::OsString>,
    /// 目的：shell 的只读工具链根：项目内基础 profile + 该 agent 模块的运行包根。
    /// 约束：只进 shell 的 per-call 围栏；模块工具的放行仍走它自己的命令解释器基线。
    pub runtime_roots: Vec<PathBuf>,
    /// 目的：这一回合**可以调的系统工具 id**（由会话装配机的 `assemble` 按来源合成）。
    ///   存在的理由：把"谁能用哪些工具"变成**校验**，而不是提示词里的一句话。
    pub allowed: Vec<String>,
    /// 目的：这一回合在哪个流程段（loop 来源按 (段, 角色) 查角色位的机制动作）。
    pub loop_id: String,
    /// 目的：这一回合的角色 id（流程段声明的角色）——工具调用的审计记录用它当调用者身份。
    pub role: String,
    /// 目的：会话类型声明挂哪些工具来源（换席时装配机按它造来源实例）。
    pub source_ids: Vec<String>,
    /// 目的：会话 meta 里的 system 工具 id（会话类型展开，成员回合为空）。
    pub system_tools: Vec<String>,
    /// 目的：这个 agent 的能力组（读面 / 工作面 / 执行面按它发）。
    pub abilities: Vec<String>,
    /// 目的：会话类型 id（诊断与装配用）。
    pub profile: String,
    /// 目的：这个回合**挂没挂模块来源**（挂不挂由会话类型决定，与角色无关）。
    pub with_modules: bool,
    /// 目的：工具说明块的素材（patch 语法 / 模块工具 / 模块工具参数）：装配期算一次，随回合注入。
    pub notes: crate::units::business::tools::api::ToolNotes,
}

/// 目的：成员的工具执行环境：工具面（值）+ 注入的执行端口 + 本成员的活状态。
pub(crate) struct MemberTools {
    /// 目的：工具面（值部分）：这一回合的工具面与身份（见 `ToolFace`）。
    pub face: ToolFace,
    /// 目的：llm 用例面（**不持它的端口**，R12）：手写信封不合法时问它能不能按无歧义的写法修好。
    pub llm: Arc<dyn crate::units::business::llm::api::Llm + Send + Sync>,
    /// 目的：运行日志：模型输出被长度截断这类"看不见的事实"要落盘，供事后确定问题。
    pub log: Arc<dyn crate::kernel::ports::Log + Send + Sync>,
    /// 目的：工具执行面（**不持它的端口**，R12）：跑外部/内置工具都走它。
    pub tools: Arc<dyn ToolExec + Send + Sync>,
    /// 目的：虚拟机档的 guest 用例面（**不持它的端口**，R12）：guest 执行走它。
    pub vm: Arc<dyn crate::units::business::vm::api::Vm + Send + Sync>,
    /// 目的：常驻服务的统一管理面——模块的 `services` 操作经它调用（**不持它的端口**，R12）。
    pub residents: Arc<dyn crate::units::business::residents::api::ResidentOps + Send + Sync>,
    /// 目的：工作区用例面——写工具落地后把沙箱改动记进**该 agent 的存档线**（**不持它的端口**，R12）。
    /// 约束：None = 这一席没有工作区（核心核实用途、部分测试装配）；有工作区的席位必须给 Some，
    ///   否则写入只落在沙箱、共享区永远不动（静默漏提交）。
    pub workspace: Option<Arc<dyn crate::units::business::workspace::api::Workspace + Send + Sync>>,
    /// 目的：本次会话的观察账本（哪些文件完整读过 / 由核心写过）：改动前的证据（见 crate::units::business::tools::api::Observations）。
    pub observations: crate::units::business::tools::api::Observations,
    /// 目的：这个会话最近一次 shell 的实时状态（状态栏卡片）；没有 shell 的席位为 None。
    pub shell_live: Option<Arc<ShellSlot>>,
    /// 目的：**回复 id 计数器**：一次模型回复一个号，跨重启单调（重建时按转录里的最大值续号）。
    ///   转录行靠它分组（哪几行属于同一次回复），会话靠它按回复原子回档。
    pub reply_seq: u64,
    /// 目的：**当前行锚**：值 = 下一条转录行的 id。会话每生成一行就推进它；工具执行者（如 work_commit）
    ///   据此把共享区提交点钉到那一行，回档才能按行精确物化共享区。
    pub line: Arc<std::sync::atomic::AtomicU64>,
    /// 目的：**核心自有工具**的执行者（按名字认领）：内置与模块之外的第三类不再是特例——
    ///   循环只问“这一回合的工具面里有没有它、谁认领它”（见 `kernel::ports::ToolHandler`）。
    ///   空 = 这一席没有这类工具（讨论席与普通执行席都是空）。
    pub handlers: Vec<Arc<dyn crate::kernel::ports::ToolHandler>>,
    /// 目的：这个成员的模块：换席时现拼工具面要用，与席位无关（席位只决定发不发模块工具）。
    pub modules: Vec<crate::units::business::catalog::modules::api::Module>,
    /// 目的：提示词面（换席时现拼说明块的素材）：**不持它的端口**（R12）。
    pub prompt: Arc<dyn crate::units::business::prompt::api::Prompt>,
}

/// 目的：核心操作回路里「先核实」的那半个环境——判定一个名字是不是只读核实工具、把它跑一次。
///   契约只收自己的用法（R16：api 面不收活对象）：调用方递 `&mut MemberTools` 的实现，
///   `session::api::core_operation` 与 `slate::api::Request` 的公共签名里只出现这一个端口。
pub trait CoreVerify {
    /// 目的：这个名字是不是本席可用的只读核实工具（读面那三个内置工具之一）。
    fn is_readonly(&self, name: &str) -> bool;
    /// 目的：跑一次只读核实工具，回执行回执（读到的东西进本席观察账本）。
    fn run_readonly(
        &mut self,
        name: &str,
        args: &str,
    ) -> crate::units::business::tools::api::ToolOutcome;
}

impl CoreVerify for MemberTools {
    fn is_readonly(&self, name: &str) -> bool {
        use crate::units::business::tools::api::{GREP, LIST, READ};
        matches!(name, READ | LIST | GREP)
            && self.face.builtin_tools.contains_key(name)
            && self.face.allowed.iter().any(|n| n == name)
    }

    fn run_readonly(
        &mut self,
        name: &str,
        args: &str,
    ) -> crate::units::business::tools::api::ToolOutcome {
        let done = self.tools.run_builtin(
            &self.face.sandbox,
            &self.face.builtin_tools,
            std::mem::take(&mut self.observations),
            None,
            crate::units::business::tools::api::ResultBudget::full(),
            name,
            args,
        );
        self.observations = done.observations;
        done.outcome
    }
}

impl MemberTools {
    /// 目的：把这一回合的活工具面**换成另一段里的另一个角色**（换身份的唯一入口）：发放名单、模块工具表、
    ///   说明块与共享区工具执行者都交给会话的装配机现拼，调用方不得自己改 `face.allowed`。
    /// 约束：身份没变就原样返回；未知段或角色由装配机退化成空面（装配期已由 `loop_check::problems` 拦下）。
    pub(crate) fn enter_identity(&mut self, loop_id: &str, role: &str) {
        if self.face.loop_id == loop_id && self.face.role == role {
            return;
        }
        let next = super::assemble::assemble(super::assemble::FaceInput {
            loop_id,
            role,
            agent: &self.face.sandbox.agent,
            workspace_name: Some(self.face.sandbox.work_name.as_str()).filter(|n| !n.is_empty()),
            abilities: &self.face.abilities,
            profile: &self.face.profile,
            source_ids: &self.face.source_ids,
            system_tools: &self.face.system_tools,
            modules: &self.modules,
            sandbox: &self.face.sandbox,
            prompt: &*self.prompt,
            residents: &*self.residents,
            workspace: self.workspace.as_ref(),
        });
        self.face.allowed = next.allowed;
        self.face.modules = next.modules;
        self.face.with_modules = next.with_modules;
        self.face.notes = next.notes;
        self.face.loop_id = loop_id.to_string();
        self.face.role = next.role;
        self.handlers = next.handlers;
    }

    /// 目的：取下一个回复 id（一次模型回复调用一次）。
    pub(crate) fn next_reply(&mut self) -> u64 {
        self.reply_seq += 1;
        self.reply_seq
    }

    /// 目的：**本回合的工具说明块**：核心按这一回合的身份（ids）现渲染，只列这一回合真能调的。
    ///   为什么不是系统提示里的整本总表：模型会照着给的清单去调工具，列出必然被拒的等于请它去撞墙；
    ///   总表只该留在核心手里当校验判据（见 docs/tools/tools-and-roles.md 二、三之二）。
    ///   为什么随回合：同一个 agent 会话会用两种身份干活（说话 / 干活），能用的工具随回合变。
    ///   空串 = 这一回合没有可用工具（调用方不注入空块）。
    pub(crate) fn tools_block(&self, ids: &[String], with_modules: bool) -> String {
        // 原生通道的工具面由**声明槽**下发（名字 / 说明 / JSON Schema），这里不再重复文字清单：
        // 只保留声明表达不了的内容（patch 的自由格式写法）。信封通道仍由这段文字承载全部工具说明。
        let native = self.face.mode == crate::units::pure::registry_model::api::ToolMode::Native;
        let mut parts: Vec<String> = Vec::new();
        if !native {
            for id in ids {
                if let Some(schema) = self.face.builtin_tools.get(id) {
                    // 环境参数只属于人经呈现层：核心代理的 create_session 工具面不列 tier / guest。
                    let text = if self.face.role == "core_proxy" && id == "create_session" {
                        schema.render_for_prompt_hiding(&["tier", "guest"])
                    } else {
                        schema.render_for_prompt()
                    };
                    parts.push(format!("{}\n{}", id, text));
                }
            }
        }
        // patch 是自由格式工具：它不在参数清单里，写法跟一段补丁正文（只有拿到它的席位才给）。
        if ids
            .iter()
            .any(|i| i == crate::units::business::tools::api::PATCH)
        {
            parts.push(self.face.notes.patch_guide.clone());
        }
        if with_modules && !native {
            parts.push(self.face.notes.module_tools.clone());
            parts.push(self.face.notes.module_tool_params.clone());
        }
        if parts.is_empty() {
            return String::new();
        }
        self.face
            .tools_this_turn
            .replace("{{tools}}", &parts.join("\n"))
    }
}

/// 目的：已就绪服务**只保留这个 agent 工具面里的模块**——服务清单是全局的，
///   未分配给它的模块服务不该出现在提示词里（写了也调不动，只会把模型带偏）。
pub fn service_ops_for_modules(
    started: BTreeMap<String, Vec<ServiceOp>>,
    modules: &[crate::units::business::catalog::modules::api::Module],
) -> BTreeMap<String, Vec<ServiceOp>> {
    let allowed: BTreeSet<&str> = modules.iter().map(|m| m.manifest.id.as_str()).collect();
    started
        .into_iter()
        .filter(|(id, _)| allowed.contains(id.as_str()))
        .collect()
}

/// 目的：放行表：模块 id → 该模块的（目录, 工具表）。
///   **包含没有声明任何工具的模块**（命令表为空）——这样报错能区分「没有这个模块」与「这个模块没有这个工具」。
///   跨模块同名工具不再冲突：模块内名字唯一由 map 保证，跨模块由信封里的 module 消歧。
pub fn tool_table(
    modules: &[crate::units::business::catalog::modules::api::Module],
) -> BTreeMap<String, ModuleTools> {
    modules
        .iter()
        .map(|m| {
            (
                m.manifest.id.clone(),
                ModuleTools {
                    root: m.root.clone(),
                    commands: m
                        .manifest
                        .tools
                        .iter()
                        .map(|(name, decl)| (name.clone(), decl.command.clone()))
                        .collect(),
                    books: m
                        .manifest
                        .tools
                        .iter()
                        .filter_map(|(name, decl)| decl.schema().map(|s| (name.clone(), s)))
                        .collect(),
                    parallel: m
                        .manifest
                        .tools
                        .iter()
                        .filter(|(_, decl)| decl.parallel)
                        .map(|(name, _)| name.clone())
                        .collect(),
                    // 清单里只有声明；操作是服务跑起来才发现的，由 runtime 装配时填。
                    services: BTreeMap::new(),
                },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn transcript(lines: serde_json::Value) -> serde_json::Value {
        json!({ "type": "transcript", "lines": lines })
    }

    #[test]
    fn last_shell_reads_command_cwd_and_elapsed_from_the_tool_row() {
        let events = vec![transcript(json!([
            {"id": 0, "tool": {"name": "shell", "module": "", "ok": true,
             "args": r#"{"command":"first"}"#, "output": "x"}},
            {"id": 1, "tool": {"name": "shell", "module": "", "ok": true,
             "args": r#"{"command":"npm test"}"#,
             "output": "ok",
             "shell": {"cwd": "/w/a", "elapsed_ms": 7_000, "exit_code": 0, "status": "exit"}}}
        ]))];
        let live = last_shell(&events).expect("要派生最后一条");
        assert_eq!(live.command, "npm test");
        assert_eq!(live.cwd, "/w/a");
        assert_eq!(live.elapsed_secs, Some(7));
        assert_eq!(live.exit_code, Some(0));
        assert_eq!(live.status.as_deref(), Some("exit"));
        assert_eq!(live.ok, Some(true));
        assert!(live.historical);
        assert!(live.finished_at.is_none(), "派生快照不伪造墙钟时间");
    }

    #[test]
    fn last_shell_skips_folded_rows_and_other_tools() {
        let events = vec![
            transcript(json!([
                {"id": 0, "tool": {"name": "read", "module": "", "ok": true, "args": "{}", "output": "x"}},
                {"id": 1, "tool": {"name": "shell", "module": "", "ok": true,
                 "args": r#"{"command":"gone"}"#, "output": "",
                 "shell": {"cwd": "/w", "elapsed_ms": 1_000, "exit_code": 0, "status": "exit"}}}
            ])),
            json!({"type": "rewind", "mark": 1, "mode": "archive", "before": 0, "from": 2}),
            transcript(json!([
                {"id": 2, "tool": {"name": "shell", "module": "m", "ok": true,
                 "args": r#"{"command":"module"}"#, "output": "",
                 "shell": {"cwd": "/w", "elapsed_ms": 1_000, "exit_code": 0, "status": "exit"}}}
            ])),
        ];
        assert!(
            last_shell(&events).is_none(),
            "被折叠的 shell 与模块工具都不算"
        );
    }

    #[test]
    fn last_shell_keeps_a_refused_attempt_as_an_attempt() {
        let events = vec![transcript(json!([
            {"id": 0, "tool": {"name": "shell", "module": "", "ok": false,
             "args": r#"{"command":"cat ../x"}"#,
             "output": "",
             "shell": {"cwd": "/w", "elapsed_ms": 0}}}
        ]))];
        let live = last_shell(&events).expect("最近一次尝试");
        assert_eq!(live.ok, Some(false));
        assert_eq!(live.cwd, "/w", "落点是尝试的落点，不是结果");
        assert_eq!(live.elapsed_secs, Some(0));
        assert_eq!(live.exit_code, None);
        assert_eq!(live.status, None, "没执行的调用没有结束状态");
    }

    #[test]
    fn last_shell_reads_only_the_structured_facts() {
        let events = vec![transcript(json!([
            {"id": 0, "tool": {"name": "shell", "module": "", "ok": true,
             "args": r#"{"command":"npm test"}"#,
             "output": "ok",
             "shell": {"cwd": "/w/b", "elapsed_ms": 331_400,
                       "fence_prep_ms": 327_000, "exit_code": 0,
                       "status": "exit", "truncated": true}}}
        ]))];
        let live = last_shell(&events).expect("要派生");
        assert_eq!(live.cwd, "/w/b");
        assert_eq!(live.elapsed_secs, Some(331));
        assert_eq!(live.fence_prep_secs, Some(327));
        assert_eq!(live.exit_code, Some(0));
        assert_eq!(live.status.as_deref(), Some("exit"));
        assert!(live.historical);
    }

    #[test]
    fn last_shell_ignores_rows_without_structured_facts() {
        let events = vec![transcript(json!([
            {"id": 0, "tool": {"name": "shell", "module": "", "ok": true,
             "args": r#"{"command":"npm test"}"#,
             "output": "ok\n[shell] 退出码 0\n[shell] cwd /w/a · 耗时 7 秒"}}
        ]))];
        assert!(last_shell(&events).is_none(), "正文里的旧式尾行不再被解析");
    }

    #[test]
    fn last_shell_keeps_fence_prep_only_from_one_second() {
        let row = |ms: u64| {
            transcript(json!([
                {"id": 0, "tool": {"name": "shell", "module": "", "ok": true,
                 "args": r#"{"command":"x"}"#, "output": "ok",
                 "shell": {"cwd": "/w", "elapsed_ms": 2_000, "fence_prep_ms": ms}}}
            ]))
        };
        assert_eq!(last_shell(&[row(400)]).unwrap().fence_prep_secs, None);
        assert_eq!(last_shell(&[row(1_000)]).unwrap().fence_prep_secs, Some(1));
    }
}
