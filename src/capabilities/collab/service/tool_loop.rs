//! 成员回合里的**工具循环**：声明面 → 放行判定 → 并发调度 → 执行（内置 / 模块外部）→ 回填。
//!
//! 它只认 `MemberTools`（这一回合的工具面与观察账本），策略在这里，机制在 `tools` 的端口后面。

use super::discussion::*;
use crate::capabilities::llm::api::ToolInvoke;
use crate::capabilities::session::api::{MemberTools, SessionEvent};
use crate::capabilities::tools::api::ToolOutcome;
/// 执行一次工具调用：内置优先；外部工具按模块走（模块为空时由 dispatch_external 如实报错）。
/// 账本经**分支副本**回到本成员（见 run_branch）——串行与并发只有这一条执行路径。
pub(crate) fn run_one(
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
/// 所以"副本 + 按原序提交"与串行执行的结果完全相同（见 crate::capabilities::tools::api::Observations::absorb）。
pub(crate) fn run_branch(
    ctx: &MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
) -> (
    String,
    ToolOutcome,
    crate::capabilities::tools::api::Observations,
) {
    let mut branch = ctx.observations.clone();
    let (label, outcome) = if !face_has(ctx, module, name) {
        // **按这一回合的工具面校验**：不在面里的调用一律如实拒绝（不静默执行、也不当表态）。
        (
            String::new(),
            crate::capabilities::tools::api::refuse(&ctx.sandbox.texts, name),
        )
    } else if crate::capabilities::tools::api::is_builtin(name) {
        (
            String::new(),
            ctx.tools.run_builtin(
                &ctx.sandbox,
                &ctx.builtin_tools,
                &mut branch,
                name,
                args_json,
            ),
        )
    } else if let Some(h) = handler_for(ctx, name) {
        // 核心自有工具：与内置、模块走**同一条派发路径**，不是循环里的特例。
        let tctx = crate::kernel::ports::ToolCtx {
            work: &ctx.sandbox.work_name,
            agent: &ctx.sandbox.agent,
            line: ctx.line.load(std::sync::atomic::Ordering::Relaxed),
        };
        (String::new(), h.run(&tctx, name, args_json))
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

/// 这次调用在**本回合的工具面**里吗：内置按角色表发放的 id 清单，外部工具按"这一回合给不给模块工具"
/// 与模块归属（判据只有这一处——执行侧与"如实说一句越权"都读它）。
pub(crate) fn face_has(ctx: &MemberTools, module: Option<&str>, name: &str) -> bool {
    if crate::capabilities::tools::api::is_builtin(name) {
        ctx.allowed.iter().any(|t| t == name)
    } else if handler_for(ctx, name).is_some() {
        // 核心自有工具与内置**同口径**：名字在这一回合的工具面里，就是放行。
        ctx.allowed.iter().any(|t| t == name)
    } else {
        // 没写 module 不算越权：那是"派发时消歧"的事，由 dispatch_external 如实说清（多模块下不猜）。
        ctx.with_modules && module.map(|m| ctx.modules.contains_key(m)).unwrap_or(true)
    }
}

/// 这一回合有没有哪个执行者认领这个名字（核心自有工具）。
/// 认领了就按**工具面**放行，不再走模块派发——判据因此只剩“面里有没有它”。
fn handler_for<'a>(
    ctx: &'a MemberTools,
    name: &str,
) -> Option<&'a std::sync::Arc<dyn crate::kernel::ports::ToolHandler>> {
    ctx.handlers.iter().find(|h| h.owns(name))
}

/// 本回合的工具面之外的调用：**如实说一句**（用户可见），执行侧负责不执行（见 run_branch）。
pub(crate) fn note_unauthorized(
    ctx: &MemberTools,
    plan: &[(Option<String>, String, String)],
    sink: &mut dyn FnMut(SessionEvent),
) {
    for (module, tool, _args) in plan {
        if !face_has(ctx, module.as_deref(), tool) {
            sink(SessionEvent::Notice(format!(
                "[越权] 本席位没有工具 {}：本轮不执行、不当表态（如实拒绝）",
                tool
            )));
        }
    }
}

/// 这个工具有没有**声明可并发**（策略在册子/清单里，代码里不写名单）：
/// 内置工具看 `prompts/shared/tools.yaml` 的 `builtin_tools.<名字>.parallel`，模块工具看 `module.yaml` 的 `tools.<名字>.parallel`。
/// 未声明 = 独占串行；**没写 module 的外部工具也按独占**（那要等 dispatch 才知道是哪个模块，核心不猜）。
pub(crate) fn is_parallel(ctx: &MemberTools, module: Option<&str>, name: &str) -> bool {
    match module {
        Some(id) => ctx
            .modules
            .get(id)
            .map(|m| m.parallel.contains(name))
            .unwrap_or(false),
        None => {
            crate::capabilities::tools::api::is_builtin(name)
                && ctx
                    .builtin_tools
                    .get(name)
                    .map(|s| s.parallel)
                    .unwrap_or(false)
        }
    }
}

/// 一次待用户确认的工具调用：工具名 + 所属模块（内置/核心自有为空）+ 参数原文。
pub struct ToolConfirm {
    pub module: Option<String>,
    pub tool: String,
    pub args: String,
}

/// 确认回调：`(待确认调用, 事件出口) -> 放行 / 拒绝 / 本轮不再问`（实现方阻塞等用户回答）。
pub type ConfirmFn<'a> =
    &'a mut dyn FnMut(&ToolConfirm, &mut dyn FnMut(SessionEvent)) -> crate::kernel::api::Approval;

/// 一次生成里的确认通道：`full` 一旦置位，本轮（这次生成）剩余调用都不再问；
/// `confirm` 由调用方实现（阻塞等用户回答，返回放行 / 拒绝 / 本轮不再问）。
pub struct ApprovalGate<'a> {
    pub full: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub confirm: ConfirmFn<'a>,
}

