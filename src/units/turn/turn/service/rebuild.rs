//! 目的：turn 侧的会话重建——从落盘的事件与元数据重建出一条会话（单 agent / 协作 / 代理三种形态）。
//! 管：外部工作区绑定的复检与绑定、`rebuild_session` 的三形态分支、转录还原（`replay_ledgers` / `replay_dialogue`）、代理会话的装配。
//! 不管：准入与派发闸（`service/admit.rs`）、沙箱与权限事实（`service/facts.rs`）、会话怎么落盘（`session`）。
//! 联动：`runtime` 的启用与回档只经这里重建会话；代理会话由 `runtime` 的代理用例经 `build_proxy` 取。

use std::collections::BTreeMap;
use std::sync::Arc;

use crate::units::catalog::modules::api::Module;
use crate::units::catalog::registry::api::Registry;
use crate::units::conversation::session::api::{
    AgentSession, SessionMeta, SessionParams, guest_lease_for, work_root, work_root_of,
};
use crate::units::llm::api::{Channel, Msg};
use crate::units::turn::turn::api::Session;
use crate::units::turn::turn::service::session::Turn;
use crate::units::workspace::api::{Sandbox, WORKIGNORE};
use crate::kernel::ports::HostProbe;

/// 目的：代理会话的私有沙箱目录名（只有这一处）。
pub const PROXY_PRIVATE_DIR: &str = ".proxy";
/// 目的：代理会话里发言席的名字（只有这一处）。
pub const SPEAKER: &str = "核心";

/// 目的：会话重建与代理会话装配。
impl Turn {
    /// 目的：这条会话的外部工作区绑定：自己或顶层会话写过绑定就复检并返回根，没写过就是 None。
    /// 参数：`probe` = 宿主探针（复检要规范化路径与读元数据）。
    /// 错误：绑定路径已失效或不可写时如实报错（加载复检，见 `workspace::api::recheck`）。
    pub fn workspace_binding_of(
        &self,
        meta: &SessionMeta,
        probe: &dyn HostProbe,
    ) -> Result<Option<std::path::PathBuf>, String> {
        let (root, tier) = match &meta.workspace {
            Some(binding) => (binding.root.clone(), meta.exec.tier),
            None => match &meta.parent {
                None => return Ok(None),
                Some(p) => {
                    let top = work_root(self.history.as_ref(), p)?;
                    let top_meta = self.history.meta(&top)?;
                    match top_meta.workspace {
                        Some(binding) => (binding.root, top_meta.exec.tier),
                        None => return Ok(None),
                    }
                }
            },
        };
        let product = crate::units::workspace::api::product_root(&self.root, probe);
        crate::units::workspace::api::recheck(&root, tier, &product, probe)?;
        Ok(Some(root))
    }

