//! 协作会话的「转录即状态」派生：从会话事件流水纯函数地算出当前状态。
//! 核心只据此判定「当前该走哪一步、断在哪就继续」；不保留任何影子状态。
//! 只依赖事件里的文本锚点，不依赖任何内存对象，因此可回放、可单测。

use std::collections::BTreeMap;

/// 从事件流水派生出的协作状态。
#[derive(Debug, Default, Clone)]
pub struct CollabState {
    /// 需求（最后一条 [用户:需求]）。
    pub task: Option<String>,
    /// 代拟名单原文（最后一条 [代拟]）。
    pub slate: Option<String>,
    pub slate_confirmed: bool,
    /// 用户已授权开始。
    pub begun: bool,
    /// 开始即授权小组自裁（yes,allow）。
    pub allow: bool,
    /// 实际在组名单（agent 实例名；来自会话 meta 的 agents）。
    pub picked: Vec<String>,
    pub round: usize,
    /// 留组状态（leave 后为 false）。
    pub present: BTreeMap<String, bool>,
    /// 本轮表态：agree 置 true，[轮次] 与开始会重置。
    pub agreed: BTreeMap<String, bool>,
    /// 讨论已收敛或超限（discussion_done）。
    pub closed: bool,
    /// 用户在审查关卡点过「同意」（[用户:同意方案]）：方案过关，可以开工。
    pub plan_approved: bool,
    pub plan: Option<String>,
    /// 核心给出的任务链（从 plan_review 事件派生）。
    pub chain: crate::capabilities::taskchain::api::TaskChain,
    pub reports: BTreeMap<String, String>,
    pub review_raw: Option<String>,
    pub review_pass: bool,
    pub rework: usize,
    pub delivery: Option<bool>,
    pub ended: bool,
    /// 目的：转录里**还没人回答、也还没作废**的裁决卡（按先来后到，队首在最前）：整队重建的唯一来源。
    pub open_gates: Vec<(String, String, serde_json::Value)>,
    /// 目的：已经发过的卡号（含已答、已作废的）：卡号计数与"同一张重推"的去重都认它。
    pub issued_cards: std::collections::BTreeSet<String>,
    /// 目的：已发出的裁决卡数（卡号计数器；重启后从转录续号，卡号因此稳定）。
    pub cards: u64,
    /// 工具执行次数（回档警告用）。
    pub tool_runs: usize,
}

impl CollabState {
    fn report_rework(&mut self, id: String, text: String, rework: usize) {
        self.rework = self.rework.max(rework);
        self.reports.insert(id, text);
    }
}