/// 这次调用的候选名字（用于比对 `ask` 表）：内置与核心自有工具用工具名；
/// 模块工具同时接受"工具名"与"模块.工具"（省略 module 且只有唯一模块时也按模块名兜底）。
fn ask_candidates(ctx: &MemberTools, module: Option<&str>, tool: &str) -> Vec<String> {
    if crate::capabilities::tools::api::is_builtin(tool) || handler_for(ctx, tool).is_some() {
        return vec![tool.to_string()];
    }
    match module {
        Some(m) => vec![tool.to_string(), format!("{}.{}", m, tool)],
        None if ctx.modules.len() == 1 => {
            let m = ctx.modules.keys().next().cloned().unwrap_or_default();
            vec![tool.to_string(), format!("{}.{}", m, tool)]
        }
        None => vec![tool.to_string()],
    }
}

/// 这次调用要不要先问用户：没有确认通道、或本轮已被"全部放行"，一律不问；
/// 否则按这一席的生效权限（`full` 粒度不问；`ask` 命中才问）。
fn needs_ask(
    ctx: &MemberTools,
    gate: Option<&ApprovalGate<'_>>,
    module: Option<&str>,
    tool: &str,
) -> bool {
    let Some(gate) = gate else {
        return false;
    };
    if gate.full.load(std::sync::atomic::Ordering::Relaxed) {
        return false;
    }
    ask_candidates(ctx, module, tool)
        .iter()
        .any(|n| ctx.sandbox.permissions.should_ask(n))
}