    /// 目的：从落盘的事件与元数据重建一条会话：布局、沙箱、权限、模型通道与工具面都按 meta 复原。
    /// 参数：`registry` = 登记处（通道、模型、工具模式）；`probe` = 宿主探针。
    /// 错误：meta 与落盘不一致（缺名单 / 模块已不在清单 / 缺沙箱 / 无可用通道）时如实报错。
    pub fn rebuild_session(
        &self,
        meta: &SessionMeta,
        events: &[serde_json::Value],
        registry: &dyn Registry,
        probe: &dyn HostProbe,
    ) -> Result<crate::units::turn::turn::api::Session, String> {
        let roster = self.modules.roster();
        // 打开 / 重建时按 meta 刷新布局投影（会话目录可能被复制或迁移；meta 是权威）。
        let work = work_root_of(self.history.as_ref(), meta)?;
        // 过期清理开关与保底都是**工作根**的会话级事实：子会话沿根继承，自己 meta 里那份不作准。
        let root_meta = self.history.meta(&work)?;
        let freshness = (
            root_meta.context_freshness,
            root_meta.freshness_keep_percent,
        );
        let binding = self.workspace_binding_of(meta, probe)?;
        self.workspace.bind(&work, binding.as_deref())?;
        if binding.is_some() {
            // 旧会话首次启用：补一次初始导入（有 head 时无操作）。
            if let Err(e) = self
                .workspace
                .work_import_root(&work, crate::kernel::api::procs::unix_seconds())
            {
                self.log.warn(
                    "turn::rebuild_session",
                    &format!("工作区初始导入失败：{}", e),
                );
            }
        }
        let sandboxes = self.sandboxes(registry, probe, meta, &roster)?;
        // 整队的权威是**转录**：重建之前先把队清空并按已发出的卡号续号（内存里那一份不作数）。
        let issued = crate::units::conversation::session::api::issued_in(events);
        let desk = self.desk_for(&meta.name, issued);
        desk.rebuild(issued);
        let built = if meta.spec.driven() {
            {
                // 按这条会话声明里的流程序列重建：本层不认识任何具体流程（工厂是组合根注入的）。
                let ids = meta.spec.flow_ids();
                let setup = crate::units::conversation::session::api::FlowSetup {
                    settings: registry.snapshot(),
                    core_channel: None,
                    roster: meta.agents.clone(),
                    delegated: meta.delegate,
                    sandboxes: sandboxes.clone(),
                    // 重建出来的整队进**同一条队**（按转录里已发出的卡号续号，已答过的不重问）。
                    desk: Arc::clone(&desk),
                    restore: Some((meta, events)),
                };
                let pipeline = crate::units::conversation::session::api::sequence_of(
                    &ids,
                    &setup,
                    &*self.flows,
                )?;
                Session::Flow(Box::new(pipeline))
            }
        } else if !meta.is_proxy() {
            // 没有流程的单 agent：按 meta.agents[0] 重建（名单是唯一真相；模块数不限）。
            {
                let a = meta
                    .agents
                    .first()
                    .ok_or_else(|| format!("会话 {} 缺少 agent 名单", meta.name))?;
                let modules: Vec<Module> = a
                    .modules
                    .iter()
                    .filter_map(|id| {
                        roster
                            .modules
                            .iter()
                            .find(|m| &m.manifest.id == id)
                            .cloned()
                    })
                    .collect();
                if modules.len() != a.modules.len() {
                    return Err(format!("agent {} 的模块已不在清单", a.name));
                }
                let sb = sandboxes.for_agent(&a.name).cloned().ok_or_else(|| {
                    format!("会话 {} 缺少 agent {} 的沙箱信息", meta.name, a.name)
                })?;
                let channel = registry
                    .channel(a.model.as_deref())
                    .map_err(|e| format!("agent {} 没有可用模型通道：{}", a.name, e))?;
                // 重建时同样按登记处派生形态：系统提示与实际协议必须一致（回放才与实时一致）
                let mode = registry.tool_mode(a.model.as_deref());
                // **会话参数**：与建立时同一个口径（身份块每回合现渲染，不进消息列表）。
                let params =
                    crate::units::conversation::session::api::SessionParams::from_workspace(
                        &a.name,
                        &sb,
                        &modules,
                        &meta.spec.seat,
                    )
                    .with_chara(a.chara.clone())
                    .with_guest(crate::units::vm::api::guest_os_of(
                        &self.root, probe, &meta.exec,
                    ));
                let (history, marks, line_reply, line_ids, compacted_upto, compacted_line) =
                    replay_dialogue(events, mode);
                let unavailable = crate::units::vm::api::unavailable_modules(
                    &self.root,
                    probe,
                    &self.packages.library(),
                    &self.packages.base(),
                    &meta.exec,
                    &modules,
                );
                // 席位取落盘声明里的那一席（成员 / 节点会话是 executor）——与实时同一个口径。
                let seat = meta.spec.seat.clone();
                let host_plan = if meta.exec.tier == crate::kernel::api::types::Tier::Host {
                    crate::units::vm::api::host_plan_for(
                        &self.packages.library(),
                        &self.packages.base(),
                        probe,
                        &meta.exec,
                        &modules,
                        crate::units::catalog::packages::api::PinPolicy::Require,
                    )
                } else {
                    crate::units::catalog::packages::api::PackagePlan::default()
                };
                // 工具环境事实（围栏 / 注入项 / 运行路径 / 工具链根）由这里经 `env` 派生后按值拷进 `session`。
                let member = crate::units::conversation::session::api::MemberSpec {
                    modules: &modules,
                    sandbox: &sb,
                    unavailable,
                    mode,
                    // 身份与席位**同一把尺子**（重建与实时不能各写一份）。
                    seat: &seat,
                    tier: meta.exec.tier,
                    guest_lease: guest_lease_for(self.history.as_ref(), meta)?,
                    shell: self.shell_slot(&sb.session),
                    env: self.member_env(
                        registry,
                        &modules,
                        &sb,
                        crate::units::turn::env::api::network_allowed(registry.app()),
                        meta.exec.tier,
                        &host_plan,
                    ),
                };
                // 重建也要恢复上下文预算与最近一次用量：否则重启后自动压缩与状态栏都失忆。
                let (window, percent, keep_percent) =
                    self.context_budget(registry, a.model.as_deref());
                // 回复 id 跨重启单调：从转录里的最大值续号，否则新回复会与旧回复并成一组。
                let (observations, reads) = replay_ledgers(events, compacted_upto);
                let spec = crate::units::conversation::session::api::RebuildSpec {
                    params,
                    member,
                    channel,
                    dialogue: history,
                    marks,
                    line_reply,
                    line_ids,
                    compacted_upto,
                    compacted_line,
                    reply_seq: crate::units::conversation::session::api::max_reply(events),
                    context: (window, percent, keep_percent),
                    model: a.model.clone().or_else(|| registry.core_model()),
                    freshness: freshness.0,
                    freshness_keep_percent: freshness.1,
                    reads,
                    observations,
                    usage: crate::units::conversation::session::api::last_usage(events),
                };
                Session::Single(
                    crate::units::conversation::session::api::rebuild_single(
                        &self.session_ports(),
                        spec,
                    ),
                )
            }
        } else {
            // 代理会话：没有 agent（核心自己说话）；外壳由 build_proxy 装配，回放与单 agent 共用同一件。
            {
                let mut s = self.build_proxy(meta, registry)?;
                let (history, marks, line_reply, line_ids, compacted_upto, compacted_line) =
                    replay_dialogue(events, s.tool_mode());
                s.dialogue = history;
                s.marks = marks;
                s.line_reply = line_reply;
                s.line_ids = line_ids;
                s.compacted_upto = compacted_upto;
                s.compacted_line = compacted_line;
                s.next_line = s.marks.len() as u64;
                s.set_reply_seq(crate::units::conversation::session::api::max_reply(
                    events,
                ));
                s.set_context_freshness(freshness.0);
                s.set_freshness_keep_percent(freshness.1);
                let (observations, reads) = replay_ledgers(events, compacted_upto);
                s.restore_observations(observations);
                for (reply, body, fact) in reads {
                    s.note_read_at(body, reply, fact);
                }
                s.set_last_usage(crate::units::conversation::session::api::last_usage(
                    events,
                ));
                Session::Single(s)
            }
        };
        // 状态栏的 shell 快照同样按转录派生：重启 / 启用 / 回档重建后不再"失忆"。
        self.shell_slot(&meta.name).set(
            crate::units::conversation::session::api::last_shell(events),
        );
        Ok(built)
    }
}

