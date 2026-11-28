//! 目的：成员回合里的工具循环——声明面 → 放行判定 → 并发调度 → 执行（内置 / 模块外部）→ 回填。
//! 管：只认 MemberTools（这一回合的工具面与观察账本）；策略（放行表、并发调度）在这里。
//! 不管：模型调用（在 llm / session 端口后）；工具机制本身（在 tools 的端口后面）。
//! 联动：执行经 tools 的 ToolExec 端口；回填顺序与转录行形状见 docs/session/session-model.md。

use super::decls::ToolDecls;
use crate::kernel::api::types::ToolOutcome;
use crate::units::business::agentloop::api::ToolFace;
use crate::units::business::agentloop::domain::member_tools::MemberTools;
use crate::units::business::llm::api::ToolInvoke;
use crate::units::business::tools::api::{Observations, ShellFacts, ToolSchema};
use crate::units::business::transcript::api::SessionEvent;

/// 目的：一次调用的结果形态（标签，结果，观察事实，shell 结构化事实），只为把签名读短。
pub(crate) type CallDone = (
    String,
    ToolOutcome,
    Vec<crate::units::business::tools::api::ObservationFact>,
    Option<ShellFacts>,
);

/// 目的：分支执行体的结果形态，在 CallDone 前多带一份观察账本副本（并发批次按原序合并）。
pub(crate) type BranchDone = (
    String,
    ToolOutcome,
    Observations,
    Vec<crate::units::business::tools::api::ObservationFact>,
    Option<ShellFacts>,
);

/// 目的：一次调用的执行上下文（问卡、取消、批前观察快照），只为把签名读短。
pub(crate) struct CallCtl<'a> {
    pub ask: Option<&'a dyn crate::kernel::ports::AskUser>,
    pub cancel: &'a std::sync::atomic::AtomicBool,
    pub gate_facts: &'a [crate::units::business::tools::api::ObservationFact],
    pub budget: crate::units::business::tools::api::ResultBudget,
}

/// 目的：执行一次工具调用：内置优先；外部工具按模块走（模块为空时由 dispatch_external 如实报错）。
///   账本经**分支副本**回到本成员（见 run_branch）——串行与并发只有这一条执行路径。
pub(crate) fn run_one(
    ctx: &mut MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
    ctl: &CallCtl<'_>,
) -> CallDone {
    let (label, outcome, branch, facts, shell) = run_branch(ctx, module, name, args_json, ctl);
    // 串行：分支的副本就是"这次调用之后"的账本，直接接管（与逐个记账等价）。
    ctx.observations = branch;
    (label, outcome, facts, shell)
}

