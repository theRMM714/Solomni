//! 协作会话状态机：建组 → 讨论 → 整理 → 执行 → 验收（拉模式）。
//! 前端经 Core 门面按 pending 驱动（set_task → confirm_slate? → begin → answer…），泵式收事件。
//! 依赖全部为端口与核心数据（Registry 只是数据快照）；无 IO，无具体适配器。

use crate::core::engine::{Discussion, Execution, Member, TurnOut, MAX_REWORK, MAX_ROUNDS};
use crate::core::envelope;
use crate::core::events::{CheckView, Pending, SessionEvent};
use crate::core::module::Module;
use crate::core::ports::{ChatGateway, ModuleSource, Msg};
use crate::core::prompt::Prompts;
use crate::core::providers::Registry;
use std::sync::Arc;

pub struct CollabSession {
    /// 是否委托代拟（ids == "?"）。
    delegated: bool,
    /// 名单（点名路径在 start 时填充；代拟路径在确认后填充）。
    picked: Vec<Module>,
    task: String,
    /// 代拟名单（id, 理由）。
    slate_picks: Vec<(String, String)>,
    /// 登记处快照：成员/核心通道的供应商解析在此进行（策略在 core）。
    registry: Registry,
    /// 当前用户介入请求。
    pub pending: Option<Pending>,
    allow: bool,
    disc: Option<Discussion>,
    /// 已发出的转录行数（增量事件用）。
    emitted: usize,
    core_chat: crate::core::ports::BoxedChat,
    core_is_demo: bool,
    prompts: Prompts,
    gateway: Arc<dyn ChatGateway + Send + Sync>,
    source: Arc<dyn ModuleSource + Send + Sync>,
    done: bool,
}

impl CollabSession {
    /// 装配会话：gateway 定通道（含核心通道与回落告知）；source 提供清单；registry 供解析。
    pub fn start(
        gateway: Arc<dyn ChatGateway + Send + Sync>,
        source: Arc<dyn ModuleSource + Send + Sync>,
        registry: &Registry,
        prompts: Prompts,
        ids: &str,
    ) -> Result<CollabSession, String> {
        let roster = source.scan();
        let delegated = ids.trim() == "?";
        let picked = if delegated {
            Vec::new()
        } else {
            let picked = pick_owned(&roster, ids);
            if picked.is_empty() {
                return Err("名单为空或无有效模块".to_string());
            }
            picked
        };
        let core_provider = registry.resolve(None, None).map(|(_, p)| p);
        let (core_chat, core_is_demo) = gateway.core_channel(core_provider);
        Ok(CollabSession {
            delegated,
            picked,
            task: String::new(),
            slate_picks: Vec::new(),
            registry: registry.clone(),
            pending: None,
            allow: false,
            disc: None,
            emitted: 0,
            core_chat,
            core_is_demo,
            prompts,
            gateway,
            source,
            done: false,
        })
    }

