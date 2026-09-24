//! 协作引擎：建组 → 讨论 → 整理 → 执行 → 验收（纯状态机，不做输入输出）。
//! 状态机只认信封动词；发言内容永远是数据，不是指令。
//! 所有发给模型的文案经 core/prompt.rs 渲染自提示词册；成员拥有自己的会话通道。
//! 工具循环（联动 envelope::Verb::Tool 与 ports::ToolRunner）：策略（放行表、并发调度）在核心，机制在适配层。
//! 一次回复里的多个原生调用按**声明**调度：声明可并发的只读类并发跑，其余（含写入类）独占并按原序生效；
//! 结果与工具行一律按原始顺序回填——并发只影响执行，不影响上下文里的顺序。

use crate::core::envelope::{self, ToolInvoke, Verb};
use crate::core::events::{SessionEvent, ToolCallView};
#[cfg(test)]
use crate::core::ports::BoxedChat;
use crate::core::ports::{Chat, Chunk, CompleteOpts, Msg, ToolOutcome, ToolRunner};
use crate::core::prompt::Prompts;
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

/// 讨论轮次上限（超限交用户裁决——上限必生效）。
pub const MAX_ROUNDS: usize = 6;
/// 一轮内对同一个成员最多提醒几次（**内存驱动**用；生产按设置走）。
#[cfg(test)]
pub const MAX_DISCUSS_REMIND: u32 = 3;

/// 一次成员回合之后的处置（核心据此决定，见 docs/architecture/session-model.md 二）。
#[derive(Debug, PartialEq)]
pub enum AfterTurn {
    /// 已表态：正常往下走。
    Done,
    /// 没表态但还有提醒额度：**注入提醒后重问同一个人**（核心只提醒、不强制）。
    Remind,
    /// 没表态且提醒到顶：主会话记一行"未回应"，本轮放过它（**不阻塞整轮**）。
    Unanswered,
}

/// 一个模块的外部工具环境：模块目录（外部工具进程的 cwd）+ 它声明的工具表。
/// cwd 必须落在声明它的模块里（命令形如 python tools/x.py，是相对模块根写的）。
#[derive(Debug, Clone)]
pub struct ModuleTools {
    pub root: PathBuf,
    /// 工具名 → 启动命令（模块作者在 module.yaml 的 tools.<名字>.command 里声明）。
    pub commands: BTreeMap<String, String>,
    /// 工具名 → 参数契约（只含**声明了** params 的工具；没声明的工具不校验、不进提示词）。
    pub books: BTreeMap<String, crate::core::schema::ToolSchema>,
    /// 声明了 parallel 的工具名（只读、无副作用；同一回复里的多个可并发调用会真的并发跑）。
    pub parallel: BTreeSet<String>,
}

/// 放行表：模块 id → 该模块的（目录, 工具表）。
/// **包含没有声明任何工具的模块**（命令表为空）——这样报错能区分「没有这个模块」与「这个模块没有这个工具」。
/// 跨模块同名工具不再冲突：模块内名字唯一由 map 保证，跨模块由信封里的 module 消歧。
pub fn tool_table(modules: &[crate::core::module::Module]) -> BTreeMap<String, ModuleTools> {
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
                },
            )
        })
        .collect()
}

/// 成员的工具执行环境：来自 module.yaml（按模块分组的放行表）+ 注入的执行端口 + 本成员的沙箱。
pub struct MemberTools {
    /// 这条通道的工具调用形态：envelope = 手写信封（任何供应商都能用）；native = 供应商结构化槽位。
    /// **两套互斥**：native 就不解析信封、正文里的信封也不执行（但如实记失败行）。
    pub mode: crate::core::providers::ToolMode,
    /// 模块 id → 该模块的（目录, 工具表）；内置 read/write 不走这里。
    pub modules: BTreeMap<String, ModuleTools>,
    /// 本次会话的观察账本（哪些文件完整读过 / 由核心写过）：改动前的证据（见 systool::Observations）。
    pub observations: crate::core::systool::Observations,
    /// 信封修复端口：手写信封不合法时先问它能不能按无歧义的写法修好（默认只转义裸控制字符）。
    pub repair: Arc<dyn crate::core::ports::EnvelopeRepair + Send + Sync>,
    /// 运行日志：模型输出被长度截断这类"看不见的事实"要落盘，供事后确定问题。
    pub log: Arc<dyn crate::core::ports::Log + Send + Sync>,
    pub runner: Arc<dyn ToolRunner + Send + Sync>,
    /// 本成员的沙箱：内置文件工具的寻址与越界依据（权限收口在 core）。
    pub sandbox: crate::core::workspace::Sandbox,
    /// 内置文件工具的读写端口。
    pub io: Arc<dyn crate::core::ports::SysIo + Send + Sync>,
    /// 模块 id → 它缺的运行包能力（本档位下该模块的工具不执行；空表 = 都能执行）。
    pub unavailable: BTreeMap<String, Vec<String>>,
    /// 本成员工具进程的围栏（可达范围 + 断网）：策略在 core 派生，机制在 ToolRunner 适配层安装。
    pub fence: crate::core::fence::FenceSpec,
    /// **回复 id 计数器**：一次模型回复一个号，跨重启单调（重建时按转录里的最大值续号）。
    /// 转录行靠它分组（哪几行属于同一次回复），会话靠它按回复原子回档。
    pub reply_seq: u64,
    /// 这个席位**可以调的系统工具 id**（由角色表发放：讨论席 = discussant、执行席 = executor）。
    /// 存在的理由：把"谁能用哪些工具"变成**校验**，而不是提示词里的一句话。
    pub allowed: Vec<String>,
    /// 这个席位**能不能用它自己模块的工具**（角色表的 module_tools；执行席是，讨论席不是）。
    pub with_modules: bool,
    /// 工具说明块的素材（patch 语法 / 模块工具 / 模块工具参数）：装配期算一次，随回合注入。
    pub notes: crate::core::systool::ToolNotes,
}

impl MemberTools {
    /// 取下一个回复 id（一次模型回复调用一次）。
    fn next_reply(&mut self) -> u64 {
        self.reply_seq += 1;
        self.reply_seq
    }

    /// **本回合的工具说明块**：核心按这一回合的身份（ids）现渲染，只列这一回合真能调的。
    ///
    /// 为什么不是系统提示里的整本总表：模型会照着给的清单去调工具，列出必然被拒的等于请它去撞墙；
    /// 总表只该留在核心手里当校验判据（见 docs/architecture/tools-and-roles.md 二、三之二）。
    /// 为什么随回合：同一个 agent 会话会用两种身份干活（说话 / 干活），能用的工具随回合变。
    /// 空串 = 这一回合没有可用工具（调用方不注入空块）。
    pub(crate) fn tools_block(&self, ids: &[String], with_modules: bool) -> String {
        let mut parts: Vec<String> = Vec::new();
        for id in ids {
            if let Some(schema) = self.sandbox.builtin_tools.get(id) {
                parts.push(format!("{}\n{}", id, schema.render_for_prompt()));
            }
        }
        // patch 是自由格式工具：它不在参数清单里，写法跟一段补丁正文（只有拿到它的席位才给）。
        if ids.iter().any(|i| i == crate::core::systool::PATCH) {
            parts.push(self.notes.patch_guide.clone());
        }
        if with_modules {
            parts.push(self.notes.module_tools.clone());
            parts.push(self.notes.module_tool_params.clone());
        }
        if parts.is_empty() {
            return String::new();
        }
        self.sandbox.texts.render(
            &self.sandbox.texts.tools_this_turn,
            &[("tools", parts.join("\n"))],
        )
    }
}

/// 转录流水里用过的最大回复 id：重建时据此续号。
/// 为什么必须续号：回复 id 是分组依据，重复就会把新回复与旧回复并成一组。
pub fn max_reply(events: &[serde_json::Value]) -> u64 {
    let mut max = 0u64;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            if let Some(r) = l.get("reply").and_then(|x| x.as_u64()) {
                max = max.max(r);
            }
            if let Some(r) = l
                .get("tool")
                .and_then(|t| t.get("reply"))
                .and_then(|x| x.as_u64())
            {
                max = max.max(r);
            }
        }
    }
    max
}

/// 线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
pub type WireTools = BTreeMap<String, (Option<String>, String)>;

/// 一次请求要声明的工具表（给供应商的那一份）。
pub type Decls = Vec<crate::core::ports::ToolDecl>;

/// 本成员这次请求的**工具声明面**（原生通道用）：发给供应商的声明 + 线上名回译表 + 可并发的线上名。
#[derive(Default)]
struct ToolDecls {
    list: Decls,
    /// 线上名 → (模块 id, 工具名)：原生协议里没有 module 字段，跨模块同名工具靠它消歧。
    wire: WireTools,
}

/// 执行一次工具调用：内置优先；外部工具按模块走（模块为空时由 dispatch_external 如实报错）。
/// 账本经**分支副本**回到本成员（见 run_branch）——串行与并发只有这一条执行路径。
fn run_one(
    ctx: &mut MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
) -> (String, ToolOutcome) {
    let (label, outcome, branch) = run_branch(ctx, module, name, args_json);
    // 串行：分支的副本就是"这次调用之后"的账本，直接接管（与逐个记账等价）。
    ctx.observations = branch;
    (label, outcome)
}

/// 一次调用的执行体（**不碰本成员的账本**）：分支各持账本副本，由调用方按原始顺序合并/接管。
/// 为什么这样：并发批次里多个调用同时跑，而账本是可变状态；只读类调用的结果不依赖账本，
/// 所以"副本 + 按原序提交"与串行执行的结果完全相同（见 systool::Observations::absorb）。
fn run_branch(
    ctx: &MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
) -> (String, ToolOutcome, crate::core::systool::Observations) {
    let mut branch = ctx.observations.clone();
    let (label, outcome) = if crate::core::systool::is_builtin(name) {
        // **按角色表校验**：这个席位没有的工具，如实拒绝（不静默执行）。
        if !ctx.allowed.iter().any(|t| t == name) {
            (
                String::new(),
                crate::core::systool::refuse(&ctx.sandbox.texts, name),
            )
        } else {
            (
                String::new(),
                crate::core::systool::execute(
                    &ctx.sandbox,
                    ctx.io.as_ref(),
                    &mut branch,
                    name,
                    args_json,
                ),
            )
        }
    } else {
        let inv = ToolInvoke {
            malformed: None,
            module: module.map(|m| m.to_string()),
            name: name.to_string(),
            args_json: args_json.to_string(),
            body: String::new(),
            lead: String::new(),
        };
        dispatch_external(ctx, &inv)
    };
    (label, outcome, branch)
}