/// 目的：把转录里的观察事实与读事实**一次扫描**装回两个账本：写门禁的账本 + 发送视图的读台账。
///
/// 约束：两个账本必须同水位——压缩点之前的行既不再授权写、也不再参与过期替换；
///   回档后的转录本身已截断，不另判。正文按 `tool_result_body` 现算，必须与 `reply_msgs`
///   同源（发送视图按正文找消息）。
fn replay_ledgers(
    events: &[serde_json::Value],
    compacted_upto: u64,
) -> (
    Vec<crate::units::tools::api::ObservationFact>,
    Vec<(u64, String, crate::units::workspace::api::ReadFact)>,
) {
    let mut facts = Vec::new();
    let mut reads = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for l in lines {
            let Some(t) = l.get("tool") else {
                continue;
            };
            let Ok(c) = serde_json::from_value::<
                crate::units::conversation::session::api::ToolCallView,
            >(t.clone()) else {
                continue;
            };
            let body = crate::units::conversation::session::api::tool_result_body(&c);
            for f in c.observations {
                if f.schema != crate::units::tools::api::OBSERVATION_FORMAT
                    || f.reply < compacted_upto
                {
                    continue;
                }
                for read in f.read_facts() {
                    reads.push((f.reply, body.clone(), read));
                }
                facts.push(f);
            }
        }
    }
    (facts, reads)
}

