//! 核心层：三种模式会话状态机 + 通道网关 + 登记处操作端口。
//! 分层纪律：core 只依赖 model/module/providers/orchestrator/envelope，
//! 永不打印、永不读 stdin；前端（CLI/未来 Web）只见 Core 门面、SessionEvent 流与 pending。
//! 呈现即上下文：转录行与核心记录完全一致，前端按事件累积渲染。
//! 密钥只进登记处；前端只见供应商 id，永不接触密钥。

use crate::envelope;
use crate::model::{BoxedChat, FakeChat, HttpChat, Msg};
use crate::module::{self, Module};
use crate::orchestrator::{Discussion, Execution, Member, TurnOut, MAX_REWORK, MAX_ROUNDS};
use crate::providers::Registry;
use std::path::PathBuf;

// ---------- 事件流：核心吐事实，前端自行渲染 ----------

/// 会话事件：驱动前端渲染；转录行为增量，前端按序累积。
/// 预留字段说明：DiscussionDone 的 round/over_cap 供 Web 前端做裁决确认页（CLI 暂不渲染）。
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum SessionEvent {
    /// 状态提示（通道回落、建组、返工、上限等）。
    Notice(String),
    /// 转录新增行（呈现即上下文，行格式与核心记录一致）。
    Transcript(Vec<String>),
    /// 讨论收敛（over_cap = 轮次超限，需用户裁决）。
    DiscussionDone { round: usize, over_cap: bool },
    /// 整理方案就绪。
    Plan(String),
    /// 成员执行回报（rework = 第几轮执行，0 为首轮）。
    Report { id: String, text: String, rework: usize },
    /// 验收清单（raw = 解析失败时的原文）。
    Review { items: Vec<CheckView>, raw: String },
    /// 交付结论（over_rework = 返工超限交用户裁决）。
    Delivery { ok: bool, over_rework: bool },
    /// 会话结束。
    Ended,
}

/// 验收条目的呈现视图。
#[derive(Debug, Clone)]
pub struct CheckView {
    pub item: String,
    pub status: String,
    pub note: String,
}

/// 用户介入请求：会话暂停，等前端回应。
#[derive(Debug, Clone)]
pub enum Pending {
    /// 模块请教用户（yes,allow 自裁模式下不会出现）。
    Ask { member: String, question: String },
    /// 核心代拟名单待确认。
    ConfirmSlate,
    /// 名单已定，等用户确认开始讨论（可授权自裁）。
    ConfirmBegin,
}

// ---------- Core 门面：前端唯一入口 ----------

pub struct Core {
    root: PathBuf,
    registry: Registry,
}

impl Core {
    /// 装配核心：产品根目录（modules/ 与 .home/ 的锚点，相对路径基准）。
    pub fn open(root: PathBuf) -> Core {
        Core { registry: Registry::load(&root.join(".home").join("providers.yaml")), root }
    }

    /// 清单即事实：每次调用重扫 modules/。
    pub fn scan(&self) -> module::Roster {
        module::scan(&self.root.join("modules"))
    }

    // ---- 登记处操作端口（密钥在此层进出登记处，前端只见 id） ----

    pub fn provider_list(&self) -> Vec<String> {
        self.registry.display_lines()
    }

    /// 当前全局默认供应商 id（前端展示用，不含密钥）。
    pub fn provider_default(&self) -> Option<String> {
        self.registry.default.clone()
    }

    /// 新增/更新供应商；首入者自动成为默认。
    pub fn provider_upsert(&mut self, id: &str, base_url: &str, api_key: &str, models: &[String]) -> Result<(), String> {
        if id.is_empty() || base_url.is_empty() || api_key.is_empty() {
            return Err("id / base_url / api_key 均不能为空".to_string());
        }
        self.registry.providers.insert(
            id.to_string(),
            crate::providers::Provider {
                kind: "llm".to_string(),
                base_url: base_url.to_string(),
                api_key: api_key.to_string(),
                models: models.to_vec(),
            },
        );
        if self.registry.default.is_none() {
            self.registry.default = Some(id.to_string());
        }
        self.save_registry()
    }

    pub fn provider_remove(&mut self, id: &str) -> Result<bool, String> {
        let removed = self.registry.providers.remove(id).is_some();
        if removed && self.registry.default.as_deref() == Some(id) {
            self.registry.default = self.registry.providers.keys().next().cloned();
        }
        if removed {
            self.save_registry()?;
        }
        Ok(removed)
    }

    pub fn provider_set_default(&mut self, id: &str) -> Result<bool, String> {
        if !self.registry.providers.contains_key(id) {
            return Ok(false);
        }
        self.registry.default = Some(id.to_string());
        self.save_registry()?;
        Ok(true)
    }

    fn save_registry(&self) -> Result<(), String> {
        self.registry.save(&self.root.join(".home").join("providers.yaml"))
    }

