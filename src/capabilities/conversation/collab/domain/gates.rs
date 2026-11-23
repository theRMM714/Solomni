//! 目的：协作的裁决关——关卡种类、选项 id、载荷与卡片文案（**只属于协作**）。
//! 管：`Pending` 的形态，以及它到会话裁决通道 `Gate` 的互转（挂卡时装材料、出队后还原成类型化的一关）。
//! 不管：排队、等待、落盘、重建（在会话能力）；谁在什么时候挂哪一关（在本能力的 `service/`）。
//! 联动：`service/` 用 `Pending::gate` 挂卡；`Pending::from_gate` 把排队回来的一关还原成类型化形态。

use crate::capabilities::conversation::session::api::{
    DecisionCard, DecisionEnvelope, DecisionMessage, Gate, OptionView,
};

/// 目的：协作的一关：会话暂停，等前端回应。
#[derive(Debug, Clone)]
pub enum Pending {
    /// 模块请教用户（yes,allow 自裁模式下不会出现）。
    Ask { member: String, question: String },
    /// 核心代拟名单待确认。
    ConfirmSlate,
    /// 名单已定，等用户确认开始讨论（可授权自裁）。
    ConfirmBegin,
    /// 方案待审：整理完**不自动开工**，等用户点「同意」（见 docs/collab/task-chain.md）。
    PlanReview,
    /// 节点验收没过：等用户点「继续」重派这些节点。
    NodeBlocked { nodes: Vec<String> },
    /// **工具层自己发起的一类**（围栏这一环装不上、今后任何底层 yes/no）：消息与选项**由发起方给**，
    /// 通道不解释内容。与工具级确认同属**等在工作线程上**的那一类——发起方阻塞等回答，不点不继续。
    /// 装箱：它比其它变体大得多，而这一关本来就只挂几条（不值得让整队每个格子都变宽）。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    ToolAsk(Box<crate::kernel::api::types::Ask>),
}

/// 目的：请教这一关的选项 id——附一句回话（id 是契约，改文案不改行为）。
pub const OPT_ASK_REPLY: &str = "ask_reply";
/// 目的：代拟名单这一关的选项 id——按此建组。
pub const OPT_SLATE_CONFIRM: &str = "slate_confirm";
/// 目的：代拟名单这一关的选项 id——取消这次工作。
pub const OPT_SLATE_CANCEL: &str = "slate_cancel";
/// 目的：开始讨论这一关的选项 id——开始。
pub const OPT_BEGIN: &str = "begin";
/// 目的：开始讨论这一关的选项 id——开始，并授权小组自裁细节。
pub const OPT_BEGIN_ALLOW: &str = "begin_allow";
/// 目的：方案待审这一关的选项 id——开工。
pub const OPT_PLAN_START: &str = "plan_start";
/// 目的：方案待审这一关的选项 id——先说一句，由核心 AI 判这句话够不够明确。
pub const OPT_PLAN_SAY: &str = "plan_say";
/// 目的：节点没过这一关的选项 id——重派没过的那些节点。
pub const OPT_NODE_REWORK: &str = "node_rework";
/// 目的：节点没过这一关的选项 id——先说一句，由核心 AI 判这句话够不够明确。
pub const OPT_NODE_SAY: &str = "node_say";