/// 目的：把转录事件还原成（对话, 每行历史长度, 每行回复号, 每行 id, 压缩点, 压缩行 id）。
/// 约束：重建必须与实时逐条一致（见 `reply_msgs` 与 session-model.md）。
pub fn replay_dialogue(
    events: &[serde_json::Value],
    mode: crate::units::llm::api::ToolMode,
) -> (
    Vec<crate::units::llm::api::Msg>,
    Vec<usize>,
    Vec<u64>,
    Vec<u64>,
    u64,
    u64,
) {
    // 先把转录行按顺序摊平：分组判断要看「下一行是不是 tool 行」。
    let mut rows: Vec<&serde_json::Value> = Vec::new();
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            rows.extend(lines.iter());
        }
    }
    // **压缩过**的会话按最后一条压缩行重建发送视图（见 session-model.md 六）：`up_to` 之前的行由摘要代替，
    // 转录本身完整保留；回档到压缩行之前时它已随转录被截掉，所以「没有它」就是「回到压缩前」。
    let compacted = crate::units::conversation::session::api::last_compaction(events);
    let compacted_upto = compacted.as_ref().map(|(up_to, _, _)| *up_to).unwrap_or(0);
    let compacted_line = compacted.as_ref().map(|(_, _, id)| *id).unwrap_or(0);
    // 对话里**只有**真正发生过的事；身份与环境由 params 现渲染。
    let mut history: Vec<Msg> = compacted
        .as_ref()
        .map(|(_, summary, _)| {
            crate::units::conversation::session::api::summary_message(summary)
        })
        .into_iter()
        .collect();
    let mut marks: Vec<usize> = Vec::new();
    let mut line_reply: Vec<u64> = Vec::new();
    let mut line_ids: Vec<u64> = Vec::new();
    let reply_of = |v: &serde_json::Value| v.get("reply").and_then(|x| x.as_u64()).unwrap_or(0);
    let id_of = |v: &serde_json::Value| v.get("id").and_then(|x| x.as_u64()).unwrap_or(0);
    // 回复级原文可能挂在该回复的正文行上（正文行随后被工具组跳过）：先接住再交给工具组。
    let mut pending_raw = String::new();
    let mut i = 0usize;
    while i < rows.len() {
        let l = rows[i];
        let id = id_of(l);
        // 被总结掉的行：不进对话，但仍占一行（marks / line_reply / line_ids 与转录行一一对应）。
        if compacted_upto > 0 && id < compacted_upto {
            if l.get("tool").is_some() {
                // 同一次回复的 tool 行连续同号：整组一起跳，别从中间切开。
                let reply = reply_of(l.get("tool").expect("已判存在"));
                while i < rows.len()
                    && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null)) == reply
                {
                    line_reply.push(reply);
                    line_ids.push(id_of(rows[i]));
                    marks.push(history.len().max(1));
                    i += 1;
                }
            } else {
                line_reply.push(reply_of(l));
                line_ids.push(id);
                marks.push(history.len().max(1));
                i += 1;
            }
            continue;
        }
        let line = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
        // **读结构化字段**（种类 / 系统标记 / 正文），不从正文里抠 [标签]。
        let kind = l.get("kind").and_then(|x| x.as_str()).unwrap_or("");
        // **压缩记录**只给人看：它不产生对话消息；摘要由最后一条记录行在顶部合成。
        if kind == "compacted" {
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        // **内容类系统提示**只给人看：不进对话，但仍占一行。
        } else if kind == "notice" {
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        // **被中断的半句**：留档给人看，但**不进对话**——「继续」是重开一轮，不是把这半句当发言续写。
        } else if l.get("cut").and_then(|x| x.as_bool()).unwrap_or(false) {
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        // 系统注入的行按它该有的角色还原：提醒/边界是 system，
        // **派发行**（`task`）是 user——否则重建出来的请求又变成一条 user 都没有，供应商照样拒收。
        } else if kind == "system" || l.get("system").and_then(|x| x.as_bool()).unwrap_or(false) {
            let is_task = l.get("task").and_then(|x| x.as_bool()).unwrap_or(false);
            history.push(if is_task {
                Msg::user(line.to_string())
            } else {
                Msg::system(line.to_string())
            });
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        } else if kind == "user" {
            // 用户说的行：正文就是用户那句话（[用户] / [用户:需求] 这类标签在字段里）。
            history.push(Msg::user(line.to_string()));
            // 用户行不属于任何回复：给它自己的行号，回档时才不会与相邻行误并成一组。
            line_reply.push(id);
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        } else if l.get("tool").is_some() {
            // 【回复分组 · 改动前务必读完】**同一次回复的 tool 行连续同号**（reply 由引擎给、
            // 落行时写入）；整组一起翻译成消息，靠的正是这个号——不靠"相邻行猜分组"。
            let reply = reply_of(l.get("tool").expect("已判存在"));
            let mut group: Vec<&serde_json::Value> = Vec::new();
            while i < rows.len()
                && reply_of(rows[i].get("tool").unwrap_or(&serde_json::Value::Null)) == reply
            {
                group.push(rows[i]);
                i += 1;
            }
            // 这一回复的助手消息原文（每条回复只落一份：可能在正文行上，也可能在组内首行）。
            let raw = if pending_raw.is_empty() {
                group
                    .iter()
                    .find_map(|t| {
                        t.get("raw")
                            .and_then(|x| x.as_str())
                            .filter(|s| !s.is_empty())
                    })
                    .unwrap_or_default()
                    .to_string()
            } else {
                std::mem::take(&mut pending_raw)
            };
            let views: Vec<crate::units::conversation::session::api::ToolCallView> = group
                .iter()
                .filter_map(|t| t.get("tool").cloned())
                .filter_map(|t| serde_json::from_value(t).ok())
                .collect();
            for m in crate::units::conversation::session::api::reply_msgs(mode, &raw, &views)
            {
                history.push(m);
            }
            for row in &group {
                line_reply.push(reply);
                line_ids.push(id_of(row));
                marks.push(history.len());
            }
        } else {
            // 文本行：它紧跟 tool 行时属于同一次回复（历史由那组 tool 行统一推进，这里不推）；
            // 否则这一行自己就是一条回复，推 assistant(该行文本)。
            let next_is_tool = rows
                .get(i + 1)
                .map(|n| n.get("tool").is_some())
                .unwrap_or(false);
            // 同一次回复的正文行可能带着回复级原文；工具组在下一轮消费它。
            if next_is_tool {
                pending_raw = l
                    .get("raw")
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
            } else {
                pending_raw.clear();
            }
            if !next_is_tool {
                // 正文本身就是内容（说话人/动词在字段里），直接进助手消息。
                history.push(Msg::assistant(line.to_string()));
            }
            line_reply.push(reply_of(l));
            line_ids.push(id);
            marks.push(history.len());
            i += 1;
        }
    }
    (
        history,
        marks,
        line_reply,
        line_ids,
        compacted_upto,
        compacted_line,
    )
}