/// 这个工具有没有**声明可并发**（策略在册子/清单里，代码里不写名单）：
/// 内置工具看 `prompts/shared/tools.yaml` 的 `builtin_tools.<名字>.parallel`，模块工具看 `module.yaml` 的 `tools.<名字>.parallel`。
/// 未声明 = 独占串行；**没写 module 的外部工具也按独占**（那要等 dispatch 才知道是哪个模块，核心不猜）。
fn is_parallel(ctx: &MemberTools, module: Option<&str>, name: &str) -> bool {
    match module {
        Some(id) => ctx
            .modules
            .get(id)
            .map(|m| m.parallel.contains(name))
            .unwrap_or(false),
        None => {
            crate::core::systool::is_builtin(name)
                && ctx
                    .sandbox
                    .builtin_tools
                    .get(name)
                    .map(|s| s.parallel)
                    .unwrap_or(false)
        }
    }
}

/// 执行一批调用：**连续**声明可并发的合成一批并发跑，其余各自独占；结果按**原始下标**返回。
/// 原生通道与手写信封通道共用这一处调度——并发策略只有一份，两个通道不会各写一套。
/// 账本走分支副本 + 按原序合并（与串行执行等价，见 systool::Observations::absorb）。
fn run_batch(
    ctx: &mut MemberTools,
    plan: &[(Option<String>, String, String)],
) -> Vec<(String, ToolOutcome)> {
    let mut done: Vec<Option<(String, ToolOutcome)>> = (0..plan.len()).map(|_| None).collect();
    let mut i = 0;
    while i < plan.len() {
        if is_parallel(ctx, plan[i].0.as_deref(), &plan[i].1) {
            let mut j = i;
            while j < plan.len() && is_parallel(ctx, plan[j].0.as_deref(), &plan[j].1) {
                j += 1;
            }
            let batch: Vec<(String, ToolOutcome, crate::core::systool::Observations)> =
                std::thread::scope(|s| {
                    let handles: Vec<_> = plan[i..j]
                        .iter()
                        .map(|(module, tool, args)| {
                            s.spawn(|| run_branch(ctx, module.as_deref(), tool, args))
                        })
                        .collect();
                    handles
                        .into_iter()
                        .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                        .collect()
                });
            for (k, (label, outcome, branch)) in batch.into_iter().enumerate() {
                ctx.observations.absorb(&branch);
                done[i + k] = Some((label, outcome));
            }
            i = j;
        } else {
            let (module, tool, args) = &plan[i];
            done[i] = Some(run_one(ctx, module.as_deref(), tool, args));
            i += 1;
        }
    }
    done.into_iter()
        .map(|d| d.expect("每个调用都有执行结果"))
        .collect()
}

/// 本成员这次请求要声明的工具（原生通道用）：内置工具 + 模块工具。
/// 原生协议里**没有 module 字段**，所以跨模块同名工具靠线上名消歧（{模块}_{工具}，撞名再加序号）；
/// 返回的映射把线上名翻回（模块, 工具）。模块没声明参数的照旧声明（参数结构交回给工具自己解释）。
fn tool_decls(ctx: &MemberTools) -> ToolDecls {
    let mut decls = ToolDecls::default();
    let mut taken: Vec<String> = Vec::new();
    // **只声明这个席位拿到的工具**（判据与执行时校验同一份 allowed）：声明了却调不动没有意义，
    // 模型会照着声明去调，被拒一次就白烧一轮（见 docs/architecture/tools-and-roles.md 二）。
    for (name, schema) in &ctx.sandbox.builtin_tools {
        if !ctx.allowed.iter().any(|t| t == name) {
            continue;
        }
        // patch 是自由格式：它的声明单独写（参数是 body 字符串，不是 JSON 信封的 args）
        if crate::core::systool::is_freeform(name) {
            continue;
        }
        decls.list.push(schema.decl(name));
        taken.push(name.clone());
        decls.wire.insert(name.clone(), (None, name.clone()));
    }
    if ctx.allowed.iter().any(|t| t == crate::core::systool::PATCH) {
        let patch = crate::core::systool::patch_decl();
        taken.push(patch.name.clone());
        decls.wire.insert(
            patch.name.clone(),
            (None, crate::core::systool::PATCH.to_string()),
        );
        decls.list.push(patch);
    }
    // 模块工具按**成员归属**发放（不是角色属性）：拿不到模块工具的身份（讨论席）不声明它们。
    for (id, mt) in ctx.modules.iter().filter(|_| ctx.with_modules) {
        for tool in mt.commands.keys() {
            let mut wire_name = format!("{}_{}", id, tool);
            let mut n = 2;
            while taken.contains(&wire_name) {
                wire_name = format!("{}_{}_{}", id, tool, n);
                n += 1;
            }
            taken.push(wire_name.clone());
            decls
                .wire
                .insert(wire_name.clone(), (Some(id.clone()), tool.clone()));
            let decl = match mt.books.get(tool) {
                Some(schema) => schema.decl(&wire_name),
                // 没声明参数：如实说明参数由工具自己解释（不编 schema）
                None => crate::core::ports::ToolDecl {
                    name: wire_name.clone(),
                    description: format!("模块 {} 的外部工具 {}（参数由工具自己解释）", id, tool),
                    parameters: serde_json::json!({ "type": "object", "additionalProperties": true }),
                },
            };
            decls.list.push(decl);
        }
    }
    decls
}

/// 可用的外部工具清单：逐条列成「模块.工具」，末尾补上内置工具。
/// 三种失败（模块不存在 / 模块没这个工具 / 缺 module 且模块不唯一）都用它把可选范围说回去。
fn available_tools(ctx: &MemberTools) -> String {
    let mut list: Vec<String> = Vec::new();
    for (id, mt) in &ctx.modules {
        for name in mt.commands.keys() {
            list.push(format!("{}.{}", id, name));
        }
    }
    list.extend(crate::core::systool::names());
    list.join(&ctx.sandbox.texts.tool_list_separator)
}

fn deny(ctx: &MemberTools, why: String) -> ToolOutcome {
    let texts = &ctx.sandbox.texts;
    ToolOutcome {
        ok: false,
        output: texts.render(
            &texts.available_wrapper,
            &[("why", why), ("tools", available_tools(ctx))],
        ),
    }
}

/// 外部工具派发：先定模块（信封里的 module；省略时只有唯一模块才兜底），再查该模块的工具表。
/// 返回（实际使用的模块 id, 执行结果）。不猜：多模块 agent 下省略 module 直接如实报错。
fn dispatch_external(ctx: &MemberTools, inv: &ToolInvoke) -> (String, ToolOutcome) {
    let module = match inv.module.as_deref() {
        Some(m) => m.to_string(),
        None if ctx.modules.len() == 1 => ctx.modules.keys().next().cloned().unwrap_or_default(),
        None => {
            let why = ctx.sandbox.texts.render(
                &ctx.sandbox.texts.no_module_field,
                &[("tool", inv.name.clone())],
            );
            return (String::new(), deny(ctx, why));
        }
    };
    let Some(mt) = ctx.modules.get(&module) else {
        let why = ctx.sandbox.texts.render(
            &ctx.sandbox.texts.unknown_module,
            &[("module", module.clone())],
        );
        return (module.clone(), deny(ctx, why));
    };
    // 运行包未就绪（本档位下该模块的工具不执行）：如实报缺哪个能力，让模型换工具或告诉用户。
    if let Some(caps) = ctx.unavailable.get(&module) {
        let why = ctx.sandbox.texts.render(
            &ctx.sandbox.texts.module_unavailable,
            &[
                ("module", module.clone()),
                (
                    "capability",
                    caps.join(&ctx.sandbox.texts.tool_list_separator),
                ),
            ],
        );
        return (module.clone(), deny(ctx, why));
    }
    match mt.commands.get(&inv.name) {
        // 工具进程的工作目录 = 它所属模块的根目录；围栏按该模块的根收口。
        Some(command) => {
            // 模块声明了参数契约就按它校验（参数错了不必启动进程）：没声明就照旧把 args 原样交给工具。
            if let Some(book) = mt.books.get(&inv.name) {
                let full = format!("{}.{}", module, inv.name);
                match serde_json::from_str::<serde_json::Value>(&inv.args_json) {
                    Ok(args) => {
                        if let Err(fault) = book.check(&args) {
                            let why = crate::core::systool::arg_fault_text(
                                &ctx.sandbox.texts,
                                &full,
                                book,
                                &fault,
                            );
                            return (module.clone(), deny(ctx, why));
                        }
                    }
                    Err(e) => {
                        let why = ctx.sandbox.texts.render(
                            &ctx.sandbox.texts.bad_args_json,
                            &[("error", e.to_string())],
                        );
                        return (module.clone(), deny(ctx, why));
                    }
                }
            }
            (
                module,
                ctx.runner
                    .run(&ctx.fence.at(&mt.root), command, &inv.args_json),
            )
        }
        None => {
            let why = ctx.sandbox.texts.render(
                &ctx.sandbox.texts.module_lacks_tool,
                &[("module", module.clone()), ("tool", inv.name.clone())],
            );
            (module.clone(), deny(ctx, why))
        }
    }
}

pub struct Member {
    pub id: String,
    /// 会话参数：身份块由它**每回合现渲染**（不给成员存一份渲染好的文本）。
    pub params: crate::core::session::SessionParams,
    /// 该席位的通道形态（登记处派生）：身份块里的调用约定按它渲染。
    pub mode: crate::core::providers::ToolMode,
    /// 内存通道：**只有测试用**（生产里回合跑在各自的 agent 会话里，见 session-model.md 二之二）。
    #[cfg(test)]
    pub chat: Option<BoxedChat>,
    pub present: bool,
    pub agreed: bool,
    /// 工具环境；None = 本模块未声明工具（tool 信封按原文收录）。
    pub tools: Option<MemberTools>,
}

impl Member {
    /// 生产构造：成员不持有通道（驱动权在核心，回合在各自的会话里跑）。
    pub fn plain(
        id: &str,
        params: crate::core::session::SessionParams,
        mode: crate::core::providers::ToolMode,
    ) -> Member {
        Member {
            id: id.to_string(),
            params,
            mode,
            present: true,
            agreed: false,
            tools: None,
            #[cfg(test)]
            chat: None,
        }
    }