/// 从事件流水派生协作状态：纯函数，可单测；回放可复现。
/// roster_names = 会话 meta 里的 agent 实例名（顺序即名单，权威来源）；其余状态由转录文本锚点派生。
/// 人名即发言席：present/agreed/reports/pending_ask 的键都是 agent 名。
pub fn derive(events: &[serde_json::Value], roster_names: &[String]) -> CollabState {
    let mut st = CollabState::default();
    if !roster_names.is_empty() {
        st.picked = roster_names.to_vec();
    }
    let mut in_discussion = false;
    reset_agreed(&mut st);
    let picked = st.picked.clone();
    for id in picked {
        st.present.insert(id, true);
    }

    for ev in events {
        let kind = ev.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "transcript" => {
                let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
                    continue;
                };
                for l in lines {
                    // **读结构化字段**（说话人 / 动词 / 种类 / 正文），不从正文里抠标签。
                    let speaker = l.get("speaker").and_then(|x| x.as_str()).unwrap_or("");
                    let verb = l.get("verb").and_then(|x| x.as_str()).unwrap_or("");
                    let kind = l.get("kind").and_then(|x| x.as_str()).unwrap_or("");
                    let text = l.get("line").and_then(|x| x.as_str()).unwrap_or("");
                    if kind == "round" {
                        st.round = text.parse().unwrap_or(st.round);
                        st.closed = false;
                        // 轮次边界**不清空同意**：同意是粘住的（与 Discussion::step 同一口径），
                        // 否则重建出来的"谁已同意"会与实时不一致。
                    } else if speaker == "用户" {
                        match verb {
                            "需求" => st.task = Some(text.to_string()),
                            "撤回" => {
                                st.agreed.insert(text.trim().to_string(), false);
                            }
                            "名单" => st.slate_confirmed = text.starts_with("确认"),
                            "同意方案" => st.plan_approved = true,
                            "开始" => {
                                st.begun = true;
                                st.allow = text.contains("allow");
                                st.round = 1;
                                in_discussion = true;
                                st.closed = false;
                                reset_agreed(&mut st);
                            }
                            // 普通用户发言：不牵动门（门由卡与回答认，见上）。
                            _ => {}
                        }
                    } else if speaker == "代拟" {
                        // 代拟行只给人看；名单的权威来源是 meta.agents（确认后写回）。
                        st.slate = Some(text.to_string());
                    } else if speaker == "core" {
                        // 核心自己的行不进表态统计（同意 / 离开只认成员席）。
                    } else if !speaker.is_empty() {
                        match verb {
                            "agree" => {
                                st.agreed.insert(speaker.to_string(), true);
                            }
                            "leave" => {
                                st.present.insert(speaker.to_string(), false);
                            }
                            _ => {}
                        }
                    }
                }
            }
            "discussion_done" => st.closed = true,
            "plan" => {
                st.plan = ev
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string())
            }
            // 任务链随"方案待审"事件落档：重启/回档后按它重建，**不重新整理**（省一次模型调用）。
            "plan_review" => {
                if let Some(chain) = ev.get("chain") {
                    if let Ok(parsed) = serde_json::from_value::<
                        crate::capabilities::taskchain::api::TaskChain,
                    >(chain.clone())
                    {
                        st.chain = parsed;
                    }
                }
            }
            "report" => {
                let id = ev
                    .get("id")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let text = ev
                    .get("text")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let rework = ev.get("rework").and_then(|t| t.as_u64()).unwrap_or(0) as usize;
                st.report_rework(id, text, rework);
            }
            "review" => {
                let items = ev
                    .get("items")
                    .and_then(|t| t.as_array())
                    .cloned()
                    .unwrap_or_default();
                st.review_pass = !items.is_empty()
                    && items.iter().all(|i| {
                        i.get("status")
                            .and_then(|s| s.as_str())
                            .map(|s| s.eq_ignore_ascii_case("pass"))
                            .unwrap_or(false)
                    });
                st.review_raw = ev
                    .get("raw")
                    .and_then(|t| t.as_str())
                    .map(|s| s.to_string());
            }
            // 裁决卡、回答与作废：队列只认这一族（答过的不重问、作废的不再挂，见 session-model.md「请用户裁决」）。
            "decision_card" => {
                let id = ev
                    .get("id")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let gate = ev
                    .get("gate")
                    .and_then(|t| t.as_str())
                    .unwrap_or("")
                    .to_string();
                let payload = ev.get("payload").cloned().unwrap_or(serde_json::json!({}));
                upsert_card(&mut st, id, gate, payload);
                // 队首那条事件还带着**排在它后面的那几张**（谁在等 + 各自的重建材料）：
                // 整队因此只凭转录就能重建，而不是只剩队首一张。
                if let Some(list) = ev.get("waiting").and_then(|w| w.as_array()) {
                    for w in list {
                        let wid = w
                            .get("id")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let wgate = w
                            .get("gate")
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string();
                        let wpayload = w.get("payload").cloned().unwrap_or(serde_json::json!({}));
                        if wid.is_empty() || wgate.is_empty() {
                            continue;
                        }
                        upsert_card(&mut st, wid, wgate, wpayload);
                    }
                }
            }
            "decision_answer" => {
                let card = ev.get("card").and_then(|t| t.as_str()).unwrap_or("");
                st.open_gates.retain(|(g, _, _)| g != card);
            }
            "decision_void" => {
                let cards: Vec<String> = ev
                    .get("cards")
                    .and_then(|c| c.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                st.open_gates.retain(|(g, _, _)| !cards.contains(g));
            }
            "delivery" => st.delivery = ev.get("ok").and_then(|t| t.as_bool()),
            "ended" => st.ended = true,
            _ => {}
        }
    }
    // 工具执行次数 = 带 tool 字段的转录行数（回档警告用；行即事实）。
    st.tool_runs = tool_runs(events);
    if in_discussion && !st.closed {
        let picked = st.picked.clone();
        let all = !picked.is_empty()
            && picked.iter().all(|id| {
                st.present.get(id).copied().unwrap_or(true)
                    && st.agreed.get(id).copied().unwrap_or(false)
            });
        if all {
            st.closed = true;
        }
    }
    st
}

/// 目的：把一张卡的机制材料并入队列——认得出（同卡号）就就地更新，新的追加到队尾。
/// 约束：卡号计数只认**没见过的卡号**——队列形态变了会重推同一张，那不算新卡。
fn upsert_card(st: &mut CollabState, id: String, gate: String, payload: serde_json::Value) {
    if st.open_gates.iter().any(|(g, _, _)| *g == id) {
        if let Some(slot) = st.open_gates.iter_mut().find(|(g, _, _)| *g == id) {
            *slot = (id, gate, payload);
        }
        return;
    }
    if st.issued_cards.insert(id.clone()) {
        st.cards += 1;
    }
    st.open_gates.push((id, gate, payload));
}

/// 统计事件流水里的工具执行次数：数带 tool 字段的转录行（回档警告用它，行即事实）。
pub fn tool_runs(events: &[serde_json::Value]) -> usize {
    let mut n = 0;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        if let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) {
            n += lines.iter().filter(|l| l.get("tool").is_some()).count();
        }
    }
    n
}

/// 把本轮表态全部重置为未表态。
fn reset_agreed(st: &mut CollabState) {
    let ids = st.picked.clone();
    for id in ids {
        st.agreed.insert(id, false);
    }
}