/// 执行一批调用：**连续**声明可并发的合成一批并发跑，其余各自独占；结果按**原始下标**返回。
/// 原生通道与手写信封通道共用这一处调度——并发策略只有一份，两个通道不会各写一套。
/// 账本走分支副本 + 按原序合并（与串行执行等价，见 crate::capabilities::tools::api::Observations::absorb）。
/// **要问用户的调用强制串行**：先经确认通道拿到回答；拒绝就回一条"用户拒绝"的结果、不执行；
/// 答"本轮不再问"（`Approval::Full`）则放行这一次并把 `full` 置位，本轮剩余调用都不再问。
pub(crate) fn run_batch(
    ctx: &mut MemberTools,
    plan: &[(Option<String>, String, String)],
    mut approval: Option<&mut ApprovalGate<'_>>,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<(String, ToolOutcome)> {
    let mut done: Vec<Option<(String, ToolOutcome)>> = (0..plan.len()).map(|_| None).collect();
    let mut i = 0;
    while i < plan.len() {
        if needs_ask(ctx, approval.as_deref(), plan[i].0.as_deref(), &plan[i].1) {
            let (module, tool, args) = &plan[i];
            let req = ToolConfirm {
                module: module.clone(),
                tool: tool.clone(),
                args: args.clone(),
            };
            let gate = approval.as_deref_mut().expect("needs_ask 已确认有通道");
            let decision = (gate.confirm)(&req, sink);
            gate.full.store(
                decision == crate::kernel::api::Approval::Full,
                std::sync::atomic::Ordering::Relaxed,
            );
            let allowed = decision != crate::kernel::api::Approval::Deny;
            done[i] = Some(if allowed {
                run_one(ctx, module.as_deref(), tool, args)
            } else {
                let label = module.clone().unwrap_or_default();
                (
                    label,
                    ToolOutcome {
                        ok: false,
                        output: ctx.sandbox.texts.tool_denied_by_user.clone(),
                    },
                )
            });
            i += 1;
            continue;
        }
        if is_parallel(ctx, plan[i].0.as_deref(), &plan[i].1) {
            let mut j = i;
            while j < plan.len() && is_parallel(ctx, plan[j].0.as_deref(), &plan[j].1) {
                j += 1;
            }
            let batch: Vec<(
                String,
                ToolOutcome,
                crate::capabilities::tools::api::Observations,
            )> = std::thread::scope(|s| {
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
pub(crate) fn tool_decls(ctx: &MemberTools) -> ToolDecls {
    let mut decls = ToolDecls::default();
    let mut taken: Vec<String> = Vec::new();
    // **只声明这个席位拿到的工具**（判据与执行时校验同一份 allowed）：声明了却调不动没有意义，
    // 模型会照着声明去调，被拒一次就白烧一轮（见 docs/tools/tools-and-roles.md 二）。
    for (name, schema) in &ctx.builtin_tools {
        if !ctx.allowed.iter().any(|t| t == name) {
            continue;
        }
        // patch 是自由格式：它的声明单独写（参数是 body 字符串，不是 JSON 信封的 args）
        if crate::capabilities::tools::api::is_freeform(name) {
            continue;
        }
        decls.list.push(schema.decl(name));
        taken.push(name.clone());
        decls.wire.insert(name.clone(), (None, name.clone()));
    }
    if ctx
        .allowed
        .iter()
        .any(|t| t == crate::capabilities::tools::api::PATCH)
    {
        let patch = crate::capabilities::tools::api::patch_decl();
        taken.push(patch.name.clone());
        decls.wire.insert(
            patch.name.clone(),
            (None, crate::capabilities::tools::api::PATCH.to_string()),
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
                None => crate::capabilities::llm::api::ToolDecl {
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
pub(crate) fn available_tools(ctx: &MemberTools) -> String {
    let mut list: Vec<String> = Vec::new();
    for (id, mt) in &ctx.modules {
        for name in mt.commands.keys() {
            list.push(format!("{}.{}", id, name));
        }
    }
    list.extend(crate::capabilities::tools::api::names());
    list.join(&ctx.sandbox.texts.tool_list_separator)
}

pub(crate) fn deny(ctx: &MemberTools, why: String) -> ToolOutcome {
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
pub(crate) fn dispatch_external(ctx: &MemberTools, inv: &ToolInvoke) -> (String, ToolOutcome) {
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
                            let why = crate::capabilities::tools::api::arg_fault_text(
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
                ctx.tools
                    .run_module(&ctx.fence.at(&mt.root), command, &inv.args_json),
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