/// 目的：一次调用的执行体（**不碰本成员的账本**）：分支各持账本副本，由调用方按原始顺序合并/接管。
///   为什么这样：并发批次里多个调用同时跑，而账本是可变状态；只读类调用的结果不依赖账本，
///   所以"副本 + 按原序提交"与串行执行的结果完全相同（见 crate::units::business::tools::api::Observations::absorb）。
pub(crate) fn run_branch(
    ctx: &MemberTools,
    module: Option<&str>,
    name: &str,
    args_json: &str,
    ctl: &CallCtl<'_>,
) -> BranchDone {
    let CallCtl {
        ask,
        cancel,
        gate_facts,
        budget,
    } = ctl;
    let mut branch = ctx.observations.clone();
    branch.set_reply(ctx.reply_seq);
    let facts_before = branch.facts().len();
    // 这一趟若是一次 shell：执行层当场算好的结构化事实（耗时 / 围栏准备 / 退出码 / 状态 / 截断）。
    let mut shell: Option<ShellFacts> = None;
    let (label, outcome) = if !face_has(ctx, module, name) {
        // **按这一回合的工具面校验**：不在面里的调用一律如实拒绝（不静默执行、也不当表态）。
        (
            String::new(),
            crate::units::business::tools::api::refuse(name),
        )
    } else if let Some(outcome) = authorized(ctx, name, *ask) {
        // 在面内、但这一段不给用（或问了没点头）：回执说清原因；拒绝路径不产生任何副作用。
        (String::new(), outcome)
    } else if crate::units::business::tools::api::is_builtin(name) {
        // 门禁看的是"这一批调用发出时模型还能看到的观察"：批前快照，同批 read 不授权后面的 write。
        let facts = gate_facts.to_vec();
        // 审计口要交给门禁长期持有，类型收窄成不带 auto trait 的那一种（同一个对象）。
        let audit: std::sync::Arc<dyn crate::kernel::ports::Log> =
            std::sync::Arc::clone(&ctx.log) as std::sync::Arc<dyn crate::kernel::ports::Log>;
        let audit = Some(audit);
        let gate = ctx.driver.as_ref().and_then(|d| {
            d.gate(&crate::units::business::workbench::api::GateRequest {
                work: &ctx.face.bench.work_name,
                agent: &ctx.face.bench.agent,
                worktree: &ctx.face.bench.agent,
                facts: &facts,
                log: audit,
            })
        });
        let done = ctx.tools.run_builtin(
            &ctx.face.bench,
            &ctx.face.builtin_tools,
            branch,
            gate.as_deref(),
            *budget,
            name,
            args_json,
        );
        let mut outcome = done.outcome;
        branch = done.observations;
        let written = branch.take_written();
        let removed = branch.take_removed();
        if let Some(note) = ctx.driver.as_ref().and_then(|d| {
            d.after_write(&crate::units::business::workbench::api::WriteDone {
                bench: &ctx.face.bench,
                written: &written,
                removed: &removed,
                line: ctx.line.load(std::sync::atomic::Ordering::Relaxed),
            })
        }) {
            outcome.output.push_str(&note);
        }
        (String::new(), outcome)
    } else if crate::units::business::tools::api::is_exec(name) {
        let (outcome, facts) = run_shell(ctx, name, args_json, *ask, cancel, *budget);
        shell = facts;
        (String::new(), outcome)
    } else if let Some(h) = handler_for(ctx, name) {
        // 核心自有工具：与内置、模块走**同一条派发路径**，不是循环里的特例。
        let refusal = ctx.driver.as_ref().and_then(|d| {
            d.before(&crate::units::business::workbench::api::CallRequest {
                bench: &ctx.face.bench,
                name,
                args_json,
                facts: gate_facts,
            })
        });
        if let Some(outcome) = refusal {
            (String::new(), outcome)
        } else {
            let tctx = crate::kernel::ports::ToolCtx {
                session: &ctx.face.bench.session,
                agent: &ctx.face.bench.agent,
                line: ctx.line.load(std::sync::atomic::Ordering::Relaxed),
            };
            (String::new(), h.run(&tctx, name, args_json))
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
        dispatch_external(ctx, &inv, *ask, *budget)
    };
    // 统一兜底：无论内置、shell、模块还是核心自有工具，回执进上下文前都按这一批的预算收口。
    let mut outcome = outcome;
    crate::units::business::tools::api::cap_result_with(
        &mut outcome,
        crate::units::business::tools::api::continuation_for(module.unwrap_or(""), name),
        *budget,
    );
    let facts = if branch.facts().len() > facts_before {
        branch.facts()[facts_before..].to_vec()
    } else {
        Vec::new()
    };
    (label, outcome, branch, facts, shell)
}

/// 目的：授权判定——这一次调用准不准、要不要先问一次；没拿到点头就如实拒绝（fail-closed）。
/// 参数：`ctx` = 这一回合的工具面与身份；`name` = 要调的工具 id（已在面内）；`ask` = 会话的提问端口（没有就是没有前端）。
/// 返回：`None` = 放行；`Some(回执)` = 不执行，回执说明原因。
/// 约束：顺序固定为「在场（face_has，调用方已判）→ 授权（这里）→ 分发」；授权不写观察账本、不提交共享区。
fn authorized(
    ctx: &MemberTools,
    name: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
) -> Option<ToolOutcome> {
    use crate::units::business::agentloop::domain::policy::ToolRequest;
    let req = ToolRequest {
        tool: name,
        sources: &ctx.face.source_ids,
        loop_id: &ctx.face.loop_id,
        role: &ctx.face.role,
    };
    gate(ctx.policy.decide(&req), &req, &ctx.face.bench.agent, ask)
}

/// 目的：把策略的结论落成结果：准 → 放行；不准 → 拒绝；要问 → 问一次，只有「允许这一次」才放行。
/// 参数：`agent` = 撞上这一问的 agent 实例名（卡片信封第二段）。
/// 约束：唯独 `Chosen(OPT_ALLOW)` 放行；拒绝、没人答、停止、构不出选项、没有前端一律不执行。
fn gate(
    verdict: crate::units::business::agentloop::domain::policy::Verdict,
    req: &crate::units::business::agentloop::domain::policy::ToolRequest<'_>,
    agent: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
) -> Option<ToolOutcome> {
    use crate::kernel::api::types::AskOutcome;
    use crate::units::business::agentloop::domain::policy::{OPT_ALLOW, Verdict, ask_card};
    match verdict {
        Verdict::Allow => None,
        Verdict::Deny(why) => Some(refuse(why)),
        Verdict::Ask { why } => {
            let card = ask_card(agent, req, &why);
            let Some(port) = ask else {
                return Some(refuse(format!(
                    "这一问没人能答（这一趟没有可回答的前端）：{}；{} 没有执行。",
                    card.title, req.tool
                )));
            };
            match port.ask(&card) {
                AskOutcome::Chosen(id) if id == OPT_ALLOW => None,
                AskOutcome::Chosen(_) => Some(refuse(format!(
                    "你这一次没有同意（{}）：{} 没有执行。",
                    card.title, req.tool
                ))),
                AskOutcome::NoAnswer => Some(refuse(format!(
                    "这一问没人答（{}）：{} 没有执行——没人点头就当没放行。",
                    card.title, req.tool
                ))),
                AskOutcome::Stopped => Some(refuse(format!(
                    "这一问被停止（{}）：{} 没有执行。",
                    card.title, req.tool
                ))),
                AskOutcome::NoOptions => Some(refuse(format!(
                    "这一问构不出可答的选项（{}）：{} 没有执行。",
                    card.title, req.tool
                ))),
            }
        }
    }
}

/// 目的：一批结束后让工作台策略采纳不透明写者（shell / 模块工具）的改动，回执挂在这一批最后一个跑成的那次调用后面。
/// 约束：只有这一批里真的跑成过不透明工具才问策略；策略不带回合钩子（不版本化）时整段不做。
fn adopt_batch(
    ctx: &MemberTools,
    plan: &[(Option<String>, String, String)],
    done: &mut [Option<CallDone>],
    facts: &[crate::units::business::tools::api::ObservationFact],
) {
    let Some(driver) = ctx.driver.as_ref() else {
        return;
    };
    if ctx.face.bench.work_name.is_empty() {
        return;
    }
    // 最后一个跑成的不透明工具：回执挂它的结果后面（下一次生成能看到的通道）。
    let mut last: Option<usize> = None;
    for (i, (module, tool, _)) in plan.iter().enumerate() {
        if module.is_none() && !crate::units::business::tools::api::is_exec(tool) {
            continue;
        }
        if matches!(&done[i], Some((_, o, _, _)) if o.ok) {
            last = Some(i);
        }
    }
    let Some(i) = last else {
        return;
    };
    let note = driver.after_batch(&crate::units::business::workbench::api::BatchDone {
        bench: &ctx.face.bench,
        facts,
        line: ctx.line.load(std::sync::atomic::Ordering::Relaxed),
    });
    if let Some(note) = note
        && let Some((_, outcome, _, _)) = &mut done[i]
    {
        outcome.output.push_str(&note);
    }
}

/// 目的：这次调用在**本回合的工具面**里吗：内置与核心自有工具按 id 清单；模块工具按限定 id `<模块>.<工具>`
///   （判据只有这一处——执行侧与"如实说一句越权"都读它）。
pub(crate) fn face_has(ctx: &MemberTools, module: Option<&str>, name: &str) -> bool {
    // 系统工具册里的名字（含 `compact` 这类只由系统发起的工具）也按工具面校验：
    // 不能因为“没写 module”就落进模块工具的兜底分支。
    if crate::units::business::tools::api::is_builtin(name)
        || crate::units::business::tools::api::is_exec(name)
        || ctx.face.builtin_tools.contains_key(name)
    {
        ctx.face.allowed.iter().any(|t| t == name)
    } else if handler_for(ctx, name).is_some() {
        // 核心自有工具与内置**同口径**：名字在这一回合的工具面里，就是放行。
        ctx.face.allowed.iter().any(|t| t == name)
    } else {
        // 模块工具按限定 id 判：面里有 `<模块>.<工具>` 才是放行。
        // 没写 module 不算越权：那是"派发时消歧"的事，由 dispatch_external 如实说清（多模块下不猜）。
        let hit = |m: &str| {
            ctx.face
                .allowed
                .iter()
                .any(|t| t == &format!("{}.{}", m, name))
        };
        module
            .map(hit)
            .unwrap_or_else(|| ctx.face.modules.keys().any(|m| hit(m)))
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

/// 目的：现在在哪个段、哪个角色（回执里说清"我是谁"，模型才知道该换哪个来源要工具）。
fn where_of(ctx: &MemberTools) -> String {
    if ctx.face.loop_id.is_empty() {
        format!("角色 {}", ctx.face.role)
    } else {
        format!("段 {} · 角色 {}", ctx.face.loop_id, ctx.face.role)
    }
}

/// 目的：这一回合的面长什么样（前若干个 id + 总数），给越权与未知工具的回执用。
fn face_preview(ctx: &MemberTools, max: usize) -> String {
    if ctx.face.allowed.is_empty() {
        return "空".to_string();
    }
    let head: Vec<&str> = ctx
        .face
        .allowed
        .iter()
        .take(max)
        .map(String::as_str)
        .collect();
    if ctx.face.allowed.len() > max {
        format!("{}… 共 {} 个", head.join("、"), ctx.face.allowed.len())
    } else {
        format!("{}（共 {} 个）", head.join("、"), ctx.face.allowed.len())
    }
}

/// 目的：本回合的工具面之外的调用：**如实说一句**（用户可见），执行侧负责不执行（见 run_branch）。
/// 约束：回执必须说清当时在哪个段/角色、面里实际有哪些 id——模型据此换来源或改口，而不是反复试同一个名字。
pub(crate) fn note_unauthorized(
    ctx: &MemberTools,
    plan: &[(Option<String>, String, String)],
    sink: &mut dyn FnMut(SessionEvent),
) {
    for (module, tool, _args) in plan {
        if !face_has(ctx, module.as_deref(), tool) {
            sink(SessionEvent::Notice(format!(
                "[越权] 本席位没有工具 {}：{}，这一回合的面有 {}。本轮不执行、不当表态（如实拒绝）",
                tool,
                where_of(ctx),
                face_preview(ctx, 8)
            )));
        }
    }
}

/// 目的：这个工具有没有**声明可并发**（策略在册子/清单里，代码里不写名单）：
///   内置工具看 `prompts/shared/tools.yaml` 的 `builtin_tools.<名字>.parallel`，模块工具看 `module.yaml` 的 `tools.<名字>.parallel`。
///   未声明 = 独占串行；**没写 module 的外部工具也按独占**（那要等 dispatch 才知道是哪个模块，核心不猜）。
pub(crate) fn is_parallel(face: &ToolFace, module: Option<&str>, name: &str) -> bool {
    match module {
        Some(id) => face
            .modules
            .get(id)
            .map(|m| m.parallel.contains(name))
            .unwrap_or(false),
        None => {
            crate::units::business::tools::api::is_builtin(name)
                && face
                    .builtin_tools
                    .get(name)
                    .map(|s| s.parallel)
                    .unwrap_or(false)
        }
    }
}

/// 目的：模块工具的派生声明（模块不声明参数契约；参数原样交给它自己解释）。
fn module_schema() -> ToolSchema {
    ToolSchema {
        desc: String::new(),
        params: None,
        parallel: false,
        human: Vec::new(),
    }
}

/// 目的：按这次调用找到工具声明；系统工具读总表，模块工具用派生声明。
fn schema_for(ctx: &MemberTools, module: Option<&str>, tool: &str) -> Option<ToolSchema> {
    if crate::units::business::tools::api::is_builtin(tool)
        || crate::units::business::tools::api::is_exec(tool)
        || handler_for(ctx, tool).is_some()
    {
        return ctx.face.builtin_tools.get(tool).cloned();
    }
    let id = match module {
        Some(m) => m,
        None if ctx.face.modules.len() == 1 => ctx.face.modules.keys().next()?.as_str(),
        None => return None,
    };
    let mt = ctx.face.modules.get(id)?;
    if mt.commands.contains_key(tool) || mt.services.contains_key(tool) {
        Some(module_schema())
    } else {
        None
    }
}

/// 目的：执行前的预检——未知工具、非法的 shell 参数（命令长度 / timeout）都在这里如实拒绝。
/// 约束：预检不过的调用不执行、不加进并发批；返回的 Err 就是回给模型的那句事实。
fn check_call(
    ctx: &MemberTools,
    module: Option<&str>,
    tool: &str,
    args_json: &str,
) -> Result<(), String> {
    let schema = match schema_for(ctx, module, tool) {
        Some(s) => s,
        // 模块工具的名字/归属由 dispatch 如实说清（没指明模块、没有这个模块、模块没声明这个工具）。
        None if !ctx.face.modules.is_empty() => module_schema(),
        None => {
            return Err(format!(
                "未知工具：{}（{}，这一回合的面有 {}）",
                tool,
                where_of(ctx),
                face_preview(ctx, 8)
            ));
        }
    };
    // shell 的语义校验（命令长度 / timeout）放在执行前：非法命令不落进程。
    if crate::units::business::tools::api::is_exec(tool) {
        crate::units::business::tools::api::parse_shell(&schema, args_json)?;
    }
    Ok(())
}

/// 目的：执行一批调用：连续声明可并发的合成一批并发跑，其余各自独占；结果按原始下标返回。
///   每次调用先过预检：未知工具 / 非法 shell 参数如实拒绝、不执行。
pub(crate) fn run_batch(
    ctx: &mut MemberTools,
    plan: &[(Option<String>, String, String)],
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    cancel: &std::sync::atomic::AtomicBool,
    budget: crate::units::business::tools::api::ResultBudget,
) -> Vec<CallDone> {
    // 预检：剩余上下文连一条最小回执都放不下时，整批都不执行——避免 shell 已经产生副作用却看不到输出。
    if budget.exhausted() {
        let why = "上下文剩余不足：这次调用没有执行；请先压缩会话，再重试。".to_string();
        return plan
            .iter()
            .map(|(module, _tool, _args)| {
                (
                    module.clone().unwrap_or_default(),
                    ToolOutcome::new(false, why.clone()),
                    Vec::new(),
                    None,
                )
            })
            .collect();
    }
    let mut done: Vec<Option<CallDone>> = (0..plan.len()).map(|_| None).collect();
    let checks: Vec<Result<(), String>> = plan
        .iter()
        .map(|(module, tool, args)| check_call(ctx, module.as_deref(), tool, args))
        .collect();
    // 同批调用共享批前观察快照：这一批里 read 出来的内容，模型还没看到，不能授权同批后面的写。
    let gate_facts = ctx.observations.facts().to_vec();
    let mut i = 0;
    while i < plan.len() {
        let (module, tool, args) = &plan[i];
        let label = module.clone().unwrap_or_default();
        if let Err(why) = &checks[i] {
            done[i] = Some((label, deny(&ctx.face, why.clone()), Vec::new(), None));
            i += 1;
            continue;
        }
        if is_parallel(&ctx.face, module.as_deref(), tool) {
            let mut j = i;
            while j < plan.len()
                && is_parallel(&ctx.face, plan[j].0.as_deref(), &plan[j].1)
                && checks[j].is_ok()
            {
                j += 1;
            }
            let ctx_ref: &MemberTools = ctx;
            let batch: Vec<BranchDone> = std::thread::scope(|s| {
                let handles: Vec<_> = plan[i..j]
                    .iter()
                    .map(|(module, tool, args)| {
                        let facts = gate_facts.clone();
                        s.spawn(move || {
                            run_branch(
                                ctx_ref,
                                module.as_deref(),
                                tool,
                                args,
                                &CallCtl {
                                    ask,
                                    cancel,
                                    gate_facts: &facts,
                                    budget,
                                },
                            )
                        })
                    })
                    .collect();
                handles
                    .into_iter()
                    .map(|h| h.join().unwrap_or_else(|e| std::panic::resume_unwind(e)))
                    .collect()
            });
            for (k, (label, outcome, branch, facts, shell)) in batch.into_iter().enumerate() {
                ctx.observations.absorb(&branch);
                done[i + k] = Some((label, outcome, facts, shell));
            }
            i = j;
        } else {
            done[i] = Some(run_one(
                ctx,
                module.as_deref(),
                tool,
                args,
                &CallCtl {
                    ask,
                    cancel,
                    gate_facts: &gate_facts,
                    budget,
                },
            ));
            i += 1;
        }
    }
    adopt_batch(ctx, plan, &mut done, &gate_facts);
    // 每一次调用都留一条**同形状的审计记录**（与呈现层的动作记录同一处格式化）：可查、可对账。
    for ((module, tool, _args), slot) in plan.iter().zip(done.iter()) {
        if let Some((_label, outcome, _facts, _shell)) = slot {
            let name = crate::units::business::transcript::api::qualify(module.as_deref(), tool);
            ctx.log.info(
                "action",
                &crate::units::business::tools::api::action_audit(
                    &ctx.face.role,
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

/// 目的：本成员这次请求要声明的工具（原生通道用）：内置工具 + 模块工具。
///   原生协议里**没有 module 字段**，所以跨模块同名工具靠线上名消歧（{模块}_{工具}，撞名再加序号）；
///   返回的映射把线上名翻回（模块, 工具）。模块没声明参数的照旧声明（参数结构交回给工具自己解释）。
pub(crate) fn tool_decls(face: &ToolFace) -> ToolDecls {
    let mut decls = ToolDecls::default();
    let mut taken: Vec<String> = Vec::new();
    // **只声明这个席位拿到的工具**（判据与执行时校验同一份 allowed）：声明了却调不动没有意义，
    // 模型会照着声明去调，被拒一次就白烧一轮（见 docs/tools/tools-and-roles.md 二）。
    for (name, schema) in &face.builtin_tools {
        if !face.allowed.iter().any(|t| t == name) {
            continue;
        }
        // patch 是自由格式：它的声明单独写（参数是 body 字符串，不是 JSON 信封的 args）
        if crate::units::business::tools::api::is_freeform(name) {
            continue;
        }
        decls.list.push(schema.decl(name));
        taken.push(name.clone());
        decls.wire.insert(name.clone(), (None, name.clone()));
    }
    if face
        .allowed
        .iter()
        .any(|t| t == crate::units::business::tools::api::PATCH)
    {
        let patch = crate::units::business::tools::api::patch_decl();
        taken.push(patch.name.clone());
        decls.wire.insert(
            patch.name.clone(),
            (None, crate::units::business::tools::api::PATCH.to_string()),
        );
        decls.list.push(patch);
    }
    // 模块工具按**成员归属**发放（不是席位的通用属性）：面里没有 `<模块>.<工具>` 就不声明它们。
    for (id, mt) in &face.modules {
        for tool in mt.commands.keys() {
            if !face
                .allowed
                .iter()
                .any(|t| t == &format!("{}.{}", id, tool))
            {
                continue;
            }
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
                None => crate::units::business::llm::api::ToolDecl {
                    name: wire_name.clone(),
                    description: format!("模块 {} 的外部工具 {}（参数由工具自己解释）", id, tool),
                    parameters: serde_json::json!({ "type": "object", "additionalProperties": true }),
                },
            };
            decls.list.push(decl);
        }
        // 已启动常驻服务的操作：与外部工具同一条声明路径（线上名里的点换成下划线，供应商函数名不收点）。
        for tool in mt.services.keys() {
            if !face
                .allowed
                .iter()
                .any(|t| t == &format!("{}.{}", id, tool))
            {
                continue;
            }
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
            decls.list.push(crate::units::business::llm::api::ToolDecl {
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

/// 目的：可用的外部工具清单：**只列这一回合工具面上真有的东西**（模块工具 + 系统工具），
///   三种失败（模块不存在 / 模块没这个工具 / 缺 module 且模块不唯一）都用它把可选范围说回去。
/// 约束：模块工具按 `with_modules` 的发放口径列；系统工具只列 `allowed`——总表不是这一回合的面，
///   列出去会让模型以为能调，同一句回执里再说「没有这个工具」就自相矛盾了。
pub(crate) fn available_tools(face: &ToolFace) -> String {
    let mut list: Vec<String> = Vec::new();
    for (id, mt) in &face.modules {
        for name in mt.commands.keys().chain(mt.services.keys()) {
            let q = format!("{}.{}", id, name);
            if face.allowed.contains(&q) {
                list.push(q);
            }
        }
    }
    list.extend(face.allowed.iter().cloned());
    list.join("、")
}

/// 目的：拒绝回执，**不**附工具清单——档位、确认、放行、参数这类拒绝与「换个工具」无关，清单只会扰民。
pub(crate) fn refuse(why: String) -> ToolOutcome {
    ToolOutcome::new(false, why)
}

/// 目的：发现类拒绝（没指明模块 / 没有这个模块 / 模块没声明这个工具）：把可选范围说回去。
/// 约束：`why` 自带的句号先去掉，别拼出「。。」。
pub(crate) fn deny(face: &ToolFace, why: String) -> ToolOutcome {
    ToolOutcome::new(
        false,
        format!(
            "{}\n可用：{}",
            why.trim_end_matches('。'),
            available_tools(face)
        ),
    )
}

/// 目的：执行一次 shell：按会话的固定授予派生围栏，host 走 ProcessRunner，VM 走 guest。
/// 约束：没有 scope 这类范围旋钮；围栏恒为沙箱 rw + 模块目录只读 + 主机只读根，网络按会话档位。
pub(crate) fn run_shell(
    ctx: &MemberTools,
    name: &str,
    args_json: &str,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    cancel: &std::sync::atomic::AtomicBool,
    budget: crate::units::business::tools::api::ResultBudget,
) -> (ToolOutcome, Option<ShellFacts>) {
    use crate::units::business::tools::api;
    let Some(schema) = ctx.face.builtin_tools.get(name) else {
        return (api::refuse(name), None);
    };
    let call = match api::parse_shell(schema, args_json) {
        Ok(c) => c,
        Err(e) => {
            return (ToolOutcome::new(false, e), None);
        }
    };
    if ctx.face.bench.private.as_os_str().is_empty() {
        return (
            ToolOutcome::new(
                false,
                "shell 没有私有沙箱（cwd 为空），这次调用没有执行。".to_string(),
            ),
            None,
        );
    }
    let mut fence =
        crate::kernel::api::fence::FencePlan::for_call(&ctx.face.bench, None, &ctx.face.fence)
            .at(&ctx.face.bench.private);
    fence.extend_read_tree(ctx.face.runtime_roots.iter().cloned());
    // 要连宿主回环时才换网络档：容器侧起中继、按端口放行（宿主侧还会拒产品自己的端口）。
    // 能不能在本档这么做由执行面自己判（虚拟机档的 guest 回环不映射宿主端口）。
    if !call.local_ports.is_empty() {
        fence.net = crate::kernel::api::fence::NetPolicy::Local {
            ports: call.local_ports.clone(),
        };
    }
    let started = std::time::Instant::now();
    // cwd 用**本环境写法**：本机档是宿主路径，虚拟机档是 guest 内路径（与文件工具、提示词同一座路径桥）。
    let cwd_view = match ctx.face.bench.path_codec.as_ref() {
        Some(codec) => codec.to_view(&ctx.face.bench.private),
        None => ctx.face.bench.private.display().to_string(),
    };
    let slot = ctx.shell_live.clone();
    if let Some(slot) = &slot {
        slot.start(&call.script, &cwd_view);
    }
    // 执行面自己认这一档（本机 / 虚拟机），引擎只递请求：档位差异不外泄到这里。
    let exec = crate::units::business::sandbox::api::ExecRequest {
        kind: crate::units::business::sandbox::api::ExecKind::Shell {
            script: &call.script,
        },
        cwd: &ctx.face.bench.private,
        fence,
        timeout: call.timeout,
        ask,
        cancel: Some(cancel),
        budget,
    };
    if let Some(why) = ctx.exec.precheck(&exec) {
        return (refuse(why), None);
    }
    let got = ctx.exec.run(&exec);
    let fence_prep = got.fence_prep;
    let mut outcome = got.outcome;
    // 共享区不承载执行位：相对路径脚本被直接执行（EACCES）时，补一条"用解释器跑"的说明。
    if let Some(note) = api::shared_exec_note(&call.script, &outcome.output) {
        outcome.push_note(note);
    }
    // 回执补「落点 + 本次耗时」：模型据此判断相对路径与耗时（围栏准备到一秒才单独报一条）。
    // 只对执行层给出收尾说明的调用追加；耗时只取一次：回执行与结构化事实同源（不让两处各测一次）。
    let elapsed = started.elapsed();
    let ran = api::did_run(&outcome.notes);
    if ran {
        for note in api::place_notes(
            &cwd_view,
            elapsed.as_secs(),
            fence_prep.map(|t| t.as_secs()),
        ) {
            outcome.push_note(note);
        }
    }
    // 事实只从这一次执行产出：实况槽与会话回放读同一份，呈现层不再解析回执。
    // 没跑成（围栏拒绝、启动失败）也留一条：落点与耗时是真的，退出码与状态缺省，界面据此显示"最近一次尝试"。
    let facts = api::shell_facts(&cwd_view, elapsed, fence_prep, &outcome.notes);
    if let Some(slot) = &slot {
        slot.finish(Some(&facts), outcome.ok, &outcome.output);
    }
    (outcome, Some(facts))
}

/// 目的：外部工具派发：先定模块（信封里的 module；省略时只有唯一模块才兜底），再查该模块的工具表。
///   返回（实际使用的模块 id, 执行结果）。不猜：多模块 agent 下省略 module 直接如实报错。
pub(crate) fn dispatch_external(
    ctx: &MemberTools,
    inv: &ToolInvoke,
    ask: Option<&dyn crate::kernel::ports::AskUser>,
    budget: crate::units::business::tools::api::ResultBudget,
) -> (String, ToolOutcome) {
    let module = match inv.module.as_deref() {
        Some(m) => m.to_string(),
        None if ctx.face.modules.len() == 1 => {
            ctx.face.modules.keys().next().cloned().unwrap_or_default()
        }
        None => {
            let why = format!(
                "工具 {} 没有指明所属模块（本 agent 有多个模块，核心不替你猜），请在信封里写 module",
                inv.name
            );
            return (String::new(), deny(&ctx.face, why));
        }
    };
    let Some(mt) = ctx.face.modules.get(&module) else {
        let why = format!("本 agent 没有模块 {}", module);
        return (module.clone(), deny(&ctx.face, why));
    };
    // 运行包未就绪（本档位下该模块的工具不执行）：如实报缺哪个能力，让模型换工具或告诉用户。
    if let Some(caps) = ctx.face.unavailable.get(&module) {
        let why = format!(
            "模块 {} 的运行包 {} 未装载，本会话不执行它的工具；请如实告知用户，并改用其它可用工具。",
            module,
            caps.join("、")
        );
        return (module.clone(), deny(&ctx.face, why));
    }
    // 常驻服务的操作：不走外部命令，转给常驻服务单元（服务进程已在它自己的围栏里跑着）。
    if let Some(op) = mt.services.get(&inv.name) {
        let args: serde_json::Value = match serde_json::from_str(&inv.args_json) {
            Ok(a) => a,
            Err(e) => {
                let why = format!("参数不是合法 JSON：{}", e);
                return (module.clone(), refuse(why));
            }
        };
        let outcome = match ctx.residents.call(&module, &op.service, &op.op, &args) {
            Ok(receipt) => ToolOutcome::new(receipt.ok, receipt.output),
            Err(e) => ToolOutcome::new(false, e),
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
                        if let Err(fault) = book.check_model(&args) {
                            let why = crate::units::business::tools::api::arg_fault_text(
                                &full, book, &fault,
                            );
                            return (module.clone(), refuse(why));
                        }
                    }
                    Err(e) => {
                        let why = format!("参数不是合法 JSON：{}", e);
                        return (module.clone(), refuse(why));
                    }
                }
            }
            let env = ctx
                .face
                .module_env
                .get(&module)
                .map(|v| v.as_slice())
                .unwrap_or(&[]);
            // 每次调用按固定授予派生围栏：sh 与模块工具同一把尺子（模块目录恒只读，该模块 userdata 可写）。
            // 容器身份 = 会话 + agent + 受保护根集合（见 container_name）；租约就是会话 id，调用结束不释放。
            let fence = crate::kernel::api::fence::FencePlan::for_call(
                &ctx.face.bench,
                Some(&module),
                &ctx.face.fence,
            )
            .at(&mt.root);
            // 执行面自己认这一档：本机直接起进程，虚拟机先把相对入口落进 guest 本地副本再跑。
            let exec = crate::units::business::sandbox::api::ExecRequest {
                kind: crate::units::business::sandbox::api::ExecKind::Module {
                    module: &module,
                    command,
                    args_json: &inv.args_json,
                    env,
                    parallel: mt.parallel.contains(&inv.name),
                },
                cwd: &mt.root,
                fence,
                timeout: std::time::Duration::from_secs(30),
                ask,
                cancel: None,
                budget,
            };
            if let Some(why) = ctx.exec.precheck(&exec) {
                return (module, refuse(why));
            }
            let outcome = ctx.exec.run(&exec).outcome;
            (module, outcome)
        }
        None => {
            let why = format!("模块 {} 未声明工具 {}", module, inv.name);
            (module.clone(), deny(&ctx.face, why))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::api::types::{Ask, AskOutcome};
    use crate::kernel::ports::AskUser;
    use crate::units::business::agentloop::domain::policy::{
        OPT_ALLOW, OPT_DENY, ToolRequest, Verdict,
    };

    /// 目的：按固定答复回话的提问端口——同时记下问过哪几张卡。
    struct Answering {
        /// 这一次的答复。
        reply: AskOutcome,
        /// 问过的卡片标题（断言"该问才问"）。
        seen: std::sync::Mutex<Vec<String>>,
    }

    impl Answering {
        /// 目的：造一个只会回 \`reply\` 的端口。
        fn new(reply: AskOutcome) -> Answering {
            Answering {
                reply,
                seen: std::sync::Mutex::new(Vec::new()),
            }
        }
    }

    impl AskUser for Answering {
        fn ask(&self, ask: &Ask) -> AskOutcome {
            self.seen.lock().expect("锁没坏").push(ask.title.clone());
            self.reply.clone()
        }
        fn halt(&self, _why: &str) {}
    }

    /// 目的：答了「允许这一次」才放行——这是唯一的放行路径。
    #[test]
    fn an_allowed_answer_lets_the_call_through() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let req = ToolRequest {
            tool: crate::units::business::tools::api::WRITE,
            sources: &sources,
            loop_id: "asking",
            role: "solo",
        };
        let port = Answering::new(AskOutcome::Chosen(OPT_ALLOW.to_string()));
        let verdict = Verdict::Ask {
            why: "要你点头".to_string(),
        };
        assert!(gate(verdict, &req, "甲", Some(&port)).is_none());
        assert_eq!(port.seen.lock().expect("锁没坏").len(), 1, "该问且只问一次");
    }

    /// 目的：答「不允许」就不执行，回执说清是哪一问没通过。
    #[test]
    fn a_refused_answer_stops_the_call() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let req = ToolRequest {
            tool: crate::units::business::tools::api::WRITE,
            sources: &sources,
            loop_id: "asking",
            role: "solo",
        };
        let port = Answering::new(AskOutcome::Chosen(OPT_DENY.to_string()));
        let verdict = Verdict::Ask {
            why: "要你点头".to_string(),
        };
        let out = gate(verdict, &req, "甲", Some(&port)).expect("不许放行");
        assert!(!out.ok);
        assert!(out.output.contains("没有执行"), "{}", out.output);
    }

    /// 目的：没人答就当没放行（不静默执行）。
    #[test]
    fn nobody_answering_stops_the_call() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let req = ToolRequest {
            tool: crate::units::business::tools::api::WRITE,
            sources: &sources,
            loop_id: "asking",
            role: "solo",
        };
        let port = Answering::new(AskOutcome::NoAnswer);
        let verdict = Verdict::Ask {
            why: "要你点头".to_string(),
        };
        let out = gate(verdict, &req, "甲", Some(&port)).expect("不许放行");
        assert!(!out.ok);
        assert!(out.output.contains("没人点头"), "{}", out.output);
    }

    /// 目的：没有前端就不问、也不执行。
    #[test]
    fn with_no_frontend_the_call_never_runs() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let req = ToolRequest {
            tool: crate::units::business::tools::api::WRITE,
            sources: &sources,
            loop_id: "asking",
            role: "solo",
        };
        let verdict = Verdict::Ask {
            why: "要你点头".to_string(),
        };
        let out = gate(verdict, &req, "甲", None).expect("不许放行");
        assert!(!out.ok);
        assert!(out.output.contains("没有可回答的前端"), "{}", out.output);
    }

    /// 目的：策略直接拒绝的动作不去问用户（问了也放行不了）。
    #[test]
    fn a_denied_policy_never_asks() {
        let sources: Vec<String> =
            vec![crate::units::business::session_model::api::SOURCE_AGENT.to_string()];
        let req = ToolRequest {
            tool: crate::units::business::tools::api::WRITE,
            sources: &sources,
            loop_id: "discuss",
            role: "discussant",
        };
        let port = Answering::new(AskOutcome::Chosen(OPT_ALLOW.to_string()));
        let out = gate(
            Verdict::Deny("这一段不给写".to_string()),
            &req,
            "甲",
            Some(&port),
        )
        .expect("不许放行");
        assert!(!out.ok);
        assert!(out.output.contains("这一段不给写"), "{}", out.output);
        assert!(port.seen.lock().expect("锁没坏").is_empty(), "不许问");
    }
}
