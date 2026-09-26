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
    pub chain: crate::core::chain::TaskChain,
    pub reports: BTreeMap<String, String>,
    pub review_raw: Option<String>,
    pub review_pass: bool,
    pub rework: usize,
    pub delivery: Option<bool>,
    pub ended: bool,
    /// 未回答的请教（member, question）。
    pub pending_ask: Option<(String, String)>,
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
                            // 普通用户发言：未答的请教作废。
                            _ => st.pending_ask = None,
                        }
                    } else if speaker == "代拟" {
                        // 代拟行只给人看；名单的权威来源是 meta.agents（确认后写回）。
                        st.slate = Some(text.to_string());
                    } else if speaker == "core" {
                        st.pending_ask = None;
                    } else if !speaker.is_empty() {
                        match verb {
                            "agree" => {
                                st.agreed.insert(speaker.to_string(), true);
                            }
                            "leave" => {
                                st.present.insert(speaker.to_string(), false);
                            }
                            "ask" => st.pending_ask = Some((speaker.to_string(), text.to_string())),
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
                    if let Ok(parsed) =
                        serde_json::from_value::<crate::core::chain::TaskChain>(chain.clone())
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
