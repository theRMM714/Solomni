//! 目的：成员回合里的工具循环——声明面 → 放行判定 → 并发调度 → 执行（内置 / 模块外部）→ 回填。
//! 管：只认 MemberTools（这一回合的工具面与观察账本）；策略（放行表、并发调度、确认闸）在这里。
//! 不管：模型调用（在 llm / session 端口后）；工具机制本身（在 tools 的端口后面）。
//! 联动：执行经 tools 的 ToolExec 端口；回填顺序与转录行形状见 docs/session/session-model.md。

use super::discussion::*;
use crate::capabilities::llm::api::ToolInvoke;
use crate::capabilities::permission::api::{CallScope, Decision, Effect, Reach, ReachSet};
use crate::capabilities::session::api::{MemberTools, SessionEvent};
use crate::capabilities::tools::api::{Observations, ToolSchema};
use crate::capabilities::workspace::api::ReadFact;
use crate::kernel::api::ToolOutcome;
/// 目的：执行一次工具调用：内置优先；外部工具按模块走（模块为空时由 dispatch_external 如实报错）。
///   账本经**分支副本**回到本成员（见 run_branch）——串行与并发只有这一条执行路径。
pub(crate) fn run_one(
    ctx: &mut MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    grant: &CallGrant,
    cancel: &std::sync::atomic::AtomicBool,
) -> (String, ToolOutcome, Option<ReadFact>) {
    let (label, outcome, branch, read) =
        run_branch(ctx, module, name, args_json, ask, grant, cancel);
    // 串行：分支的副本就是"这次调用之后"的账本，直接接管（与逐个记账等价）。
    ctx.observations = branch;
    (label, outcome, read)
}