    /// 提交需求（总是第一步）。
    pub fn set_task(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        if task.trim().is_empty() {
            sink(SessionEvent::Notice("[取消] 需求为空".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        self.task = task.to_string();
        if self.delegated {
            self.draft_slate(sink);
        } else {
            let names: Vec<String> = self.picked.iter().map(|m| m.manifest.id.clone()).collect();
            sink(SessionEvent::Notice(format!("[建组] {}", names.join(" + "))));
            self.pending = Some(Pending::ConfirmBegin);
        }
    }

    /// 委托代拟：核心按模块简述与需求拟名单（附理由），交用户确认（选择权在用户）。
    fn draft_slate(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        let roster = self.source.scan();
        let listing = roster
            .modules
            .iter()
            .map(|m| format!("- {}：{}", m.manifest.id, m.manifest.brief))
            .collect::<Vec<_>>()
            .join("\n");
        let user = self.prompts.render(
            &self.prompts.core.slate.user,
            &[("modules", listing), ("task", self.task.clone())],
        );
        let msgs = vec![Msg::system(self.prompts.core.slate.system.clone()), Msg::user(user)];
        let raw = self.core_chat.complete(&msgs);
        let parsed = envelope::extract_json_object(&raw)
            .and_then(|obj| serde_json::from_str::<Slate>(&obj).ok());
        let Some(slate) = parsed else {
            sink(SessionEvent::Notice("[错误] 代拟失败（模型无响应格式）。请直接点名模块。".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        };
        // 只校验存在性（非法 id 拒收）；是否采纳由用户确认。
        let mut picks = Vec::new();
        for p in slate.picks {
            if roster.modules.iter().any(|m| m.manifest.id == p.id) {
                picks.push((p.id, p.why));
            } else {
                sink(SessionEvent::Notice(format!("[代拟] {} 不存在，拒收", p.id)));
            }
        }
        if picks.is_empty() {
            sink(SessionEvent::Notice("[错误] 代拟名单无有效模块".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        sink(SessionEvent::Transcript(vec![format!(
            "[代拟] {}",
            picks.iter().map(|(id, why)| format!("{}（{}）", id, why)).collect::<Vec<_>>().join("；")
        )]));
        self.slate_picks = picks;
        self.pending = Some(Pending::ConfirmSlate);
    }

    /// 回应代拟名单确认（仅 ConfirmSlate 挂起时有效）。
    pub fn confirm_slate(&mut self, ok: bool, sink: &mut dyn FnMut(SessionEvent)) {
        sink(SessionEvent::Transcript(vec![format!("[用户:名单] {}", if ok { "确认" } else { "取消" })]));
        if !ok {
            sink(SessionEvent::Notice("[取消] 已按用户意愿取消".into()));
            sink(SessionEvent::Ended);
            self.done = true;
            return;
        }
        let roster = self.source.scan();
        self.picked = self
            .slate_picks
            .iter()
            .filter_map(|(id, _)| roster.modules.iter().find(|m| &m.manifest.id == id).cloned())
            .collect();
        let names: Vec<String> = self.picked.iter().map(|m| m.manifest.id.clone()).collect();
        sink(SessionEvent::Notice(format!("[建组] {}", names.join(" + "))));
        self.pending = Some(Pending::ConfirmBegin);
    }

    /// 确认开始讨论（allow = yes,allow 自裁授权）；开聊并一路泵到暂停或交付。
    pub fn begin(&mut self, allow: bool, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_some() {
            return;
        }
        self.allow = allow;
        let prompts = self.prompts.clone();
        let mut members = Vec::new();
        for m in &self.picked {
            // 供应商解析（策略在 core）：模块选择 > 清单默认 > 全局默认；None = 回落演示。
            let provider = self
                .registry
                .resolve(m.selected_provider.as_deref(), m.manifest.model.provider.as_deref())
                .map(|(_, p)| p);
            let (chat, note) = self.gateway.member_channel(provider, &m.manifest.id);
            if let Some(n) = note {
                sink(SessionEvent::Notice(n));
            }
            members.push(Member::new(&m.manifest.id, m.system_block(&prompts), chat));
        }
        if self.core_is_demo {
            sink(SessionEvent::Notice("[提示] 核心未配置供应商：整理/验收使用内置假模型（演示）".into()));
        }
        let mut disc = Discussion::new(members, self.allow, prompts);
        disc.open(&self.task);
        self.disc = Some(disc);
        self.pump_with(sink);
    }

    /// 回答 ask（仅 Ask 挂起时有效）；回答转达后继续泵。
    pub fn answer(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        if matches!(self.pending, Some(Pending::Ask { .. })) {
            self.pending = None;
            if let Some(disc) = self.disc.as_mut() {
                disc.pending_user_answers.push(text.to_string());
            }
            self.pump_with(sink);
        }
    }

    /// 泵：推进讨论直至暂停（ask）或收敛并走完整理/执行/验收/交付。
    /// 泵：推进讨论直至暂停（ask）或收敛并走完整理/执行/验收/交付；事件逐条经 sink 外送。
    pub fn pump_with(&mut self, sink: &mut dyn FnMut(SessionEvent)) {
        if self.done || self.disc.is_none() {
            return;
        }
        let prompts = self.prompts.clone();
        // 讨论阶段：步进直到暂停或收敛。
        loop {
            let outcome = self.disc.as_mut().expect("disc 已确认存在").step();
            if let Some(d) = self.disc.as_ref() {
                push_delta(d, &mut self.emitted, sink);
            }
            match outcome {
                TurnOut::Round => {}
                TurnOut::AskUser { member, question } => {
                    self.pending = Some(Pending::Ask { member, question });
                    return;
                }
                TurnOut::Done => {
                    let round = self.disc.as_ref().expect("disc 存在").round;
                    let over_cap = round > MAX_ROUNDS;
                    if over_cap {
                        sink(SessionEvent::Notice("[上限] 讨论轮次超限，交用户裁决。".into()));
                    }
                    sink(SessionEvent::DiscussionDone { round, over_cap });
                    break;
                }
            }
        }
        // 整理（核心通道）。
        let plan = self.disc.as_ref().expect("disc 存在").synthesize(self.core_chat.as_mut());
        sink(SessionEvent::Plan(plan.clone()));
        // 执行 → 验收 → 返工（上限内）→ 交付。
        let members = self.disc.as_mut().expect("disc 存在").members.as_mut_slice();
        let mut exec = Execution::run(members, &plan, &prompts);
        for (id, text) in &exec.reports {
            sink(SessionEvent::Report { id: id.clone(), text: text.clone(), rework: 0 });
        }
        exec.review(self.core_chat.as_mut(), &plan, &prompts);
        sink(review_event(&exec));
        while !exec.all_pass() && exec.rework < MAX_REWORK {
            sink(SessionEvent::Notice(format!("[返工] 第 {} 次（上限 {}）", exec.rework + 1, MAX_REWORK)));
            let review_text = fail_text(&exec);
            let members = self.disc.as_mut().expect("disc 存在").members.as_mut_slice();
            exec.rerun(members, &plan, &review_text, &prompts);
            for (id, text) in &exec.reports {
                sink(SessionEvent::Report { id: id.clone(), text: text.clone(), rework: exec.rework });
            }
            exec.review(self.core_chat.as_mut(), &plan, &prompts);
            sink(review_event(&exec));
        }
        let ok = exec.all_pass();
        sink(SessionEvent::Delivery { ok, over_rework: !ok });
        sink(SessionEvent::Ended);
        self.done = true;
    }

    /// 会话是否已终结。
    pub fn is_done(&self) -> bool {
        self.done
    }
}

/// 发出自上次以来的新转录行（增量）。
fn push_delta(disc: &Discussion, emitted: &mut usize, sink: &mut dyn FnMut(SessionEvent)) {
    if disc.transcript.len() > *emitted {
        sink(SessionEvent::Transcript(disc.transcript[*emitted..].to_vec()));
        *emitted = disc.transcript.len();
    }
}

fn pick_owned(roster: &crate::core::module::Roster, ids: &str) -> Vec<Module> {
    ids.split(|c| c == ',' || c == '，')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id).cloned())
        .collect()
}

#[derive(serde::Deserialize)]
struct Slate {
    picks: Vec<Pick>,
}

#[derive(serde::Deserialize)]
struct Pick {
    id: String,
    why: String,
}

fn review_event(exec: &Execution) -> SessionEvent {
    let items = exec
        .items
        .iter()
        .map(|i| CheckView {
            item: i.item.clone(),
            status: i.status.clone(),
            note: i.reason.clone().or_else(|| i.evidence.clone()).unwrap_or_default(),
        })
        .collect();
    SessionEvent::Review { items, raw: exec.checklist_raw.clone() }
}

fn fail_text(exec: &Execution) -> String {
    exec.items
        .iter()
        .filter(|i| !i.status.eq_ignore_ascii_case("pass"))
        .map(|i| format!("- {}：{}", i.item, i.reason.clone().unwrap_or_default()))
        .collect::<Vec<_>>()
        .join("\n")
}