impl Pending {
    /// 目的：这一关的机制名（落盘与重建按它认门）。
    pub fn kind(&self) -> &'static str {
        match self {
            Pending::Ask { .. } => "ask",
            Pending::ConfirmSlate => "confirm_slate",
            Pending::ConfirmBegin => "confirm_begin",
            Pending::PlanReview => "plan_review",
            Pending::NodeBlocked { .. } => "node_blocked",
            // 工具层自己发起的那一类（围栏这类）：机制名固定，重建材料就是那条问题本身。
            Pending::ToolAsk(_) => "tool_ask",
        }
    }

    /// 目的：这一关的**机制载荷**（重启后重建这一关的材料：谁在问、问的什么、哪些节点没过）。
    /// 约束：它是机制自己的材料，不是渲染内容——呈现层不看它（见 session-model.md「请用户裁决」）。
    pub fn payload(&self) -> serde_json::Value {
        match self {
            Pending::Ask { member, question } => {
                serde_json::json!({ "member": member, "question": question })
            }
            Pending::NodeBlocked { nodes } => serde_json::json!({ "nodes": nodes }),
            // 工具层那一类：重建材料就是那条问题（谁在问 + 消息 + 选项集）。
            Pending::ToolAsk(ask) => serde_json::to_value(ask).unwrap_or(serde_json::Value::Null),
            _ => serde_json::json!({}),
        }
    }

    /// 目的：从落盘的机制名与载荷重建这一关（重启后按转录重建挂起，已答过的不再挂）。
    /// 返回：认得出就是这一关；认不出就是 None——不猜、不硬凑一张卡出来。
    pub fn from_payload(gate: &str, payload: &serde_json::Value) -> Option<Pending> {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        Some(match gate {
            "ask" => Pending::Ask {
                member: s("member"),
                question: s("question"),
            },
            "confirm_slate" => Pending::ConfirmSlate,
            "confirm_begin" => Pending::ConfirmBegin,
            "plan_review" => Pending::PlanReview,
            "node_blocked" => Pending::NodeBlocked {
                nodes: payload
                    .get("nodes")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter()
                            .filter_map(|x| x.as_str().map(|s| s.to_string()))
                            .collect()
                    })
                    .unwrap_or_default(),
            },
            // **工具层自己发起的那一类不重建**：它等在工作线程上，
            // 重启后那个等待方已经不存在——重建出来的卡没人能做主，等于拿一张答不了卡糊用户。
            _ => return None,
        })
    }

    /// 目的：这一关的选项集（有序）：id 是行为契约，label 只给渲染。
    /// 约束：每一条都必须是真能执行的——通道里没有"置灰"这一说（见 session-model.md「请用户裁决」）。
    pub fn options(&self) -> Vec<OptionView> {
        let o = |id: &str, label: &str| OptionView {
            id: id.to_string(),
            label: label.to_string(),
        };
        match self {
            Pending::Ask { .. } => vec![o(OPT_ASK_REPLY, "回话")],
            Pending::ConfirmSlate => vec![
                o(OPT_SLATE_CONFIRM, "确认建组"),
                o(OPT_SLATE_CANCEL, "取消"),
            ],
            Pending::ConfirmBegin => vec![
                o(OPT_BEGIN, "开始"),
                o(OPT_BEGIN_ALLOW, "开始（授权小组自裁细节）"),
            ],
            Pending::PlanReview => vec![
                o(OPT_PLAN_START, "开工"),
                o(OPT_PLAN_SAY, "先说一句（核心判明确性）"),
            ],
            Pending::NodeBlocked { .. } => vec![
                o(OPT_NODE_REWORK, "重派没过的节点"),
                o(OPT_NODE_SAY, "先说一句（核心判明确性）"),
            ],
            // 工具层那一类：选项集**由发起方给**（它才知道每条选项真能不能执行），通道原样渲染。
            Pending::ToolAsk(ask) => ask.options.iter().map(|(id, label)| o(id, label)).collect(),
        }
    }

    /// 目的：这一关的某个选项**要不要附言**：要就给出"为什么"的原文（队列在出队前按它校验）。
    /// 约束：判据属于发起方（这一关自己的规则）；校验只此一处——被拒的回答不出队、不落回答。
    pub fn note_requirement(&self, option: &str) -> Option<String> {
        match (self, option) {
            (Pending::Ask { .. }, OPT_ASK_REPLY) => {
                Some("这一关要附一句回话：把要说的话写在附言里。".to_string())
            }
            (Pending::PlanReview, OPT_PLAN_SAY) | (Pending::NodeBlocked { .. }, OPT_NODE_SAY) => {
                Some("这一关要附一句你的想法：把要说的话写在附言里。".to_string())
            }
            _ => None,
        }
    }

    /// 目的：这一关的卡片（id + 信封 + 消息 + 选项）——推的事件与快照**只从这一处**派生。
    /// 参数：id = 会话内唯一的卡号；advice = 核心 AI 给的建议（随产生这一关的那次调用一起产出）。
    pub fn card(&self, id: &str, advice: &str) -> DecisionCard {
        let (role, name, title, body, detail): (&str, String, String, String, String) = match self {
            Pending::Ask { member, question } => (
                "member",
                member.clone(),
                format!("{} 在等你回话", member),
                "你说的话会进主会话，所有成员都看得到。".to_string(),
                question.clone(),
            ),
            Pending::ConfirmSlate => (
                "core",
                "核心".to_string(),
                "是否按此建组？".to_string(),
                "核心已代拟名单（见转录）。".to_string(),
                advice.to_string(),
            ),
            Pending::ConfirmBegin => (
                "core",
                "核心".to_string(),
                "现在开始讨论？".to_string(),
                "名单已定。".to_string(),
                advice.to_string(),
            ),
            Pending::PlanReview => (
                "core",
                "核心".to_string(),
                "要不要现在开工？".to_string(),
                "方案与任务链已备好；按规则不自动开工。".to_string(),
                advice.to_string(),
            ),
            Pending::NodeBlocked { nodes } => (
                "core",
                "核心".to_string(),
                "有节点没过验收，要不要重派？".to_string(),
                format!("没过验收的节点：{}。", nodes.join("、")),
                advice.to_string(),
            ),
            // 工具层那一类：**消息与选项原样来自发起方**——通道不解释内容，只把三段话与选项渲染出来。
            Pending::ToolAsk(ask) => (
                ask.role.as_str(),
                ask.name.clone(),
                ask.title.clone(),
                ask.body.clone(),
                ask.detail.clone(),
            ),
        };
        DecisionCard {
            id: id.to_string(),
            envelope: DecisionEnvelope {
                role: role.to_string(),
                name,
            },
            message: DecisionMessage {
                title,
                body,
                detail,
            },
            options: self.options(),
        }
    }

    /// 目的：把这一关装成会话裁决通道的一关（卡片 + 机制载荷 + 附言规则）。
    /// 参数：id = 会话内唯一的卡号；advice = 核心 AI 给的建议（随产生这一关的那次调用一起产出）。
    /// 约束：advice 进载荷（重建时要它）；附言规则只从 `note_requirement` 派生，不另写一份。
    pub fn gate(&self, id: &str, advice: &str) -> Gate {
        let mut payload = self.payload();
        if let Some(o) = payload.as_object_mut() {
            o.insert("advice".to_string(), serde_json::json!(advice));
        }
        let notes = self
            .options()
            .into_iter()
            .filter_map(|o| self.note_requirement(&o.id).map(|why| (o.id, why)))
            .collect();
        Gate {
            kind: self.kind().to_string(),
            payload,
            card: self.card(id, advice),
            notes,
        }
    }

    /// 目的：从排队回来的一关还原成类型化形态（出队处置按它分派）。
    /// 返回：认得的就是这一关与它那条建议；认不出就是 None（例如工具层那一类不重建）。
    pub fn from_gate(gate: &Gate) -> Option<(Pending, String)> {
        let p = Pending::from_payload(&gate.kind, &gate.payload)?;
        let advice = gate
            .payload
            .get("advice")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some((p, advice))
    }
}