/// 目的：一次调用的执行体（**不碰本成员的账本**）：分支各持账本副本，由调用方按原始顺序合并/接管。
///   为什么这样：并发批次里多个调用同时跑，而账本是可变状态；只读类调用的结果不依赖账本，
///   所以"副本 + 按原序提交"与串行执行的结果完全相同（见 crate::capabilities::tools::api::Observations::absorb）。
pub(crate) fn run_branch(
    ctx: &MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    grant: &CallGrant,
    cancel: &std::sync::atomic::AtomicBool,
) -> (
    String,
    ToolOutcome,
    crate::capabilities::tools::api::Observations,
    Option<ReadFact>,
) {
    let mut branch = ctx.observations.clone();
    // 这一趟若是一次完整的 read：它读到了哪几行、当时的指纹（挂到工具行上，发送视图据此判过期）。
    let mut read: Option<ReadFact> = None;
    let (label, outcome) = if !face_has(ctx, module, name) {
        // **按这一回合的工具面校验**：不在面里的调用一律如实拒绝（不静默执行、也不当表态）。
        (String::new(), crate::capabilities::tools::api::refuse(name))
    } else if crate::capabilities::tools::api::is_builtin(name) {
        let mut outcome = ctx.tools.run_builtin(
            &ctx.sandbox,
            &ctx.builtin_tools,
            &mut branch,
            name,
            args_json,
        );
        commit_writes(ctx, &mut branch, &mut outcome);
        // read 只产生一条事实（一次调用只读一个路径）；没读到整份原文时是 None。
        read = branch.take_reads().into_iter().next();
        (String::new(), outcome)
    } else if crate::capabilities::tools::api::is_exec(name) {
        (
            String::new(),
            run_shell(ctx, name, args_json, ask, grant, cancel),
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
        dispatch_external(ctx, &inv, ask, grant)
    };
    (label, outcome, branch, read)
}

/// 目的：写工具落地后**自动提交**——路径取自这次调用真正写过的文件（观察账本的分支副本，见 Observations::take_written）。
/// 约束：准入在 workspace（逐路径 CAS）；基线过期 = 这次写入只留在沙箱、共享区不动，原因如实写回工具回执，
///   由模型显式 work_pull 后再写——不降级（不替它合并，也不静默留下"写了但没提交"）。
pub(crate) fn commit_writes(
    ctx: &MemberTools,
    branch: &mut Observations,
    outcome: &mut ToolOutcome,
) {
    let written = branch.take_written();
    let removed = branch.take_removed();
    if written.is_empty() && removed.is_empty() {
        return;
    }
    let Some(ws) = ctx.workspace.as_ref() else {
        return;
    };
    let rel = |abs: &std::path::PathBuf| -> Option<String> {
        abs.strip_prefix(&ctx.sandbox.private)
            .ok()
            .map(crate::kernel::api::slash)
    };
    let mut paths: Vec<String> = written.iter().filter_map(rel).collect();
    let mut deletes: Vec<String> = removed.iter().filter_map(rel).collect();
    paths.sort();
    paths.dedup();
    deletes.sort();
    deletes.dedup();
    if paths.is_empty() && deletes.is_empty() {
        return;
    }
    let message = if paths.is_empty() {
        "工具删除"
    } else {
        "工具写入"
    };
    let req = crate::capabilities::workspace::api::CommitRequest {
        paths,
        deletes,
        message: message.to_string(),
        time: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        // 锚 = 这次工具调用所属的那一行：回档才能把这次写入和它的转录证据一起退掉。
        line: ctx.line.load(std::sync::atomic::Ordering::Relaxed),
    };
    if let Err(e) = ws.work_commit_auto(
        &ctx.sandbox.work_name,
        &ctx.sandbox.agent,
        &ctx.sandbox.session,
        &req,
    ) {
        outcome.output.push_str(&format!(
            "\n\n[自动提交未完成] 这次写入只落在你的沙箱，共享区没有更新：{}",
            e
        ));
    }
}

/// 目的：这次调用在**本回合的工具面**里吗：内置按角色表发放的 id 清单，外部工具按"这一回合给不给模块工具"
///   与模块归属（判据只有这一处——执行侧与"如实说一句越权"都读它）。
pub(crate) fn face_has(ctx: &MemberTools, module: Option<&str>, name: &str) -> bool {
    // 系统工具册里的名字（含 `compact` 这类只由系统发起的工具）也按工具面校验：
    // 不能因为“没写 module”就落进模块工具的兜底分支。
    if crate::capabilities::tools::api::is_builtin(name)
        || crate::capabilities::tools::api::is_exec(name)
        || ctx.builtin_tools.contains_key(name)
    {
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
/// 认领了就按**工具面**放行——判据只剩“面里有没有它”。
fn handler_for<'a>(
    ctx: &'a MemberTools,
    name: &str,
) -> Option<&'a std::sync::Arc<dyn crate::kernel::ports::ToolHandler>> {
    ctx.handlers.iter().find(|h| h.owns(name))
}

/// 目的：本回合的工具面之外的调用：**如实说一句**（用户可见），执行侧负责不执行（见 run_branch）。
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

/// 目的：这个工具有没有**声明可并发**（策略在册子/清单里，代码里不写名单）：
///   内置工具看 `prompts/shared/tools.yaml` 的 `builtin_tools.<名字>.parallel`，模块工具看 `module.yaml` 的 `tools.<名字>.parallel`。
///   未声明 = 独占串行；**没写 module 的外部工具也按独占**（那要等 dispatch 才知道是哪个模块，核心不猜）。
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

/// 目的：一次待用户确认的工具调用：工具名 + 所属模块（内置/核心自有为空）+ 参数原文。
pub struct ToolConfirm {
    pub module: Option<String>,
    pub tool: String,
    pub args: String,
}

/// 目的：确认回调：`(待确认调用, 事件出口) -> 用户选的选项 id`（`allow` / `deny` / `full`）：实现方阻塞等回答。
pub type ConfirmFn<'a> = &'a mut dyn FnMut(&ToolConfirm, &mut dyn FnMut(SessionEvent)) -> String;

/// 目的：一次生成里的确认通道：allow_turn 一旦置位，本轮（这次生成）剩余 ask 都放行；
///   confirm 由调用方实现（阻塞等用户回答，给回选项 id）。
pub struct ConfirmGate<'a> {
    pub allow_turn: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub confirm: ConfirmFn<'a>,
}