    /// 测试构造：带内存通道（配合 cfg(test) 的 open/step/member_turn 直接驱动讨论）。
    #[cfg(test)]
    pub fn new(
        id: &str,
        params: crate::core::session::SessionParams,
        mode: crate::core::providers::ToolMode,
        chat: BoxedChat,
    ) -> Member {
        Member {
            id: id.to_string(),
            params,
            mode,
            present: true,
            agreed: false,
            tools: None,
            chat: Some(chat),
        }
    }
}

pub enum TurnOut {
    /// 一轮正常走完，转达给用户过目。
    Round,
    /// 调用失败（超时 / 网络）：本轮**中断**——不把失败当发言吸收，交给用户决定何时继续。
    Interrupted(String),
    /// 用户点了「停止」：本轮**停止**——被中断的那条发言**不吸收**（半截发言进转录会把状态算歪）。
    Stopped,
    /// 有模块请教用户：轮转中止，等用户回答。
    AskUser { member: String, question: String },
    /// 留在组的成员全部同意 → 讨论终止。
    Done,
}

/// 讨论转录的一行：文本 + 该行是否"信封缺失、按发言原文收录"。
/// 降级是结构化信号（呈现层与后续判断都读它），文本里那句说明只给人和模型看。
#[derive(Debug, Clone)]
pub struct DiscLine {
    pub text: String,
    pub degraded: bool,
    /// 这一行属于哪个回合（讨论的一次发言回合；0 = 不属任何回合）。
    /// 两边的转录行靠它对齐（回档同步，见 docs/architecture/session-model.md 五）。
    pub turn: u64,
    /// 这一行是**系统消息**（系统注入的提醒/边界；不是用户说的，也不是模型说的）。
    pub system: bool,
    /// 这一行是"讨论回合里的一次核实"（只读工具调用）时带上调用视图；普通发言没有。
    /// 呈现层据此把核实行与发言行分开样式（与单 agent 的工具行同一形态）。
    pub tool: Option<ToolCallView>,
}

pub struct Discussion {
    pub members: Vec<Member>,
    pub transcript: Vec<DiscLine>,
    pub round: usize,
    /// 用户对 ask 的回答在此队列：先入先转达。
    pub pending_user_answers: Vec<String>,
    pub closed: bool,
    /// yes,allow：授权小组自裁——ask 不中止轮转，留档待办。
    pub allow_autonomy: bool,
    /// 提示词册（讨论文案来源）。
    prompts: Prompts,
    /// 本次调用的通道参数（流式 + 预算）：**全局设置**，与单 agent 共用同一份。
    llm: crate::core::ports::LlmOpts,
    /// 讨论席的**机制说明 + 讨论约定**（开场与轮转都带它）：只说约定不说机制，AI 会空转。
    /// 能用哪些工具**不在这里**——随回合注入（见 MemberTools::tools_block）。
    protocol: String,

    /// 这一轮对每个成员**提醒过几次**（到顶就记"未回应"放过它）；每轮开始归零。
    reminded: std::collections::BTreeMap<String, u32>,
    /// 上面那份计数属于第几轮（轮次变了就清空）。
    remind_round: usize,
    /// 「停止」标志：由 CollabSession 注入（它从任务登记处拿到）。
    /// 泵在**每次调用前**与**调用中途**都看它——所以停止能在一个模型调用内收尾，而不是等它跑完。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 开场提示词（渲染一次，开场阶段每个成员都用它）——由 start() 设。
    opener: String,
    /// 轮次推进的游标（可暂停：问到哪了）——见 advance/feed。
    cursor: Cursor,
    /// 本轮到此刻**还没交出去**的行数（轮次标记也算）：一个成员说完就把它那一批交出去。
    handed: usize,
}

/// 推进讨论的**一步**：该问谁（或终态）。
/// 泵只决定"该问谁"，**不自己调模型**——驱动权归核心（见 docs/architecture/session-model.md 二之二）。
pub enum Adv {
    /// 该问这个成员一回合：身份块 + 本回合提示交给驱动（它取会话、装配、跑模型）。
    /// 身份**每回合现渲染**（由成员的 params + 当前提示词册），不存在谁的会话里。
    Ask {
        i: usize,
        identity: String,
        turn: Vec<Msg>,
    },
    /// 开场问完了（可以进轮次了）。
    Opened,
    /// 终态 / 轮次结束（原样交回上层）。
    Out(TurnOut),
}

/// 轮次推进的游标（状态机可暂停：问到哪了）。
enum Cursor {
    /// 开场：问到第 i 个成员（开场不跳过任何人）。
    Opener(usize),
    /// 本轮还没开始：下一次 advance 写轮次标记与用户回答，再从第一个人问起。
    Fresh,
    /// 本轮问到第 i 个成员。
    At(usize),
}

/// 一个成员回合的结果：两套通道**统一形态**（上层不必再关心是哪条通道）。
pub struct MemberTurn {
    /// 这一回合的表态；**None = 没表态**（散文 / 空回执 / 越权 / 到顶）。
    /// 没表态**不投影主会话**：原文只落进它自己的会话；提醒由核心在边界注入。
    pub verb: Option<Verb>,
    pub text: String,
    pub degraded: bool,
    /// 这一轮的输出被供应商按长度截断了（如实标注，不假装完整）。
    pub truncated: bool,
    /// 这一回合里**核实**留下的行（只读工具调用）。
    /// 内存通道把它并进讨论转录；核心驱动时由核心落进**该 agent 自己的会话**。
    pub lines: Vec<DiscLine>,
}