/// 目的：代理会话的装配。
impl Turn {
    /// 目的：造一条代理会话（核心身份，私有沙箱，core 通道）。
    /// 参数：`registry` = 登记处（core 通道从这里解析）。
    pub fn build_proxy(
        &self,
        meta: &SessionMeta,
        registry: &dyn Registry,
    ) -> Result<AgentSession, String> {
        let channel = registry.core_channel()?;
        self.build_proxy_with(meta, &channel, registry)
    }

    /// 目的：用已解析的通道造代理会话：私有沙箱、权限面与工具面都按 meta 取。
    fn build_proxy_with(
        &self,
        meta: &SessionMeta,
        channel: &Channel,
        registry: &dyn Registry,
    ) -> Result<AgentSession, String> {
        self.workspace
            .prepare(&meta.name, &[PROXY_PRIVATE_DIR.to_string()])?;
        let roots = self
            .workspace
            .roots(&meta.name, &[PROXY_PRIVATE_DIR.to_string()])?;
        let private = roots
            .agents
            .get(PROXY_PRIVATE_DIR)
            .cloned()
            .ok_or_else(|| "代理会话没有私有沙箱路径".to_string())?;
        let sb = Sandbox {
            work_name: meta.name.clone(),
            session: meta.name.clone(),
            agent: SPEAKER.to_string(),
            shared: roots.shared.clone(),
            private,
            modules: BTreeMap::new(),
            modules_with_userdata: std::collections::BTreeSet::new(),
            permissions: self.permissions_for(registry, meta)?,
            scope: self.shared_scope_for(registry, meta)?,
            // 代理发言席自己住在宿主上（文件工具也跑在宿主），不需要 guest 翻译；
            // 但视图盘打开时仍要同一个视图写法，否则环境块渲染视图路径、寻址却按宿主根判越界。
            path_codec: crate::kernel::api::view_drive::view_codec(),
            filter_rules: roots.store.join(WORKIGNORE),
        };
        let tool_mode = registry.tool_mode(None);
        // 身份是**席位提示词**（core_proxy），不是某个 agent 的模块能力包：
        // 流程 / 环境 / 调用约定仍与 agent 身份同一份口径（见 workspace 的 render_system）。
        let mut params = SessionParams::from_workspace(SPEAKER, &sb, &[], "core_proxy");
        params.seat_system = Some(crate::units::prompt::api::Segment::ProxySystem);
        let member = crate::units::conversation::session::api::MemberSpec {
            modules: &[],
            sandbox: &sb,
            unavailable: BTreeMap::new(),
            mode: tool_mode,
            seat: "core_proxy",
            tier: crate::kernel::api::types::Tier::Host,
            guest_lease: String::new(),
            shell: self.shell_slot(&sb.session),
            env: self.member_env(
                registry,
                &[],
                &sb,
                crate::units::turn::env::api::network_allowed(registry.app()),
                crate::kernel::api::types::Tier::Host,
                &crate::units::catalog::packages::api::PackagePlan::default(),
            ),
        };
        let (window, percent, keep_percent) = self.context_budget(registry, None);
        let spec = crate::units::conversation::session::api::ProxySpec {
            params,
            member,
            channel: channel.clone(),
            context: (window, percent, keep_percent),
            model: registry.core_model(),
            freshness: meta.context_freshness,
            freshness_keep_percent: meta.freshness_keep_percent,
        };
        Ok(
            crate::units::conversation::session::api::proxy_session(
                &self.session_ports(),
                spec,
            ),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::replay_ledgers;

    fn line(reply: u64, path: &str) -> serde_json::Value {
        serde_json::json!({
            "type": "transcript",
            "lines": [{
                "tool": {
                    "speaker": "a",
                    "module": "",
                    "name": "read",
                    "ok": true,
                    "args": "{}",
                    "output": "",
                    "reply": reply,
                    "observations": [{
                        "schema": 1,
                        "reply": reply,
                        "path": path,
                        "worktree": "a",
                        "version": "v",
                        "coverage": "Full",
                        "total_lines": 1,
                        "source": "Read"
                    }]
                }
            }]
        })
    }

    /// 目的：重启回填只认压缩点之后的行——压缩掉的读既不授权写，也不再参与过期替换。
    #[test]
    fn replay_ledgers_drop_rows_before_the_compaction_point() {
        let events = vec![line(3, "old.md"), line(5, "live.md")];
        let (facts, reads) = replay_ledgers(&events, 4);
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].path, "live.md");
        assert_eq!(facts[0].reply, 5);
        assert_eq!(reads.len(), 1);
        assert_eq!(reads[0].0, 5);
        assert_eq!(reads[0].2.path, "live.md");
    }
}