/// 目的：一次调用在这次档位下的生效范围与处置。
#[derive(Debug, Clone)]
pub(crate) struct CallGrant {
    pub reach: ReachSet,
    pub decision: Decision,
}

/// 目的：模块工具的派生声明（模块暂不声明 effect / reach；按执行类给保守默认）。
fn module_schema() -> ToolSchema {
    ToolSchema {
        desc: String::new(),
        params: None,
        parallel: false,
        capability: "none".to_string(),
        effect: Some(Effect::Exec),
        reach: Some(ReachSet::from([
            Reach::Private,
            Reach::ModuleRead,
            Reach::ModuleWrite,
            Reach::HostRead,
            Reach::Network,
        ])),
        decision: Some(Decision::Allow),
        scope: false,
        callers: Vec::new(),
    }
}

/// 目的：按这次调用找到工具声明；系统工具读总表，模块工具用派生声明。
fn schema_for(ctx: &MemberTools, module: Option<&str>, tool: &str) -> Option<ToolSchema> {
    if crate::capabilities::tools::api::is_builtin(tool)
        || crate::capabilities::tools::api::is_exec(tool)
        || handler_for(ctx, tool).is_some()
    {
        return ctx.builtin_tools.get(tool).cloned();
    }
    let id = match module {
        Some(m) => m,
        None if ctx.modules.len() == 1 => ctx.modules.keys().next()?.as_str(),
        None => return None,
    };
    let mt = ctx.modules.get(id)?;
    if mt.commands.contains_key(tool) || mt.services.contains_key(tool) {
        Some(module_schema())
    } else {
        None
    }
}

/// 目的：求这次调用的生效 reach 与处置；scope 越出档位或工具最大 reach 就拒绝。
fn grant_for(
    ctx: &MemberTools,
    module: Option<&str>,
    tool: &str,
    args_json: &str,
) -> Result<CallGrant, String> {
    let schema = match schema_for(ctx, module, tool) {
        Some(s) => s,
        // 模块工具的名字/归属由 dispatch 如实说清（没指明模块、没有这个模块、模块没声明这个工具）。
        None if !ctx.modules.is_empty() => module_schema(),
        None => return Err(format!("未知工具：{}", tool)),
    };
    let effect = schema.effect();
    if effect == Effect::None {
        return Ok(CallGrant {
            reach: ReachSet::empty(),
            decision: Decision::Allow,
        });
    }
    // shell 的语义校验（命令长度 / scope / timeout）放在闸门前：非法命令不弹卡、不落进程。
    if crate::capabilities::tools::api::is_exec(tool) {
        crate::capabilities::tools::api::parse_shell(&schema, args_json)?;
    }
    let scope = if schema.scope {
        let args: serde_json::Value =
            serde_json::from_str(args_json).map_err(|e| format!("参数不是合法 JSON：{}", e))?;
        CallScope::from_request(args.get("scope").and_then(|v| v.as_str()))?
    } else {
        CallScope::None
    };
    let requested = if schema.scope {
        Some(scope.reach())
    } else {
        None
    };
    let reach = ctx
        .sandbox
        .permissions
        .grant(&schema.max_reach(), requested.as_ref())?;
    // 只有模块工具用「模块.工具」作为逐工具决定的键；内置 / 核心自有工具仍用工具名。
    let is_system = crate::capabilities::tools::api::is_builtin(tool)
        || crate::capabilities::tools::api::is_exec(tool)
        || handler_for(ctx, tool).is_some();
    let owner = if is_system {
        None
    } else {
        module.map(|m| m.to_string()).or_else(|| {
            if ctx.modules.len() == 1 {
                ctx.modules.keys().next().cloned()
            } else {
                None
            }
        })
    };
    let qualified = owner
        .map(|m| format!("{}.{}", m, tool))
        .unwrap_or_else(|| tool.to_string());
    let decision =
        ctx.sandbox
            .permissions
            .decision_for(&qualified, effect, scope, schema.decision());
    Ok(CallGrant { reach, decision })
}