    // ---- 会话工厂 ----

    /// 模式一：单模块直连（职责提示词并入历史首条）。
    pub fn start_direct(&self, id: &str) -> Result<DirectSession, String> {
        let m = self.find(id)?;
        let (chat, note) = self.make_chat(&m);
        Ok(DirectSession { id: id.to_string(), history: vec![Msg::system(m.system_block())], chat, note })
    }

    /// 模式三：全能（ids 空 = 全部模块；拼装职责提示词）。
    pub fn start_omni(&self, ids: &str) -> Result<OmniSession, String> {
        let roster = self.scan();
        let chosen = pick_borrowed(&roster, ids);
        if chosen.is_empty() {
            return Err("无可拼装模块".to_string());
        }
        let mut merged = String::from("你是全能助手，能力由以下模块职责拼装：\n");
        for m in &chosen {
            merged.push_str(&format!("\n== {} ==\n{}", m.manifest.id, m.manifest.system));
        }
        let (chat, note) = self.make_chat(chosen[0]);
        Ok(OmniSession { history: vec![Msg::system(merged)], chat, note })
    }

    /// 模式二：多模块协作（ids = 点名名单，或 "?" 委托代拟）。
    pub fn start_collab(&self, ids: &str) -> Result<CollabSession, String> {
        let roster = self.scan();
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
        Ok(CollabSession {
            delegated,
            picked,
            task: String::new(),
            slate_picks: Vec::new(),
            pending: None,
            allow: false,
            disc: None,
            emitted: 0,
            core_chat: self.core_chat(),
            core_is_demo: !self.has_channel(),
            done: false,
        })
    }

    // ---- 通道装配（回落策略集中于此；回落必须如实告知） ----

    fn find(&self, id: &str) -> Result<Module, String> {
        self.scan()
            .modules
            .into_iter()
            .find(|m| m.manifest.id == id)
            .ok_or_else(|| format!("无此模块：{}", id))
    }

    /// 模块当前选择 > 清单默认 > 全局默认；无通道回落假模型（演示模式，如实告知）。
    fn make_chat(&self, m: &Module) -> (BoxedChat, Option<String>) {
        match self.registry.resolve(m.selected_provider.as_deref(), m.manifest.model.provider.as_deref()) {
            Some((_, provider)) => {
                let model = provider.models.first().cloned().unwrap_or_else(|| "default".to_string());
                (Box::new(HttpChat { provider: provider.clone(), model }), None)
            }
            None => (
                Box::new(FakeChat::new(vec![FakeChat::say("（演示）收到。")])),
                Some(format!("{} 未配置供应商，使用内置假模型演示", m.manifest.id)),
            ),
        }
    }

    /// 核心自身通道（整理/验收/代拟）。
    fn core_chat(&self) -> BoxedChat {
        match self.registry.resolve(None, None) {
            Some((_, p)) => Box::new(HttpChat {
                provider: p.clone(),
                model: p.models.first().cloned().unwrap_or_else(|| "default".to_string()),
            }),
            None => Box::new(FakeChat::new(vec![])),
        }
    }

    fn has_channel(&self) -> bool {
        self.registry.resolve(None, None).is_some()
    }
}

fn pick_borrowed<'a>(roster: &'a module::Roster, ids: &str) -> Vec<&'a Module> {
    if ids.trim().is_empty() {
        return roster.modules.iter().collect();
    }
    ids.split(|c| c == ',' || c == '，')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id))
        .collect()
}

fn pick_owned(roster: &module::Roster, ids: &str) -> Vec<Module> {
    ids.split(|c| c == ',' || c == '，')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|id| roster.modules.iter().find(|m| m.manifest.id == id).cloned())
        .collect()
}

const CHAT_PROTOCOL: &str = "讨论约定：直接说事；需要用户决定时用 ask；不再参与用 leave；同意方案用 agree。";

// ---------- 模式一：直连会话 ----------

pub struct DirectSession {
    id: String,
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
}

impl DirectSession {
    /// 开场事件（通道回落告知）。
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    pub fn say(&mut self, text: &str) -> SessionEvent {
        self.history.push(Msg::user(text.to_string()));
        let raw = self.chat.complete(&self.history);
        let reply = envelope::parse(&raw);
        self.history.push(Msg::assistant(reply.text.clone()));
        SessionEvent::Transcript(vec![format!("[{}] {}", self.id, reply.text)])
    }
}

// ---------- 模式三：全能会话 ----------

pub struct OmniSession {
    history: Vec<Msg>,
    chat: BoxedChat,
    note: Option<String>,
}

impl OmniSession {
    pub fn open(&self) -> Vec<SessionEvent> {
        self.note.clone().map(|n| vec![SessionEvent::Notice(n)]).unwrap_or_default()
    }