impl Discussion {
    /// 内存通道的成员回合（薄包装）：借用本席位自己的 chat/tools，把核实行并进讨论转录。
    /// 核心驱动那条路直接调 turn_with，把核实行落进该 agent 自己的会话。
    #[cfg(test)]
    fn member_turn(
        &mut self,
        i: usize,
        identity: &str,
        turn: Vec<Msg>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<MemberTurn, String> {
        let opts = self.opts();
        let speaker = self.members[i].id.clone();
        let turn = {
            let m = &mut self.members[i];
            let Member { chat, tools, .. } = m;
            let chat = chat.as_mut().expect("测试通道");
            Self::turn_with(
                &self.prompts.systools,
                "discussant",
                &self.cancel,
                opts,
                &speaker,
                identity,
                &[],
                chat.as_mut(),
                tools.as_mut(),
                turn,
                sink,
            )?
        };
        self.transcript.extend(turn.lines.clone());
        Ok(turn)
    }

    /// 一个成员回合（**关联函数**：chat/tools 由调用方给——核心驱动时来自该 agent 的会话）。
    /// 允许**先核实**（只读文件工具 read/list/search），最后用**动词**表态。
    /// 两条通道共用这一条循环：native 从结构化槽位取调用，信封通道从正文里的信封取。
    /// 动词 = 发言（收尾）；只读工具 = 核实（执行 → 收进 lines → 结果回灌 → 继续，上限 MAX_TOOL_CALLS）；
    /// 其余工具一律**如实拒绝**（讨论回合拿不到干活的手段，见 face 字段）。
    #[allow(clippy::too_many_arguments)]
    pub fn turn_with(
        systools: &crate::core::roles::SystemTools,
        role: &str,
        cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
        opts: crate::core::ports::CompleteOpts<'static>,
        speaker: &str,
        // 本回合的身份块（驱动按当前提示词册现渲染；不进对话）。
        identity: &str,
        // 该 agent **会话自己的对话**：用户进它会话说的话，下一回合它带着（不分家的核心承诺）。
        dialogue: &[Msg],
        chat: &mut dyn Chat,
        mut tools: Option<&mut MemberTools>,
        turn: Vec<Msg>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<MemberTurn, String> {
        // 工具面**由角色表发放**（动词 + 只读核实工具）：表是唯一真相，代码里不另写一份名单。
        let face_rows: Vec<(&str, &crate::core::schema::ToolSchema)> =
            systools.tool_face(role).unwrap_or_default();
        let face_ids: Vec<String> = face_rows.iter().map(|(id, _)| id.to_string()).collect();
        let face: Vec<crate::core::ports::ToolDecl> =
            face_rows.iter().map(|(id, s)| s.decl(id)).collect();
        // 身份 + 本回合工具 + 对话 + 本回合提示：**唯一的装配点**。
        // 模块工具只在"这一身份能干活"时列出（角色表的 module_tools）——讨论席列出它等于请模型去撞墙。
        let mut msgs = assemble(
            identity,
            tools.as_deref(),
            &face_ids,
            systools.allows_module_tools(role),
            dialogue,
            &turn,
        );
        let native = matches!(
            tools.as_ref().map(|t| t.mode),
            Some(crate::core::providers::ToolMode::Native)
        );
        let mut lines: Vec<DiscLine> = Vec::new();
        loop {
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("已停止".to_string());
            }
            // 每轮都要一份（Copy）：只换 tools 槽位，其余照旧。
            let mut opts = opts;
            if native {
                opts.tools = Some(&face);
            }
            let stop = std::sync::Arc::clone(cancel);
            // **逐片外送**：讨论席的发言也要能看到"正在生成"（此前只把 Chunk 当中止信号、
            // 内容全丢掉，于是整个成员回合界面一动不动）。信封不能当正文流上屏，
            // 所以复用会话那条"到 { 就截住"的分片规则（session::stream_piece）。
            let mut acc = String::new();
            let done = {
                let sink_cell = std::cell::RefCell::new(&mut *sink);
                let mut keep = |c: crate::core::ports::Chunk| {
                    if stop.load(std::sync::atomic::Ordering::Relaxed) {
                        return false;
                    }
                    let ev = match c {
                        crate::core::ports::Chunk::Start => Some(SessionEvent::Delta {
                            speaker: speaker.to_string(),
                            kind: "start".to_string(),
                            text: String::new(),
                        }),
                        crate::core::ports::Chunk::Text(t) => {
                            let (send, next) = crate::core::session::stream_piece(&acc, &t);
                            acc = next;
                            if send.is_empty() {
                                None
                            } else {
                                Some(SessionEvent::Delta {
                                    speaker: speaker.to_string(),
                                    kind: "text".to_string(),
                                    text: send,
                                })
                            }
                        }
                        crate::core::ports::Chunk::Reasoning(r) => Some(SessionEvent::Delta {
                            speaker: speaker.to_string(),
                            kind: "reasoning".to_string(),
                            text: r,
                        }),
                    };
                    if let Some(ev) = ev {
                        (sink_cell.borrow_mut())(ev);
                    }
                    true
                };
                chat.complete(&msgs, opts, &mut keep)
            };
            if cancel.load(std::sync::atomic::Ordering::Relaxed) {
                return Err("已停止".to_string());
            }
            if let Some(err) = done.error.clone() {
                return Err(err);
            }
            let parsed = envelope::parse(&done.raw);
            // 这一轮的调用：native 从结构化槽位取，信封通道从正文里的信封取（一次可以多个）。
            let calls: Vec<(String, String, String)> = if native && !done.calls.is_empty() {
                done.calls
                    .iter()
                    .map(|c| (c.id.clone(), c.name.clone(), c.args_json.clone()))
                    .collect()
            } else {
                parsed
                    .tools
                    .iter()
                    .map(|t| (String::new(), t.name.clone(), t.args_json.clone()))
                    .collect()
            };
            // ① 表态优先：**任何一个调用是动词**，这一轮就是发言（收尾）。
            // 两条通道各认各的：native 从结构化槽位认动词；**信封通道从正文里的信封认**
            //（动词信封落在 parsed.verb 里，不在 parsed.tools 里——这里漏了，信封就永远认不出动词）。
            let from_text = |r: &envelope::Reply| match r.verb {
                Verb::Tool => None,
                // **降级 = 没写信封**（散文按原文收录）：那不算表态。
                // 否则散文会被当成 say 混进主会话，与契约"没表态就不投影、由核心在边界提醒"直接冲突——
                // 真机上就是这样一轮轮空转的（每个成员每轮都"说了"一句空话，谁也不同意）。
                v if !r.degraded => Some((v, r.text.clone(), false)),
                _ => None,
            };
            let said: Option<(Verb, String, bool)> = if native {
                calls
                    .iter()
                    .find_map(|(_, name, args)| verb_of(name).map(|v| (v, arg_text(args), false)))
                    .or_else(|| from_text(&parsed))
            } else {
                from_text(&parsed)
            };
            if let Some((verb, text, degraded)) = said {
                return Ok(MemberTurn {
                    verb: Some(verb),
                    text,
                    degraded,
                    truncated: done.truncated(),
                    lines,
                });
            }
            if calls.is_empty() {
                // 既没表态也没申请调用：按发言原文收录（降级是**结构化信号**，由上层如实落转录）。
                // 既没表态也没申请调用（散文 / 空回执）：这一轮到此为止，交回"没表态"。
                return Ok(MemberTurn {
                    verb: None,
                    text: parsed.text,
                    // 工具信封却解析不出调用 = 写坏了：如实标降级，与"散文"区分开。
                    degraded: parsed.degraded || parsed.verb == Verb::Tool,
                    truncated: done.truncated(),
                    lines,
                });
            }
            // ② 核实：**逐个**执行（只读工具），并把结果按协议形状回灌。
            // 没有工具环境（替身/降级）：跑不了核实，如实按原文收录。
            let Some(texts) = tools.as_ref().map(|t| t.sandbox.texts.clone()) else {
                return Ok(MemberTurn {
                    verb: None,
                    text: done.raw.clone(),
                    degraded: true,
                    truncated: done.truncated(),
                    lines,
                });
            };
            let mut round_views: Vec<ToolCallView> = Vec::new();
            for (call_id, name, args) in &calls {
                // **按角色表校验**：这个席位没有的工具一律如实拒绝（代码里不写"谁能用哪个"）。
                // 没有调用次数上限：模型继续核实就继续跑，直到它给出表态（或用户点停止）。
                if !systools.allows(role, name) {
                    sink(SessionEvent::Notice(format!(
                        "[越权] 本席位没有工具 {}：本轮不执行、不当表态（如实拒绝）",
                        name
                    )));
                    return Ok(MemberTurn {
                        verb: None,
                        text: done.raw.clone(),
                        degraded: true,
                        truncated: done.truncated(),
                        lines,
                    });
                }
                let (ok, output) = match tools.as_deref_mut() {
                    Some(t) => {
                        let out = crate::core::systool::execute(
                            &t.sandbox,
                            t.io.as_ref(),
                            &mut t.observations,
                            name,
                            args,
                        );
                        (out.ok, out.output)
                    }
                    None => (false, "这个席位没有工具环境".to_string()),
                };
                let head = output.lines().next().unwrap_or("").to_string();
                let view = ToolCallView {
                    speaker: speaker.to_string(),
                    module: String::new(),
                    name: name.clone(),
                    ok,
                    args: args.clone(),
                    output,
                    raw: done.raw.clone(),
                    call_id: call_id.clone(),
                    reply: 0,
                };
                // 核实行**实时**外送一条（短暂，不落盘）：用户能看到它在读、在查，而不是整回合黑箱。
                // 回合定稿时这段核实行会作为权威转录行重来一次（短暂块在收到权威行时被替换掉，不重复）。
                sink(SessionEvent::ToolCall(view.clone()));
                lines.push(DiscLine {
                    // 回合 id 由驱动补（它才知道整场工作的计数）；这里先占位。
                    turn: 0,
                    system: false,
                    text: format!(
                        "[{}:{}] {} {}",
                        speaker,
                        name,
                        if ok { "成功" } else { "失败" },
                        head
                    ),
                    degraded: false,
                    tool: Some(view.clone()),
                });
                round_views.push(view);
            }
            // **回灌必须走共享拼装**：native 要带 tool_calls + role=tool + call_id。
            // 手写 assistant(raw)+user("[工具…]") 会把协议破坏掉——真机上模型于是**看不见结果、反复调同一个工具**，
            // 撞上上限后以空发言收尾，讨论因此永不收敛（e2e 的假供应商不在乎形状，只有真机能照出来）。
            msgs.extend(reply_msgs(
                tools.as_ref().map(|t| t.mode).unwrap_or_default(),
                &done.raw,
                &round_views,
                &texts,
            ));
        }
    }
}

impl Discussion {
    pub fn new(
        members: Vec<Member>,
        allow_autonomy: bool,
        prompts: Prompts,
        llm: crate::core::ports::LlmOpts,
        cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
        protocol: String,
    ) -> Discussion {
        Discussion {
            members,
            transcript: Vec::new(),
            round: 0,
            pending_user_answers: Vec::new(),
            closed: false,
            allow_autonomy,
            prompts,
            llm,
            cancel,
            protocol,
            reminded: std::collections::BTreeMap::new(),
            remind_round: 0,
            opener: String::new(),
            cursor: Cursor::Fresh,
            handed: 0,
        }
    }

    /// 开始讨论：渲染开场提示词（一次），游标进入开场阶段。
    /// 与 advance/feed 一起构成可暂停的状态机——**驱动权在核心**（见 session-model.md 二之二）。
    pub fn start(&mut self, task: &str) {
        self.opener = self.prompts.render(
            &self.prompts.core.discuss.opener,
            &[
                ("protocol", self.protocol.clone()),
                ("task", task.to_string()),
            ],
        );
        self.handed = self.transcript.len();
        self.cursor = Cursor::Opener(0);
    }

    /// 接上「停止」标志（CollabSession 注入；回档重建后也要重新接）。
    pub fn set_cancel(&mut self, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) {
        self.cancel = cancel;
    }

    /// 第 i 个成员的 agent 名（核心据此拼出它的会话名）。
    pub fn member_id(&self, i: usize) -> Option<&str> {
        self.members.get(i).map(|m| m.id.as_str())
    }

    /// 「停止」标志（与核心共享同一个）。
    pub fn cancel_flag(&self) -> std::sync::Arc<std::sync::atomic::AtomicBool> {
        std::sync::Arc::clone(&self.cancel)
    }

    /// 开场还没开始过（核心驱动的第一步据此调 start）。
    pub fn not_started(&self) -> bool {
        matches!(self.cursor, Cursor::Fresh) && self.round == 0
    }

    /// 是否已被要求停止。
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    /// 本轮的调用选项：流式与预算都取全局设置（讨论也走同一份，不再是写死的非流式）。
    fn opts(&self) -> crate::core::ports::CompleteOpts<'static> {
        crate::core::ports::CompleteOpts::plain(self.llm.stream).with_timeout(self.llm.timeout_secs)
    }