/// 目的：执行一批调用：连续声明可并发的合成一批并发跑，其余各自独占；结果按原始下标返回。
///   每次调用先过权限闸门：reach 不够 / scope 越界 / deny 不执行；ask 进统一裁决队；
///   allow_turn 只对本轮后续 ask 生效。没有可回答前端时 ask 一律拒绝。
pub(crate) fn run_batch(
    ctx: &mut MemberTools,
    plan: &[(Option<String>, String, String)],
    mut confirms: Option<&mut ConfirmGate<'_>>,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    cancel: &std::sync::atomic::AtomicBool,
    sink: &mut dyn FnMut(SessionEvent),
) -> Vec<(String, ToolOutcome, Option<ReadFact>)> {
    let mut done: Vec<Option<(String, ToolOutcome, Option<ReadFact>)>> =
        (0..plan.len()).map(|_| None).collect();
    let grants: Vec<Result<CallGrant, String>> = plan
        .iter()
        .map(|(module, tool, args)| grant_for(ctx, module.as_deref(), tool, args))
        .collect();
    let mut i = 0;
    while i < plan.len() {
        let (module, tool, args) = &plan[i];
        let label = module.clone().unwrap_or_default();
        let grant = match &grants[i] {
            Ok(g) => g.clone(),
            Err(why) => {
                done[i] = Some((label, deny(ctx, why.clone()), None));
                i += 1;
                continue;
            }
        };
        if grant.decision.is_deny() {
            let why = format!(
                "当前档位「{}」不允许这次调用。",
                ctx.sandbox.permissions.profile.label()
            );
            done[i] = Some((label, deny(ctx, why), None));
            i += 1;
            continue;
        }
        if grant.decision.is_ask() {
            let already = confirms
                .as_ref()
                .map(|g| g.allow_turn.load(std::sync::atomic::Ordering::Relaxed))
                .unwrap_or(false);
            if already {
                done[i] = Some(run_one(
                    ctx,
                    module.as_deref(),
                    tool,
                    args,
                    ask,
                    &grant,
                    cancel,
                ));
                i += 1;
                continue;
            }
            let Some(gate) = confirms.as_deref_mut() else {
                let why = "这次调用需要用户确认，但当前没有可回答的前端；按拒绝处理，未执行。"
                    .to_string();
                done[i] = Some((label, deny(ctx, why), None));
                i += 1;
                continue;
            };
            let req = ToolConfirm {
                module: module.clone(),
                tool: tool.clone(),
                args: args.clone(),
            };
            let decision = (gate.confirm)(&req, sink);
            let allow_turn = decision == crate::capabilities::session::api::OPT_TOOL_ALLOW_TURN;
            if allow_turn {
                gate.allow_turn
                    .store(true, std::sync::atomic::Ordering::Relaxed);
            }
            let allowed =
                allow_turn || decision == crate::capabilities::session::api::OPT_TOOL_ALLOW;
            done[i] = Some(if allowed {
                run_one(ctx, module.as_deref(), tool, args, ask, &grant, cancel)
            } else {
                let why =
                    "用户没有放行这次工具调用：它没有执行。请换一种做法，或请用户调整会话权限。"
                        .to_string();
                (label, deny(ctx, why), None)
            });
            i += 1;
            continue;
        }
        if is_parallel(ctx, module.as_deref(), tool) {
            let mut j = i;
            while j < plan.len()
                && is_parallel(ctx, plan[j].0.as_deref(), &plan[j].1)
                && matches!(&grants[j], Ok(g) if g.decision == Decision::Allow)
            {
                j += 1;
            }
            let ctx_ref: &MemberTools = ctx;
            let batch: Vec<(
                String,
                ToolOutcome,
                crate::capabilities::tools::api::Observations,
                Option<ReadFact>,
            )> = std::thread::scope(|s| {
                let handles: Vec<_> = plan[i..j]
                    .iter()
                    .enumerate()
                    .map(|(k, (module, tool, args))| {
                        let g = match &grants[i + k] {
                            Ok(g) => g.clone(),
                            Err(_) => unreachable!("并行批只收 allow"),
                        };
                        s.spawn(move || {
                            run_branch(ctx_ref, module.as_deref(), tool, args, ask, &g, cancel)
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect()
            });
            for (k, (label, outcome, branch, read)) in batch.into_iter().enumerate() {
                ctx.observations.absorb(&branch);
                done[i + k] = Some((label, outcome, read));
            }
            i = j;
        } else {
            done[i] = Some(run_one(
                ctx,
                module.as_deref(),
                tool,
                args,
                ask,
                &grant,
                cancel,
            ));
            i += 1;
        }
    }
    adopt_sandbox(ctx, plan, &mut done);
    // 每一次调用都留一条**同形状的审计记录**（与呈现层的动作记录同一处格式化）：可查、可对账。
    for ((module, tool, _args), slot) in plan.iter().zip(done.iter()) {
        if let Some((_label, outcome, _read)) = slot {
            let name = match module {
                Some(m) => format!("{}.{}", m, tool),
                None => tool.clone(),
            };
            ctx.log.info(
                "action",
                &crate::capabilities::tools::api::action_audit(
                    &ctx.role,
                    &name,
                    outcome.ok,
                    &outcome.output,
                ),
            );
        }
    }
    done.into_iter()
        .map(|d| d.expect("每个调用都有执行结果"))
        .collect()
}

/// 目的：把 shell / 模块工具落进沙箱的改动，在**回合边界**采纳成共享区提交
///   （write / edit / patch 的改动已在各自调用后由 commit_writes 提交）。
/// 约束：只有这一批里真的跑成过不透明工具（shell / 模块工具）才扫全盘；工作区缺失、外部工作区、
///   采纳失败都不打断回合——回执挂在这一批最后一个跑成的不透明工具结果后面。
fn adopt_sandbox(
    ctx: &MemberTools,
    plan: &[(Option<String>, String, String)],
    done: &mut [Option<(String, ToolOutcome, Option<ReadFact>)>],
) {
    let Some(ws) = ctx.workspace.as_ref() else {
        return;
    };
    if ctx.sandbox.work_name.is_empty() {
        return;
    }
    // 最后一个跑成的不透明工具：回执挂它的结果后面（下一次生成能看到的通道）。
    let mut last: Option<usize> = None;
    for (i, (module, tool, _)) in plan.iter().enumerate() {
        if module.is_none() && !crate::capabilities::tools::api::is_exec(tool) {
            continue;
        }
        if matches!(&done[i], Some((_, o, _)) if o.ok) {
            last = Some(i);
        }
    }
    let Some(i) = last else {
        return;
    };
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let line = ctx.line.load(std::sync::atomic::Ordering::Relaxed);
    let note = match ws.work_adopt_sandbox(
        &ctx.sandbox.work_name,
        &ctx.sandbox.agent,
        &ctx.sandbox.session,
        time,
        line,
    ) {
        Ok(report) => adopt_note(&report),
        Err(e) => format!("\n\n[沙箱改动未提交] 共享区没有更新：{}", e),
    };
    if let Some((_, outcome, _)) = &mut done[i] {
        outcome.output.push_str(&note);
    }
}

/// 目的：沙箱采纳的回执（给模型看）；没有改动也没有没采纳的路径时为空串。
fn adopt_note(report: &crate::capabilities::workspace::api::AdoptReport) -> String {
    let mut out = String::new();
    if let Some(id) = report.commit {
        out.push_str(&format!(
            "\n\n[沙箱改动已提交 {}] 这些文件不是 write / edit 改的（shell 或模块工具），已写进共享区：{}",
            id,
            report.changed.join("、")
        ));
    }
    for h in &report.held {
        out.push_str(&format!("\n[未采纳] {}：{}", h.path, h.why));
    }
    out
}

/// 目的：本成员这次请求要声明的工具（原生通道用）：内置工具 + 模块工具。
///   原生协议里**没有 module 字段**，所以跨模块同名工具靠线上名消歧（{模块}_{工具}，撞名再加序号）；
///   返回的映射把线上名翻回（模块, 工具）。模块没声明参数的照旧声明（参数结构交回给工具自己解释）。
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
        // 已启动常驻服务的操作：与外部工具同一条声明路径（线上名里的点换成下划线，供应商函数名不收点）。
        for tool in mt.services.keys() {
            let flat = tool.replace('.', "_");
            let mut wire_name = format!("{}_{}", id, flat);
            let mut n = 2;
            while taken.contains(&wire_name) {
                wire_name = format!("{}_{}_{}", id, flat, n);
                n += 1;
            }
            taken.push(wire_name.clone());
            decls
                .wire
                .insert(wire_name.clone(), (Some(id.clone()), tool.clone()));
            let op = &mt.services[tool];
            decls.list.push(crate::capabilities::llm::api::ToolDecl {
                name: wire_name.clone(),
                description: if op.description.is_empty() {
                    format!("模块 {} 的常驻服务 {} 操作 {}", id, op.service, op.op)
                } else {
                    op.description.clone()
                },
                // 参数契约原样声明（服务给的 JSON Schema）：没有就退回"任意对象"，不编造。
                parameters: op.parameters.clone().unwrap_or_else(
                    || serde_json::json!({ "type": "object", "additionalProperties": true }),
                ),
            });
        }
    }
    decls
}

/// 目的：可用的外部工具清单：逐条列成「模块.工具」，末尾补上内置工具。
///   三种失败（模块不存在 / 模块没这个工具 / 缺 module 且模块不唯一）都用它把可选范围说回去。
pub(crate) fn available_tools(ctx: &MemberTools) -> String {
    let mut list: Vec<String> = Vec::new();
    for (id, mt) in &ctx.modules {
        for name in mt.commands.keys() {
            list.push(format!("{}.{}", id, name));
        }
        for name in mt.services.keys() {
            list.push(format!("{}.{}", id, name));
        }
    }
    list.extend(crate::capabilities::tools::api::names());
    list.join("、")
}

pub(crate) fn deny(ctx: &MemberTools, why: String) -> ToolOutcome {
    ToolOutcome {
        ok: false,
        output: format!("{}。可用：{}", why, available_tools(ctx)),
    }
}

/// 目的：执行一次 shell：按 scope 派生围栏，host 走 ProcessRunner，VM 走 guest；回执补 scope 提示。
/// 约束：requested 已在 grant_for 里校验；这里只按 granted 派生围栏，不自动升级 scope。
fn run_shell(
    ctx: &MemberTools,
    name: &str,
    args_json: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    grant: &CallGrant,
    cancel: &std::sync::atomic::AtomicBool,
) -> ToolOutcome {
    use crate::capabilities::tools::api;
    let Some(schema) = ctx.builtin_tools.get(name) else {
        return api::refuse(name);
    };
    let call = match api::parse_shell(schema, args_json) {
        Ok(c) => c,
        Err(e) => {
            return ToolOutcome {
                ok: false,
                output: e,
            }
        }
    };
    if ctx.sandbox.private.as_os_str().is_empty() {
        return ToolOutcome {
            ok: false,
            output: "shell 没有私有沙箱（cwd 为空），这次调用没有执行。".to_string(),
        };
    }
    let mut fence =
        crate::kernel::api::FenceSpec::for_call(&ctx.sandbox, &grant.reach, None, &ctx.fence)
            .at(&ctx.sandbox.private);
    fence.ro_tree.extend(ctx.runtime_roots.iter().cloned());
    fence.ro_tree.sort();
    fence.ro_tree.dedup();
    // 容器身份按"会话 + agent + 受保护根集合"派生（见 container_name）：scope 不同 → 根集合不同 → 不同 SID；
    // 同一 reach 的后续调用复用 SID 与 ACL。生命周期归会话，调用结束不释放。
    if ctx.tier == crate::kernel::api::Tier::Vm && ctx.guest_lease.is_empty() {
        return ToolOutcome {
            ok: false,
            output: "这条虚拟机档会话没有 guest 租约，shell 没有执行。".to_string(),
        };
    }
    let started = std::time::Instant::now();
    // cwd 用**本环境写法**：本机档是宿主路径，虚拟机档是 guest 内路径（与文件工具、提示词同一座路径桥）。
    let cwd_view = match ctx.sandbox.path_codec.as_ref() {
        Some(codec) => codec.to_view(&ctx.sandbox.private),
        None => ctx.sandbox.private.display().to_string(),
    };
    let slot = ctx.shell_live.clone();
    if let Some(slot) = &slot {
        let scope = match call.scope {
            CallScope::Sandbox => "sandbox",
            CallScope::External => "external",
            CallScope::None => "",
        };
        slot.start(&call.script, scope, &cwd_view);
    }
    let mut outcome = if ctx.tier == crate::kernel::api::Tier::Vm {
        let run = crate::capabilities::vm::api::GuestRun {
            command: call.script.clone(),
            args_json: String::new(),
            env: Vec::new(),
            cwd: ctx.sandbox.private.display().to_string(),
            timeout_secs: call.timeout.as_secs(),
            output_limit: crate::capabilities::vm::api::OutputLimit::Shell,
        };
        ctx.vm
            .guest_run_fenced_cancellable(&ctx.guest_lease, &run, &fence, Some(cancel))
    } else {
        let run = crate::kernel::ports::ShellRun {
            fence: &fence,
            script: &call.script,
            timeout: call.timeout,
            cancel: Some(cancel),
            path: ctx.runtime_path.as_deref(),
            ask,
            gate: None,
        };
        ctx.tools.run_shell(&run)
    };
    // 共享区不承载执行位：相对路径脚本被直接执行（EACCES）时，补一句"用解释器跑"的出路。
    if !outcome.ok {
        if let Some(hint) = api::shared_exec_hint(&call.script, &outcome.output) {
            outcome.output.push_str(&hint);
        }
    }
    // 回执补「落点 + 本次耗时」：模型据此判断相对路径与耗时，回放也从这一行派生。
    // 只对执行层给出 `[shell]` 收尾行的调用追加；参数拒绝 / 起进程失败 / 围栏拒绝没有这一行。
    if api::did_run(&outcome.output) {
        outcome.output.push('\n');
        outcome.output.push_str(&api::cwd_elapsed_line(
            &cwd_view,
            started.elapsed().as_secs(),
        ));
    }
    let external = ctx.sandbox.permissions.decision_for(
        name,
        Effect::Exec,
        CallScope::External,
        Decision::Allow,
    );
    let outcome = api::with_scope_hint(outcome, call.scope, external);
    if let Some(slot) = &slot {
        slot.finish(
            outcome.ok,
            api::exit_code(&outcome.output),
            api::shell_status(&outcome.output).map(|s| s.as_str()),
            &outcome.output,
        );
    }
    outcome
}

/// 目的：外部工具派发：先定模块（信封里的 module；省略时只有唯一模块才兜底），再查该模块的工具表。
///   返回（实际使用的模块 id, 执行结果）。不猜：多模块 agent 下省略 module 直接如实报错。
pub(crate) fn dispatch_external(
    ctx: &MemberTools,
    inv: &ToolInvoke,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    grant: &CallGrant,
) -> (String, ToolOutcome) {
    let module = match inv.module.as_deref() {
        Some(m) => m.to_string(),
        None if ctx.modules.len() == 1 => ctx.modules.keys().next().cloned().unwrap_or_default(),
        None => {
            let why = format!(
                "工具 {} 没有指明所属模块（本 agent 有多个模块，核心不替你猜），请在信封里写 module",
                inv.name
            );
            return (String::new(), deny(ctx, why));
        }
    };
    let Some(mt) = ctx.modules.get(&module) else {
        let why = format!("本 agent 没有模块 {}", module);
        return (module.clone(), deny(ctx, why));
    };
    // 运行包未就绪（本档位下该模块的工具不执行）：如实报缺哪个能力，让模型换工具或告诉用户。
    if let Some(caps) = ctx.unavailable.get(&module) {
        let why = format!(
            "模块 {} 的运行包 {} 未装载，本会话不执行它的工具；请如实告知用户，并改用其它可用工具。",
            module,
            caps.join("、")
        );
        return (module.clone(), deny(ctx, why));
    }
    // 常驻服务的操作：不走外部命令，转给常驻服务能力（服务进程已在它自己的围栏里跑着）。
    if let Some(op) = mt.services.get(&inv.name) {
        let args: serde_json::Value = match serde_json::from_str(&inv.args_json) {
            Ok(a) => a,
            Err(e) => {
                let why = format!("参数不是合法 JSON：{}", e);
                return (module.clone(), deny(ctx, why));
            }
        };
        let outcome = match ctx.residents.call(&module, &op.service, &op.op, &args) {
            Ok(receipt) => ToolOutcome {
                ok: receipt.ok,
                output: receipt.output,
            },
            Err(e) => ToolOutcome {
                ok: false,
                output: e,
            },
        };
        return (module.clone(), outcome);
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
                                &full, book, &fault,
                            );
                            return (module.clone(), deny(ctx, why));
                        }
                    }
                    Err(e) => {
                        let why = format!("参数不是合法 JSON：{}", e);
                        return (module.clone(), deny(ctx, why));
                    }
                }
            }
            let env = ctx
                .module_env
                .get(&module)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            // 每次调用按生效 reach 派生围栏：sh 与模块工具同一把尺子，不再用会话并集围栏执行。
            // 容器身份 = 会话 + agent + 受保护根集合（见 container_name）；租约就是会话 id，调用结束不释放。
            let fence = crate::kernel::api::FenceSpec::for_call(
                &ctx.sandbox,
                &grant.reach,
                Some(&module),
                &ctx.fence,
            )
            .at(&mt.root);
            let outcome = if ctx.tier == crate::kernel::api::Tier::Vm {
                // 虚拟机档：命令送进这个会话的 guest；per-call 断网由 guest helper 落实。
                if ctx.guest_lease.is_empty() {
                    return (
                        module,
                        deny(ctx, "这条虚拟机档会话没有 guest 租约".to_string()),
                    );
                }
                // 共享盘不承载执行位：模块声明的相对入口先落进 guest 本地副本，再把首 token 换成它
                // （cwd 仍是共享的模块根，数据与 userdata 不动）。
                let declared = command.trim_start();
                let dispatched = match crate::capabilities::modules::api::local_entry(declared) {
                    Some(entry) => {
                        match ctx.vm.stage_exec(
                            &ctx.guest_lease,
                            &module,
                            &mt.root.join(entry),
                            entry,
                        ) {
                            Ok(staged) => {
                                format!("{}{}", staged, &declared[entry.len()..])
                            }
                            Err(why) => return (module, deny(ctx, why)),
                        }
                    }
                    None => declared.to_string(),
                };
                ctx.vm.guest_run_fenced(
                    &ctx.guest_lease,
                    &crate::capabilities::vm::api::GuestRun {
                        command: dispatched,
                        args_json: inv.args_json.clone(),
                        env: env.to_vec(),
                        cwd: mt.root.display().to_string(),
                        timeout_secs: 30,
                        output_limit: crate::capabilities::vm::api::OutputLimit::Head,
                    },
                    &fence,
                )
            } else {
                ctx.tools.run_module(
                    &module,
                    &fence,
                    command,
                    &inv.args_json,
                    env,
                    ctx.runtime_path.as_deref(),
                    mt.parallel.contains(&inv.name),
                    ask,
                )
            };
            (module, outcome)
        }
        None => {
            let why = format!("模块 {} 未声明工具 {}", module, inv.name);
            (module.clone(), deny(ctx, why))
        }
    }
}