    pub fn say(&mut self, text: &str) -> SessionEvent {
        self.history.push(Msg::user(text.to_string()));
        let raw = self.chat.complete(&self.history);
        let reply = envelope::parse(&raw);
        self.history.push(Msg::assistant(reply.text.clone()));
        SessionEvent::Transcript(vec![format!("[全能] {}", reply.text)])
    }
}

// ---------- 模式二：协作会话（建组 → 讨论 → 整理 → 执行 → 验收） ----------

/// 协作会话状态机：前端按 pending 驱动
/// （set_task → confirm_slate? → begin → pump/answer …），泵式收事件直到 Ended。
pub struct CollabSession {
    /// 是否委托代拟（ids == "?"）。
    delegated: bool,
    /// 名单（点名路径在 start 时填充；代拟路径在确认后填充）。
    picked: Vec<Module>,
    task: String,
    /// 代拟名单（id, 理由）。
    slate_picks: Vec<(String, String)>,
    /// 当前用户介入请求。
    pub pending: Option<Pending>,
    allow: bool,
    disc: Option<Discussion>,
    /// 已发出的转录行数（增量事件用）。
    emitted: usize,
    core_chat: BoxedChat,
    core_is_demo: bool,
    done: bool,
}

impl CollabSession {
    /// 提交需求（总是第一步）。
    pub fn set_task(&mut self, core: &Core, task: &str) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        if task.trim().is_empty() {
            events.push(SessionEvent::Notice("[取消] 需求为空".into()));
            events.push(SessionEvent::Ended);
            self.done = true;
            return events;
        }
        self.task = task.to_string();
        if self.delegated {
            self.draft_slate(core, &mut events);
        } else {
            let names: Vec<String> = self.picked.iter().map(|m| m.manifest.id.clone()).collect();
            events.push(SessionEvent::Notice(format!("[建组] {}", names.join(" + "))));
            self.pending = Some(Pending::ConfirmBegin);
        }
        events
    }

    /// 委托代拟：核心按模块简述与需求拟名单（附理由），交用户确认（选择权在用户）。
    fn draft_slate(&mut self, core: &Core, events: &mut Vec<SessionEvent>) {
        let roster = core.scan();
        let listing = roster
            .modules
            .iter()
            .map(|m| format!("- {}：{}", m.manifest.id, m.manifest.brief))
            .collect::<Vec<_>>()
            .join("\n");
        let raw = self.core_chat.complete(&[
            Msg::system("你是核心编排者。根据需求从模块简述中代拟建组名单。只输出 JSON：{\"picks\":[{\"id\":\"模块id\",\"why\":\"一句入选理由\"}]}。不选择不存在的模块。"),
            Msg::user(format!("== 模块简述 ==\n{}\n\n== 需求 ==\n{}", listing, self.task)),
        ]);
        let parsed = envelope::extract_json_object(&raw)
            .and_then(|obj| serde_json::from_str::<Slate>(&obj).ok());
        let Some(slate) = parsed else {
            events.push(SessionEvent::Notice("[错误] 代拟失败（模型无响应格式）。请直接点名模块。".into()));
            events.push(SessionEvent::Ended);
            self.done = true;
            return;
        };
        // 只校验存在性（非法 id 拒收）；是否采纳由用户确认。
        let mut picks = Vec::new();
        for p in slate.picks {
            if roster.modules.iter().any(|m| m.manifest.id == p.id) {
                picks.push((p.id, p.why));
            } else {
                events.push(SessionEvent::Notice(format!("[代拟] {} 不存在，拒收", p.id)));
            }
        }
        if picks.is_empty() {
            events.push(SessionEvent::Notice("[错误] 代拟名单无有效模块".into()));
            events.push(SessionEvent::Ended);
            self.done = true;
            return;
        }
        events.push(SessionEvent::Transcript(vec![format!(
            "[代拟] {}",
            picks.iter().map(|(id, why)| format!("{}（{}）", id, why)).collect::<Vec<_>>().join("；")
        )]));
        self.slate_picks = picks;
        self.pending = Some(Pending::ConfirmSlate);
    }

    /// 回应代拟名单确认（仅 ConfirmSlate 挂起时有效）。
    pub fn confirm_slate(&mut self, core: &Core, ok: bool) -> Vec<SessionEvent> {
        let mut events = vec![SessionEvent::Transcript(vec![format!("[用户:名单] {}", if ok { "确认" } else { "取消" })])];
        if !ok {
            events.push(SessionEvent::Notice("[取消] 已按用户意愿取消".into()));
            events.push(SessionEvent::Ended);
            self.done = true;
            return events;
        }
        let roster = core.scan();
        self.picked = self
            .slate_picks
            .iter()
            .filter_map(|(id, _)| roster.modules.iter().find(|m| &m.manifest.id == id).cloned())
            .collect();
        let names: Vec<String> = self.picked.iter().map(|m| m.manifest.id.clone()).collect();
        events.push(SessionEvent::Notice(format!("[建组] {}", names.join(" + "))));
        self.pending = Some(Pending::ConfirmBegin);
        events
    }

    /// 确认开始讨论（allow = yes,allow 自裁授权）；开聊并一路泵到暂停或交付。
    pub fn begin(&mut self, core: &Core, allow: bool) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        if self.done || self.disc.is_some() {
            return events;
        }
        self.allow = allow;
        let mut members = Vec::new();
        for m in &self.picked {
            let (chat, note) = core.make_chat(m);
            if let Some(n) = note {
                events.push(SessionEvent::Notice(n));
            }
            members.push(Member::new(&m.manifest.id, m.system_block(), chat));
        }
        if self.core_is_demo {
            events.push(SessionEvent::Notice("[提示] 核心未配置供应商：整理/验收使用内置假模型（演示）".into()));
        }
        let mut disc = Discussion::new(members, self.allow);
        disc.open(&self.task, CHAT_PROTOCOL);
        self.disc = Some(disc);
        if let Some(d) = self.disc.as_ref() {
            push_delta(d, &mut self.emitted, &mut events);
        }
        let mut rest = self.pump();
        events.append(&mut rest);
        events
    }

    /// 回答 ask（仅 Ask 挂起时有效）；回答转达后继续泵。
    pub fn answer(&mut self, text: &str) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        if matches!(self.pending, Some(Pending::Ask { .. })) {
            self.pending = None;
            if let Some(disc) = self.disc.as_mut() {
                disc.pending_user_answers.push(text.to_string());
            }
            let mut rest = self.pump();
            events.append(&mut rest);
        }
        events
    }

    /// 泵：推进讨论直至暂停（ask）或收敛并走完整理/执行/验收/交付。
    pub fn pump(&mut self) -> Vec<SessionEvent> {
        let mut out = Vec::new();
        if self.done || self.disc.is_none() {
            return out;
        }
        // 讨论阶段：步进直到暂停或收敛。
        loop {
            let outcome = self.disc.as_mut().expect("disc 在上方已确认").step();
            if let Some(d) = self.disc.as_ref() {
                push_delta(d, &mut self.emitted, &mut out);
            }
            match outcome {
                TurnOut::Round => {}
                TurnOut::AskUser { member, question } => {
                    self.pending = Some(Pending::Ask { member, question });
                    return out;
                }
                TurnOut::Done => {
                    let round = self.disc.as_ref().expect("disc 存在").round;
                    let over_cap = round > MAX_ROUNDS;
                    if over_cap {
                        out.push(SessionEvent::Notice("[上限] 讨论轮次超限，交用户裁决。".into()));
                    }
                    out.push(SessionEvent::DiscussionDone { round, over_cap });
                    break;
                }
            }
        }
        // 整理（核心通道）。
        let plan = {
            let disc = self.disc.as_ref().expect("disc 存在");
            disc.synthesize(self.core_chat.as_mut())
        };
        out.push(SessionEvent::Plan(plan.clone()));
        // 执行 → 验收 → 返工（上限内）→ 交付。
        let mut exec = Execution::run(self.disc.as_mut().expect("disc 存在").members.as_mut_slice(), &plan);
        for (id, text) in &exec.reports {
            out.push(SessionEvent::Report { id: id.clone(), text: text.clone(), rework: 0 });
        }
        exec.review(self.core_chat.as_mut(), &plan);
        out.push(review_event(&exec));
        while !exec.all_pass() && exec.rework < MAX_REWORK {
            out.push(SessionEvent::Notice(format!("[返工] 第 {} 次（上限 {}）", exec.rework + 1, MAX_REWORK)));
            let review_text = fail_text(&exec);
            let disc = self.disc.as_mut().expect("disc 存在");
            exec.rerun(disc.members.as_mut_slice(), &plan, &review_text);
            for (id, text) in &exec.reports {
                out.push(SessionEvent::Report { id: id.clone(), text: text.clone(), rework: exec.rework });
            }
            exec.review(self.core_chat.as_mut(), &plan);
            out.push(review_event(&exec));
        }
        let ok = exec.all_pass();
        out.push(SessionEvent::Delivery { ok, over_rework: !ok });
        out.push(SessionEvent::Ended);
        self.done = true;
        out
    }
}

/// 发出自上次以来的新转录行（增量）。
fn push_delta(disc: &Discussion, emitted: &mut usize, events: &mut Vec<SessionEvent>) {
    if disc.transcript.len() > *emitted {
        events.push(SessionEvent::Transcript(disc.transcript[*emitted..].to_vec()));
        *emitted = disc.transcript.len();
    }
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