    /// 首轮：聊天约定 + 用户需求（文案经提示词册渲染）。
    #[cfg(test)]
    pub fn open(
        &mut self,
        task: &str,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<(), String> {
        // 状态机驱动：开场也是"问 → 收 → 再问"，判定全在 advance/feed 里
        //（开场的表态与轮次同一口径：同意 / 离开立刻生效）。
        // 驱动权将来归核心：把这一圈换成核心的 take/put 序列即可。
        self.start(task);
        loop {
            match self.advance() {
                Adv::Ask { i, identity, turn } => {
                    let turn = self.member_turn(i, &identity, turn, sink)?;
                    // 开场也要有"没表态"的处置，否则这个循环会卡在同一个人身上空转。
                    match self.after_turn(i, turn.verb.is_some(), false, MAX_DISCUSS_REMIND) {
                        // 开场不因请教而中止（feed 已按阶段处理）。
                        AfterTurn::Done => {
                            let _ = self.feed(i, turn, 0, on_lines, sink);
                        }
                        // 内存驱动没有会话可注入提醒：直接重问同一个人（计数照走）。
                        AfterTurn::Remind => continue,
                        AfterTurn::Unanswered => {
                            self.note_system(&format!("[{}] 本轮未回应", self.members[i].id));
                            self.skip(i);
                        }
                    }
                }
                // 开场问完（或已是终态）：交回上层，轮次由 step 继续。
                Adv::Opened | Adv::Out(_) => return Ok(()),
            }
        }
    }

    /// 推进一轮：把当前转录并入上下文，依次转达给每个在组且未同意的成员。
    /// 讨论阶段不接工具循环：工具属执行机制，讨论只出主意（最小边界）。
    #[cfg(test)]
    pub fn step(
        &mut self,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> TurnOut {
        // 这个循环**必须有前进条件**：没表态时由 after_turn 计数（提醒 → 放过），
        // 否则假模型瞬间返回会把这里变成紧凑死循环（真烧过 CPU）。
        // 状态机驱动：本函数只负责"问 → 收 → 再问"，判定全在 advance/feed 里。
        // 驱动权将来归核心（见 docs/architecture/session-model.md 二之二）：那时把这一圈换成
        // 核心的 take/put 序列即可，判定一行不用改。
        loop {
            match self.advance() {
                Adv::Ask { i, identity, turn } => {
                    let turn = match self.member_turn(i, &identity, turn, sink) {
                        Ok(t) => t,
                        Err(_) if self.cancelled() => return TurnOut::Stopped,
                        Err(err) => return TurnOut::Interrupted(err),
                    };
                    let has_verb = turn.verb.is_some();
                    // 与生产路径**同一条策略**：没表态就重问（上限），到顶记"未回应"放过它。
                    match self.after_turn(i, has_verb, false, MAX_DISCUSS_REMIND) {
                        AfterTurn::Done => {
                            if let Some(out) = self.feed(i, turn, 0, on_lines, sink) {
                                return out;
                            }
                        }
                        // 内存驱动没有会话可注入提醒：直接重问同一个人（计数照走）。
                        AfterTurn::Remind => continue,
                        AfterTurn::Unanswered => {
                            self.note_system(&format!("[{}] 本轮未回应", self.members[i].id));
                            self.skip(i);
                        }
                    }
                }
                // 开场刚问完（只有 open 会碰到）：接着进轮次。
                Adv::Opened => continue,
                Adv::Out(out) => return out,
            }
        }
    }

    /// 推进讨论的**一步**：只决定"该问谁"（或给出终态）——**不自己调模型**。
    /// 驱动权归核心（它同时看得到协作会话与各 agent 的会话），见 session-model.md 二之二。
    pub fn advance(&mut self) -> Adv {
        if self.closed {
            return Adv::Out(TurnOut::Done);
        }
        // 已被要求停止：连轮次标记都不留（这一轮根本没开始）。
        if self.cancelled() {
            return Adv::Out(TurnOut::Stopped);
        }
        // 开场：逐个问一遍（开场不跳过任何人），问完进轮次。
        if let Cursor::Opener(i) = self.cursor {
            if i >= self.members.len() {
                self.round = 1;
                self.cursor = Cursor::Fresh;
                return Adv::Opened;
            }
            return Adv::Ask {
                i,
                identity: self.member_identity(i),
                turn: vec![Msg::user(self.opener.clone())],
            };
        }
        if matches!(self.cursor, Cursor::Fresh) {
            // 本轮到此刻还没交出去的行数（轮次标记也算）：一个成员说完就把它那一批交出去。
            self.handed = self.transcript.len();
            // 轮次边界：本轮的发言都在这条之后（回放时据此重算「本轮谁已同意」）。
            self.transcript.push(DiscLine {
                text: format!("[轮次 {}]", self.round + 1),
                degraded: false,
                tool: None,
                turn: 0,
                system: false,
            });
            // 用户回答优先转达。
            if let Some(ans) = self.pending_user_answers.first().cloned() {
                self.pending_user_answers.remove(0);
                self.transcript.push(DiscLine {
                    text: format!("[用户] {}", ans),
                    degraded: false,
                    tool: None,
                    turn: 0,
                    system: false,
                });
            }
            self.cursor = Cursor::At(0);
        }
        // 同意是**粘住**的：发过 agree 的人不再被追问；离开的人不算在场。
        let mut i = match self.cursor {
            Cursor::At(i) => i,
            Cursor::Fresh => 0,
            Cursor::Opener(i) => i,
        };
        while i < self.members.len() && (!self.members[i].present || self.members[i].agreed) {
            i += 1;
        }
        if i >= self.members.len() {
            // 本轮问完：收敛 / 上限判定，交回上层（下一次从新的一轮开始）。
            self.round += 1;
            self.cursor = Cursor::Fresh;
            if self.members.iter().filter(|m| m.present).all(|m| m.agreed) {
                self.closed = true;
                // 讨论收束 = 进入下一阶段：提醒计数一并归零（它只在本阶段有意义）。
                self.reminded.clear();
                return Adv::Out(TurnOut::Done);
            }
            if self.round > MAX_ROUNDS {
                self.closed = true;
                self.reminded.clear();
                return Adv::Out(TurnOut::Done);
            }
            return Adv::Out(TurnOut::Round);
        }
        self.cursor = Cursor::At(i);
        let identity = self.member_identity(i);
        let step_prompt = self.prompts.render(
            &self.prompts.core.discuss.step,
            &[(
                "transcript",
                self.transcript
                    .iter()
                    .map(|l| l.text.clone())
                    .collect::<Vec<_>>()
                    .join("\n"),
            )],
        );
        Adv::Ask {
            i,
            identity,
            turn: vec![Msg::user(step_prompt)],
        }
    }

    /// 这个席位的身份块：**每回合现渲染**（成员的参数 + 当前提示词册 + 登记处给的形态）。
    fn member_identity(&self, i: usize) -> String {
        let m = &self.members[i];
        m.params.identity(&self.prompts, m.mode)
    }

    /// 收下一个成员回合的结果（状态机的一步）：吸收、外送它那一批行、按动词改状态。
    /// 返回 Some = 该立刻交回上层（请教用户）；None = 继续问下一个。
    pub fn feed(
        &mut self,
        i: usize,
        turn: MemberTurn,
        turn_id: u64,
        on_lines: &mut LineSink<'_>,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Option<TurnOut> {
        // 开场与轮次共用这一条收尾路径；区别只在"请教要不要中止"（开场不中止）。
        let opener = matches!(self.cursor, Cursor::Opener(_));
        let id = self.members[i].id.clone();
        // **没表态**：不投影主会话（原文只在它自己的会话里）、不改它的状态。
        // 但**照样往后挪一格**——这是安全默认：任何驱动都不会因为"没表态"而卡在同一人身上
        //（要重问的驱动**根本不调 feed**，见 after_turn 的 Remind 分支）。
        let Some(verb) = turn.verb else {
            self.cursor = if opener {
                Cursor::Opener(i + 1)
            } else {
                Cursor::At(i + 1)
            };
            return None;
        };
        let (text, degraded) = (turn.text.clone(), turn.degraded);
        self.absorb(&id, verb, text.clone(), degraded, turn.truncated, turn_id);
        // 逐成员外送：**这个人说完就出它那一行**，不等整轮问完。
        on_lines(&self.transcript[self.handed..], sink);
        self.handed = self.transcript.len();
        // 往后挪一格（advance 下次从下一个人接着问）。
        self.cursor = if opener {
            Cursor::Opener(i + 1)
        } else {
            Cursor::At(i + 1)
        };
        let m = &mut self.members[i];
        match verb {
            Verb::Leave => m.present = false,
            Verb::Agree => m.agreed = true,
            Verb::Ask => {
                // 开场不因请教而中止：开场是各人表态，还没有可讨论的方案。
                if opener {
                    return None;
                }
                if self.allow_autonomy {
                    let note = self.prompts.core.discuss.autonomy_note.clone();
                    self.transcript.push(DiscLine {
                        text: note,
                        degraded: false,
                        tool: None,
                        turn: 0,
                        // 这是**系统**给的自主说明，不是用户说的。
                        system: true,
                    });
                    return None;
                }
                return Some(TurnOut::AskUser {
                    member: id,
                    question: text,
                });
            }
            Verb::Say | Verb::Tool => {}
        }
        None
    }

    fn absorb(
        &mut self,
        id: &str,
        verb: Verb,
        text: String,
        degraded: bool,
        truncated: bool,
        turn: u64,
    ) {
        let tag = match verb {
            Verb::Say => "say",
            Verb::Ask => "ask",
            Verb::Leave => "leave",
            Verb::Agree => "agree",
            Verb::Tool => "tool",
        };
        let mut line = format!("[{}:{}] {}", id, tag, text);
        if degraded {
            line.push_str(
                &self
                    .prompts
                    .render(&self.prompts.core.tool_texts.discuss_degraded, &[]),
            );
        }
        // 被长度截断：如实写在行尾（与"降级"同一套做法）——模型与用户都看得到
        if truncated {
            line.push_str(
                &self
                    .prompts
                    .render(&self.prompts.core.tool_texts.truncated_suffix, &[]),
            );
        }
        self.transcript.push(DiscLine {
            text: line,
            degraded,
            tool: None,
            turn,
            system: false,
        });
    }

    /// 成员一轮之后的处置：**核心只提醒、不强制**（见 docs/architecture/session-model.md 二）。
    /// 用户主动中止时计数不再工作（不注入提醒）；每轮开始时提醒次数归零。
    pub fn after_turn(
        &mut self,
        i: usize,
        has_verb: bool,
        user_stopped: bool,
        remind_cap: u32,
    ) -> AfterTurn {
        if self.round != self.remind_round {
            self.reminded.clear();
            self.remind_round = self.round;
        }
        let id = self.members[i].id.clone();
        if has_verb {
            self.reminded.remove(&id);
            return AfterTurn::Done;
        }
        if user_stopped {
            return AfterTurn::Done;
        }
        let n = self.reminded.entry(id.clone()).or_insert(0);
        if *n < remind_cap {
            *n += 1;
            return AfterTurn::Remind;
        }
        self.reminded.remove(&id);
        AfterTurn::Unanswered
    }

    /// 放过一个没表态的成员：不吸收、只把游标往后挪一格（提醒到顶时用）。
    pub fn skip(&mut self, i: usize) {
        self.cursor = if matches!(self.cursor, Cursor::Opener(_)) {
            Cursor::Opener(i + 1)
        } else {
            Cursor::At(i + 1)
        };
    }

    /// 记一行**系统消息**（提醒/边界这类不是谁说的内容）到主会话转录。
    /// 见 docs/architecture/session-model.md 二"系统消息"。
    pub fn note_system(&mut self, text: &str) {
        self.transcript.push(DiscLine {
            text: text.to_string(),
            degraded: false,
            tool: None,
            turn: 0,
            system: true,
        });
    }

    /// 全员同意后：核心整理——总结讨论，为每个留下的成员写执行任务提示词。
    /// 整理：核心 AI 总结讨论并出**任务链**（见 docs/architecture/task-chain.md）。
    /// 产出走 **plan 工具调用**（核心操作不手写 JSON）；没调用或载荷不合法就**如实报错**。
    pub fn synthesize(
        &self,
        core_chat: &mut dyn Chat,
        mode: crate::core::providers::ToolMode,
    ) -> Result<(String, crate::core::chain::TaskChain), String> {
        let roster = self
            .members
            .iter()
            .filter(|m| m.present)
            .map(|m| m.id.clone())
            .collect::<Vec<_>>()
            .join("、");
        let user = self.prompts.render(
            &self.prompts.core.synthesize.user,
            &[
                ("roster", roster),
                (
                    "transcript",
                    self.transcript
                        .iter()
                        .map(|l| l.text.clone())
                        .collect::<Vec<_>>()
                        .join("\n"),
                ),
            ],
        );
        let msgs = vec![
            Msg::system(self.prompts.core.synthesize.system.clone()),
            Msg::user(user),
        ];
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        let cancel = std::sync::Arc::clone(&self.cancel);
        let mut keep =
            move |_c: crate::core::ports::Chunk| !cancel.load(std::sync::atomic::Ordering::Relaxed);
        // 核心操作走工具调用：载荷形状与从前一致（plan + nodes），只是入口变成 plan 工具。
        let payload = core_operation(
            &self.prompts.systools,
            "planner",
            "plan",
            mode,
            core_chat,
            &msgs,
            self.opts(),
            &mut keep,
        )?;
        if self.cancelled() {
            return Err("已停止".to_string());
        }
        let parsed: SynthReply = serde_json::from_value(payload)
            .map_err(|e| format!("plan 工具的载荷不合法（{}）", e))?;
        let chain = crate::core::chain::TaskChain {
            nodes: parsed
                .nodes
                .into_iter()
                .map(|n| crate::core::chain::TaskNode {
                    id: n.id,
                    title: n.title,
                    objective: n.objective,
                    assignee: n.assignee,
                    deps: n.deps,
                    status: crate::core::chain::NodeStatus::Pending,
                    sub_session: None,
                    report: None,
                    acceptance: None,
                })
                .collect(),
        };
        Ok((parsed.plan, chain))
    }
}

/// 原文前 n 个字符（如实报错时带上一点现场；按字符切，不切坏多字节）。
fn head_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// 核心整理的结构化回执（形状见 prompts/roles/planner.yaml 的 synthesize）。
#[derive(Debug, Clone, Deserialize)]
struct SynthReply {
    plan: String,
    #[serde(default)]
    nodes: Vec<SynthNode>,
}

/// 任务链里的一个节点（核心给的是"意图"，状态与子会话由核心自己管）。
#[derive(Debug, Clone, Deserialize)]
struct SynthNode {
    id: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    objective: String,
    #[serde(default)]
    assignee: String,
    #[serde(default)]
    deps: Vec<String>,
}

/// 验收清单条目：核心输出的结构化核对结果。
#[derive(Debug, Clone, Deserialize)]
pub struct CheckItem {
    pub item: String,
    pub status: String,
    #[serde(default)]
    pub evidence: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// 执行与验收：成员按任务干活并回报；核心对照回报产出结构化清单。
pub struct Execution {
    pub reports: BTreeMap<String, String>,
    /// 验收原始输出（解析失败时如实呈现）。
    pub checklist_raw: String,
    /// 结构化清单；空 = 解析失败（all_pass 保守判否）。
    pub items: Vec<CheckItem>,
    /// 执行/验收途中调用失败（超时 / 网络）：非空 = 本轮**中断**，不交付。
    /// 上层据此如实告知用户；会话保持可继续（用户点「继续」重新推进）。
    pub error: Option<String>,
    /// 被用户「停止」：非空 = 本轮**停止**，未收完的回报**不入册**（半截回报进转录会误导验收）。
    pub stopped: bool,
    /// 「停止」标志（泵注入）：每次模型调用前与调用中途都看它。
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl Execution {
    pub fn new() -> Execution {
        Execution {
            reports: BTreeMap::new(),
            checklist_raw: String::new(),
            items: Vec::new(),
            error: None,
            stopped: false,
            cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    /// 是否已被要求停止（总验收共用）。
    fn cancelled(&self) -> bool {
        self.cancel.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn review(
        &mut self,
        core_chat: &mut dyn Chat,
        plan: &str,
        prompts: &Prompts,
        llm: crate::core::ports::LlmOpts,
        mode: crate::core::providers::ToolMode,
    ) {
        let reports = self
            .reports
            .iter()
            .map(|(id, r)| format!("[{}] {}\n", id, r))
            .collect::<Vec<_>>()
            .join("");
        let user = prompts.render(
            &prompts.core.review.user,
            &[("plan", plan.to_string()), ("reports", reports)],
        );
        let msgs = vec![
            Msg::system(prompts.core.review.system.clone()),
            Msg::user(user),
        ];
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        let opts =
            crate::core::ports::CompleteOpts::plain(llm.stream).with_timeout(llm.timeout_secs);
        let cancel = std::sync::Arc::clone(&self.cancel);
        let mut keep = move |_c: Chunk| !cancel.load(std::sync::atomic::Ordering::Relaxed);
        // 核心操作走工具调用：总验收清单由 checklist 工具承载。
        let made = core_operation(
            &prompts.systools,
            "orchestrator",
            "checklist",
            mode,
            core_chat,
            &msgs,
            opts,
            &mut keep,
        );
        if self.cancelled() {
            self.stopped = true;
            return;
        }
        match made {
            Ok(payload) => {
                // 载荷里的 items 数组；形状不对就如实记空清单（all_pass 保守判否）。
                self.items = payload
                    .get("items")
                    .cloned()
                    .and_then(|v| serde_json::from_value::<Vec<CheckItem>>(v).ok())
                    .unwrap_or_default();
                self.checklist_raw = payload.to_string();
            }
            Err(err) => {
                self.error = Some(err);
            }
        }
    }

    pub fn all_pass(&self) -> bool {
        // 清单为空（解析失败）= 保守判否；有清单则逐项全过才通过。
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|i| i.status.eq_ignore_ascii_case("pass"))
    }
}

/// 一次工具调用的产出：调用视图 + 它压进历史的消息。
pub struct ToolRun {
    pub view: ToolCallView,
    /// 该工具行压进历史的消息（[工具结果] …）。
    pub msgs: Vec<Msg>,
}

/// 核心 AI 的一次**操作**：声明该角色的工具面、跑一次模型、从**工具调用参数**里取载荷。
///
/// 为什么必须走工具调用（见 docs/architecture/tools-and-roles.md）：核心操作会驱动核心走下一步
/// （建任务链、判交付、确认名单、推进状态机），属于"操作"而不是"说话"——
/// 正文里手写 JSON 既没有 schema 校验、也不进工具台账，写坏就整轮失败。
///
/// 两条通道都认：native 从结构化槽位取；手写信封从正文里的信封取。
// 与 turn_with / converse_with 同一组参数（工具面 / 通道 / 消息 / 出口）：不是随手堆参数，
// 收口成参数对象只会把参数挪个地方、并让"谁拿到什么"更难读。有意取舍（见 docs/testing/quality-isolation.md）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn core_operation(
    systools: &crate::core::roles::SystemTools,
    role: &str,
    tool: &str,
    mode: crate::core::providers::ToolMode,
    chat: &mut dyn Chat,
    msgs: &[Msg],
    opts: crate::core::ports::CompleteOpts<'static>,
    keep: &mut dyn FnMut(crate::core::ports::Chunk) -> bool,
) -> Result<serde_json::Value, String> {
    // 声明面按**通道形态**给：原生通道才声明（信封通道的模型看提示词里的工具说明）。
    let mut opts = opts;
    let decls: Vec<crate::core::ports::ToolDecl> =
        if mode == crate::core::providers::ToolMode::Native {
            systools
                .tool_face(role)
                .map(|f| f.into_iter().map(|(id, s)| s.decl(id)).collect())
                .unwrap_or_default()
        } else {
            Vec::new()
        };
    if !decls.is_empty() {
        opts.tools = Some(&decls);
    }
    let done = chat.complete(msgs, opts, keep);
    if let Some(err) = done.error {
        return Err(err);
    }
    // native：结构化槽位里找这个名字的调用。
    if let Some(c) = done.calls.iter().find(|c| c.name == tool) {
        return serde_json::from_str(&c.args_json)
            .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, c.args_json));
    }
    // 手写信封：正文里的信封里找。
    let parsed = crate::core::envelope::parse(&done.raw);
    if let Some(t) = parsed.tools.iter().find(|t| t.name == tool) {
        return serde_json::from_str(&t.args_json)
            .map_err(|e| format!("{} 的参数不是合法 JSON（{}）：{}", tool, e, t.args_json));
    }
    Err(format!(
        "没有调用 {}（核心操作必须走工具调用）：{}",
        tool,
        head_chars(&done.raw, 200)
    ))
}

/// 拼一次模型调用的消息：**身份 + 本回合工具 + 对话 + 本回合提示**。
///
/// 为什么只有这一处：身份与工具块都是**派生**的（登记处 + 提示词册 + 这一回合的身份），
/// 它们不占对话的位置——对话里只有真正发生过的事（谁说了什么、调了什么工具）。
/// 实时与重建都从这里拼，所以"回放与实时产出同样的消息"只约束对话本身。
pub(crate) fn assemble(
    identity: &str,
    tools: Option<&MemberTools>,
    ids: &[String],
    with_modules: bool,
    dialogue: &[Msg],
    turn: &[Msg],
) -> Vec<Msg> {
    let mut out: Vec<Msg> = Vec::with_capacity(dialogue.len() + turn.len() + 2);
    out.push(Msg::system(identity));
    if let Some(ctx) = tools {
        let block = ctx.tools_block(ids, with_modules);
        if !block.is_empty() {
            out.push(Msg::system(block));
        }
    }
    out.extend(dialogue.iter().cloned());
    out.extend(turn.iter().cloned());
    out
}

/// 把**一次模型回复**翻译成发给模型的消息——**实时与重建都只走这一处**。
///
/// 为什么必须只有一处：转录行与消息列表是同一件事的两份表示，两边各拼一次就会漂移。
/// 真实缺陷就出在这里：一次回复里有多条原生调用时，实时推的助手消息与重建出来的既不是同一条，
/// 第二条起实时还根本不推助手消息（上下文里因此凭空多/少消息）。
///
/// 形状按**当前形态**决定，所以形态切换时旧消息会被自动表达成新形状（切回去也能还原——事实留在转录里）：
/// - 这条回复的调用都带合法原生 id 且当前走原生通道 → assistant(正文 + tool_calls) + 每条调用一条 role=tool；
/// - 其余（手写信封、原生通道里写坏的调用、切换形态后的旧消息）→ assistant(正文) + 结果当用户消息。
pub(crate) fn reply_msgs(
    mode: crate::core::providers::ToolMode,
    raw: &str,
    calls: &[ToolCallView],
    texts: &crate::core::prompt::ToolTexts,
) -> Vec<Msg> {
    let protocol = mode == crate::core::providers::ToolMode::Native
        && !calls.is_empty()
        && calls.iter().all(|c| !c.call_id.is_empty());
    let mut out: Vec<Msg> = Vec::with_capacity(calls.len() + 1);
    if protocol {
        out.push(Msg::assistant_calls(
            raw,
            calls
                .iter()
                .map(|c| crate::core::ports::ToolCall {
                    id: c.call_id.clone(),
                    name: c.name.clone(),
                    args_json: c.args.clone(),
                })
                .collect(),
        ));
    } else {
        out.push(Msg::assistant(raw));
    }
    for c in calls {
        let body = texts.render(
            &texts.tool_result_wrapper,
            &[("label", c.label()), ("output", c.output.clone())],
        );
        out.push(if protocol {
            Msg::tool(&c.call_id, body)
        } else {
            Msg::user(body)
        });
    }
    out
}

/// 逐轮产出回调：拿到刚定稿的一轮 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，调用方随后用不了同一个 sink。
pub type RoundSink<'a> = dyn FnMut(&Round, &mut dyn FnMut(SessionEvent)) + 'a;

/// 讨论行的**逐成员外送回调**：拿到刚定稿的行 + 本次的出口。
/// 出口当参数传而不是让回调捕获它——否则回调借着 sink，`step`/`open` 的调用方随后用不了它。
pub type LineSink<'a> = dyn FnMut(&[DiscLine], &mut dyn FnMut(SessionEvent)) + 'a;

/// 原生通道的工具名 → 讨论动词：**只认协作动词**，其余一律不认识（不认识 = 越权，如实拒绝）。
pub(crate) fn verb_of(name: &str) -> Option<Verb> {
    match name {
        "say" => Some(Verb::Say),
        "agree" => Some(Verb::Agree),
        "leave" => Some(Verb::Leave),
        "ask" => Some(Verb::Ask),
        _ => None,
    }
}

/// 原生调用的参数里取正文（供应商给的是一段 JSON 文本；取不到就是空串——不猜）。
pub(crate) fn arg_text(args_json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(args_json)
        .ok()
        .and_then(|v| {
            v.get("text")
                .and_then(|t| t.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 一轮模型调用的产出（一轮 = 一条文本转录行；有工具时紧跟一条工具行）。
/// 原始输出不进这里：工具轮由 ToolCallView.raw 承载、文本轮进上下文的就是解析后的文本。
pub struct Round {
    /// 这一轮属于哪次模型回复（一次回复可能产出多条工具行）。
    pub reply: u64,
    /// 解析后的可见文本（信封缺失时即原文）；工具轮为空串（它说的就是那封信封）。
    pub text: String,
    /// 该轮思维链（没给就是空串）。
    pub reasoning: String,
    /// 该轮压进历史的消息：工具轮 = [assistant(raw)]，末轮 = [assistant(text)]。
    pub text_msgs: Vec<Msg>,
    pub tool: Option<ToolRun>,
    /// 供应商给的结束原因（原样；没给 = 空串）：核心据此分辨"写完停"还是"被长度截断"。
    pub finish: String,
    /// 这次调用失败了（超时 / 网络）：非空 = **没有拿到模型回复**，这一轮不该落转录。
    /// 上层据此如实告知用户并中断本轮（用户可以点「继续」重试）。
    pub error: Option<String>,
}

impl Round {
    /// 这一轮的输出是不是被供应商按长度截断了。
    pub fn truncated(&self) -> bool {
        crate::core::ports::truncated(&self.finish)
    }
}

/// stream/on 透传给通道（呈现层在 on 里外送 Delta）；on 返回 false = 用户要求中止。
/// on_tool 在每个工具跑完后立刻回调（工具行与文本行因此天然有序）。
/// 终止保证：超限后告知一次并强制收尾；其后再来 tool 信封按原文作答，不再执行。
// 逐轮外送要的四个出口（分片 / 工具 / 逐轮 / 提示词）都是回调，收口成参数对象只是把参数挪个地方、
// 并让"谁在什么时候拿到什么"更难读。这是有意的设计取舍（同 docs/testing/quality-isolation.md 的 allow 清单）。
#[allow(clippy::too_many_arguments)]
pub(crate) fn converse_with(
    chat: &mut dyn Chat,
    mut tools: Option<&mut MemberTools>,
    identity: &str,
    dialogue: Vec<Msg>,
    llm: crate::core::ports::LlmOpts,
    speaker: &str,
    on: &mut dyn FnMut(Chunk) -> bool,
    on_tool: &mut dyn FnMut(&ToolCallView),
    on_round: &mut RoundSink<'_>,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<Round> {
    // 身份 + 本回合工具 + 对话：**唯一的装配点**。工具面取自这一席位（执行席的表现 + 它自己模块的工具）。
    let (ids, with_modules) = match tools.as_ref() {
        Some(ctx) => (ctx.allowed.clone(), ctx.with_modules),
        None => (Vec::new(), false),
    };
    let mut msgs = assemble(
        identity,
        tools.as_deref(),
        &ids,
        with_modules,
        &dialogue,
        &[],
    );
    // 观察账本随会话保存（回档时清空），这里不动它——它的语义是"这一段转录里的读取证据"。
    let mut rounds: Vec<Round> = Vec::new();
    // 逐轮产出：**一轮跑完就把它交出去**（调用方据此立刻外送与落盘，不必等整个回合结束）。
    // 用宏而不是逐个改写 push 点：5 个分支都要"先回调、再入册"，写死五遍迟早漏一处。
    macro_rules! push_round {
        ($r:expr) => {{
            let r = $r;
            on_round(&r, sink);
            rounds.push(r);
        }};
    }
    // 没有工具环境时的回复号来源（见下面 reply_id）。
    let mut local_reply = 0u64;
    // 本代是否被用户中途停止（通道回调返回 false）：循环顶部据此退出（见下）。
    let mut aborted = false;
    loop {
        // 用户在生成中途点了「停止」（通道回调返回 false）：**这一轮到此为止**，不再发起下一次调用。
        // 没有这条出口，被停的生成会以"每次调用立刻返回"的速度空转（真机上烧过一次 CPU）；
        // 此前是靠工具调用上限兜底的——上限删掉后这条出口必须自己站住。
        if aborted {
            return rounds;
        }
        // 这一回复的稳定 id：一次模型回复一个号，本次问询里的多条工具行共用它。
        // 没有工具环境时给本代内的局部号即可（那条路径不落转录行、也不分组）。
        let reply_id = match tools.as_deref_mut() {
            Some(ctx) => ctx.next_reply(),
            None => {
                local_reply += 1;
                local_reply
            }
        };
        // 形态与工具声明面：由本成员的通道形态决定（envelope = 不声明，走手写信封；native = 声明本成员的工具）
        let (mode, decls) = match tools.as_deref_mut() {
            Some(ctx) if ctx.mode == crate::core::providers::ToolMode::Native => {
                let mode = ctx.mode;
                (mode, tool_decls(ctx))
            }
            Some(ctx) => (ctx.mode, ToolDecls::default()),
            None => (
                crate::core::providers::ToolMode::Envelope,
                ToolDecls::default(),
            ),
        };
        // 逐轮累积思维链（原文以通道返回值为准：非流式通道不回 Chunk）。
        let mut reasoning = String::new();
        let done = {
            let mut sink = |chunk: Chunk| {
                match &chunk {
                    Chunk::Start => reasoning.clear(),
                    Chunk::Text(_) => {}
                    Chunk::Reasoning(r) => reasoning.push_str(r),
                }
                let keep = on(chunk);
                if !keep {
                    aborted = true;
                }
                keep
            };
            let opts = CompleteOpts {
                stream: llm.stream,
                tools: if decls.list.is_empty() {
                    None
                } else {
                    Some(&decls.list)
                },
                timeout_secs: llm.timeout_secs,
            };
            chat.complete(&msgs, opts, &mut sink)
        };
        // 调用失败（超时 / 网络）：**不是模型的回复**——这一轮不解析信封、不执行工具、不落转录，
        // 只把原因带回，让上层如实告知用户并中断本轮（用户可以点「继续」重试）。
        // 为什么必须短路：错误文本若被当成发言吸收，核心按转录派生的"下一步该谁说话"就歪了。
        if let Some(err) = done.error.clone() {
            push_round!(Round {
                reply: reply_id,
                text: String::new(),
                reasoning: String::new(),
                text_msgs: Vec::new(),
                tool: None,
                finish: String::new(),
                error: Some(err),
            });
            return rounds;
        }
        // 结束原因如实带回：被长度截断要落日志——事后才判定得出"是截断还是模型自己写错"。
        let finish = done.finish.clone();
        let truncated = done.truncated();
        let calls = done.calls.clone();
        let raw = done.raw;
        if truncated {
            if let Some(t) = tools.as_deref_mut() {
                t.log.warn(
                    "engine::converse",
                    &format!(
                        "模型输出被长度截断（finish_reason={}）：第 {} 轮，正文 {} 字",
                        finish,
                        rounds.len() + 1,
                        raw.chars().count()
                    ),
                );
            }
        }
        let mut reply = envelope::parse(&raw);
        // 手写信封不合法时**先**问修复端口：只做无歧义的修补（默认实现只转义字符串里的裸控制字符）。
        // 修好并重新解析成合法工具信封 = 本轮照常执行工具；修不了就走原来的"失败工具行"路径。
        // 封顶后不修（与"封顶后不再执行工具"同一口径）。
        // 用户中止的生成不修：半截信封是"被停下来"的产物，不是模型的意图——绝不据此执行工具。
        // 自由格式工具（patch）也不修：它的正文在信封之外，转义控制字符会把补丁里的换行弄坏。
        let freeform_tool = reply
            .tools
            .iter()
            .any(|t| crate::core::systool::is_freeform(&t.name));
        let mut repaired: Option<String> = None;
        if !aborted && !freeform_tool {
            if let Some(kind) = reply.tools.first().and_then(|t| t.malformed.clone()) {
                if let Some(ctx) = tools.as_deref_mut() {
                    let out = ctx.repair.repair(&raw, &kind);
                    if let Some(text) = out.repaired.as_deref() {
                        let again = envelope::parse(text);
                        if !again.tools.is_empty()
                            && again.tools.iter().all(|t| t.malformed.is_none())
                        {
                            reply = again;
                            repaired = Some(out.what.join("；"));
                        }
                    }
                }
            }
        }
        // ── 原生通道：工具调用来自供应商的结构化槽位（不解析信封）──
        if mode == crate::core::providers::ToolMode::Native {
            if let Some(ctx) = tools.as_deref_mut() {
                // ① 有原生调用：逐个执行，各成一条工具行；助手消息如实记下"它调了什么"（回放与下一轮都看得到）
                if !calls.is_empty() {
                    // 先定好每个调用落在哪个工具、参数是什么（patch 的正文在 body 参数里，
                    // 原生协议要求参数是 JSON 对象；转义交给供应商的解码器）。
                    let plan: Vec<(Option<String>, String, String)> = calls
                        .iter()
                        .map(|c| {
                            let (module, tool) = decls
                                .wire
                                .get(&c.name)
                                .cloned()
                                .unwrap_or((None, c.name.clone()));
                            let args = if crate::core::systool::is_freeform(&tool) {
                                serde_json::from_str::<serde_json::Value>(&c.args_json)
                                    .ok()
                                    .and_then(|v| {
                                        v.get("body")
                                            .and_then(|b| b.as_str())
                                            .map(|s| s.to_string())
                                    })
                                    .unwrap_or_default()
                            } else {
                                c.args_json.clone()
                            };
                            (module, tool, args)
                        })
                        .collect();
                    // 调度：**连续**声明可并发的调用合成一批并发跑，其余各自独占（写入类因此是批次之间的屏障）。
                    // 结果按原始下标返回，随后一律按原序回填——并发只影响执行，不影响上下文里的顺序。
                    let done = run_batch(ctx, &plan);
                    // 先按原序把工具行建好（执行已经做完），再让**唯一那处**构造函数产出这一回复的消息：
                    // 实时与重建走同一个函数，"重建上下文与实时一致"因此是结构保证的。
                    let texts = &ctx.sandbox.texts;
                    let reply_text = reply.text.clone();
                    let views: Vec<ToolCallView> = calls
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            let (label, outcome) = done[i].clone();
                            ToolCallView {
                                speaker: speaker.to_string(),
                                module: label,
                                name: plan[i].1.clone(),
                                ok: outcome.ok,
                                args: plan[i].2.clone(),
                                output: outcome.output,
                                // 助手消息正文 = 这一回复的原文（正文与调用进的是同一条消息）
                                raw: reply_text.clone(),
                                call_id: c.id.clone(),
                                reply: reply_id,
                            }
                        })
                        .collect();
                    let msgs_of = reply_msgs(ctx.mode, &reply_text, &views, texts);
                    for m in &msgs_of {
                        msgs.push(m.clone());
                    }
                    for (i, view) in views.into_iter().enumerate() {
                        on_tool(&view);
                        push_round!(Round {
                            reply: reply_id,
                            // 正文只挂在本回复的第一条工具行上（只显示一条，不重复）
                            text: if i == 0 {
                                reply_text.clone()
                            } else {
                                String::new()
                            },
                            reasoning: std::mem::take(&mut reasoning),
                            text_msgs: if i == 0 {
                                vec![msgs_of[0].clone()]
                            } else {
                                Vec::new()
                            },
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[i + 1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                        });
                    }
                    continue;
                }
                // ② 没有原生调用却写了信封：**不执行**（两套形态互斥），但也不静默丢掉意图
                {
                    if let Some(inv) = reply.tools.first().cloned() {
                        let texts = &ctx.sandbox.texts;
                        let view = ToolCallView {
                            speaker: speaker.to_string(),
                            module: inv.module.clone().unwrap_or_default(),
                            name: inv.name.clone(),
                            ok: false,
                            args: inv.args_json.clone(),
                            output: texts.native_no_envelope.clone(),
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        };
                        on_tool(&view);
                        // 这条没有合法原生 id（模型是手写的信封）：走文本形状，不能发 role=tool。
                        let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                        for m in &msgs_of {
                            msgs.push(m.clone());
                        }
                        push_round!(Round {
                            reply: reply_id,
                            text: reply.text.clone(),
                            reasoning,
                            text_msgs: vec![msgs_of[0].clone()],
                            tool: Some(ToolRun {
                                view,
                                msgs: vec![msgs_of[1].clone()],
                            }),
                            finish: finish.clone(),
                            error: None,
                        });
                        continue;
                    }
                }
            }
        }
        // 信封这一线的分派依据：不合法时恰好一条（见 envelope::build_invokes），合法时为空。
        let malformed = reply.tools.first().and_then(|t| t.malformed.clone());
        match malformed.clone() {
            // 信封不合法（缺 name / 混用两种形态 / calls 为空 / 没写完…）：**一个工具都不执行**，
            // 但记一条失败的工具行把"哪里不合法"回注给模型（下一轮自己改）。同样计入上限，不会死循环。
            _ if malformed.is_some() && tools.is_some() => {
                let inv = reply.tools.first().cloned().expect("上臂已判非空");
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                // 回执按判定出的类别给修法（未闭合 / 裸控制字符 / 语法错 / 字段不合法）。
                let mut why = ctx
                    .sandbox
                    .texts
                    .malformed_report(inv.malformed.as_ref().expect("上臂已判存在"));
                // 供应商说是长度截断：那"写坏 JSON"就不是模型的错，改法也不同（分次写/拆小步骤）。
                if truncated {
                    why.push('\n');
                    why.push_str(&ctx.sandbox.texts.malformed_truncated);
                }
                let view = ToolCallView {
                    speaker: speaker.to_string(),
                    module: inv.module.clone().unwrap_or_default(),
                    name: inv.name.clone(),
                    ok: false,
                    args: inv.args_json.clone(),
                    output: why,
                    raw: raw.clone(),
                    call_id: String::new(),
                    reply: reply_id,
                };
                on_tool(&view);
                let texts = &ctx.sandbox.texts;
                let msgs_of = reply_msgs(mode, &raw, std::slice::from_ref(&view), texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                push_round!(Round {
                    reply: reply_id,
                    text: reply.text.clone(),
                    reasoning,
                    text_msgs: vec![msgs_of[0].clone()],
                    tool: Some(ToolRun {
                        view,
                        msgs: vec![msgs_of[1].clone()],
                    }),
                    finish: finish.clone(),
                    error: None,
                });
            }
            // 合法信封：**一次回复里的多个调用一起执行**（同一套声明并发调度），各成一条工具行。
            _ if tools.is_some() && !reply.tools.is_empty() => {
                let ctx = tools.as_deref_mut().expect("上臂已判存在");
                let invokes = reply.tools.clone();
                // 自由格式工具（patch）只能单发：它的输入是**信封之后的那段正文**（不必转义），
                // 显示正文只认信封**之前**那段——补丁内容不该被当成 AI 发言渲染出来。
                if invokes.len() == 1 && crate::core::systool::is_freeform(&invokes[0].name) {
                    reply.text = invokes[0].lead.clone();
                }
                let plan: Vec<(Option<String>, String, String)> = invokes
                    .iter()
                    .map(|t| {
                        let freeform = crate::core::systool::is_freeform(&t.name);
                        let args = if freeform {
                            t.body.clone()
                        } else {
                            t.args_json.clone()
                        };
                        (t.module.clone(), t.name.clone(), args)
                    })
                    .collect();
                // 内置工具（read/write/edit/search）优先且不属于任何模块；外部工具按模块定 cwd。
                let done = run_batch(ctx, &plan);
                // 修过信封就如实标注在回执最前面（模型与用户都能看到核心没有瞎猜）
                let annotate = |outcome: ToolOutcome| -> ToolOutcome {
                    match repaired.as_deref() {
                        Some(what) if !what.is_empty() => ToolOutcome {
                            ok: outcome.ok,
                            output: format!(
                                "{}\n{}",
                                ctx.sandbox.texts.render(
                                    &ctx.sandbox.texts.envelope_repaired,
                                    &[("what", what.to_string())]
                                ),
                                outcome.output
                            ),
                        },
                        _ => outcome,
                    }
                };
                let texts = &ctx.sandbox.texts;
                let views: Vec<ToolCallView> = plan
                    .iter()
                    .enumerate()
                    .map(|(i, (_module, tool, _args))| {
                        let (label, outcome) = done[i].clone();
                        let outcome = annotate(outcome);
                        ToolCallView {
                            speaker: speaker.to_string(),
                            module: label,
                            name: tool.clone(),
                            ok: outcome.ok,
                            args: invokes[i].args_json.clone(),
                            output: outcome.output,
                            raw: raw.clone(),
                            call_id: String::new(),
                            reply: reply_id,
                        }
                    })
                    .collect();
                let msgs_of = reply_msgs(mode, &raw, &views, texts);
                for m in &msgs_of {
                    msgs.push(m.clone());
                }
                // 按原序回填：每条调用一条工具行（多调用时正文只挂第一条）。
                // text = 信封之外的那段正文（可能为空；信封 JSON 已被 parse 剥掉，永不进 text）。
                for (i, view) in views.into_iter().enumerate() {
                    on_tool(&view);
                    push_round!(Round {
                        reply: reply_id,
                        text: if i == 0 {
                            reply.text.clone()
                        } else {
                            String::new()
                        },
                        reasoning: std::mem::take(&mut reasoning),
                        text_msgs: if i == 0 {
                            vec![msgs_of[0].clone()]
                        } else {
                            Vec::new()
                        },
                        tool: Some(ToolRun {
                            view,
                            msgs: vec![msgs_of[i + 1].clone()],
                        }),
                        finish: finish.clone(),
                        error: None,
                    });
                }
            }
            // 无工具环境 / 模型不再发起调用：按原文口径如实收录（信封已被剥掉，显示文本里不会有 JSON），循环终止。
            _ => {
                // 只有会出文本行（有正文或思维链）时才往历史里放这条 assistant，
                // 否则实时历史会比重建历史多一条空消息。
                let text = reply.text;
                let has_line = !text.trim().is_empty() || !reasoning.trim().is_empty();
                let text_msgs = if has_line {
                    vec![Msg::assistant(text.trim().to_string())]
                } else {
                    Vec::new()
                };
                push_round!(Round {
                    reply: reply_id,
                    text,
                    reasoning,
                    text_msgs,
                    tool: None,
                    finish,
                    error: None,
                });
                return rounds;
            }
        }
    }
}
