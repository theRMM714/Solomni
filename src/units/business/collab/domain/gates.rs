//! 目的：协作的裁决关——关卡种类、选项 id、载荷与卡片文案（**只属于协作**）。
//! 管：`Pending` 的形态，以及它到会话裁决通道那一问（`Ask`）的互转（挂卡时装材料、出队后还原成类型化的一关）。
//! 不管：排队、等待、落盘、重建（在会话单元）；谁在什么时候挂哪一关（在本单元的 `service/`）。
//! 联动：`service/` 用 `Pending::ask` 挂卡；`Pending::from_ask` 把排队回来的一问还原成类型化形态。

use crate::kernel::api::types::Ask;
use crate::units::business::transcript::api::OptionView;

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
            // 认不出的载荷一律不重建：**不猜、不硬凑一张卡出来**（工具层那一问也走这里，它等在工作线程上，
            // 重启后等待方已不存在，重建出来没人能做主）。
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

    /// 目的：这一问的信封与三段话（谁在问 + 消息）——推的事件与快照**只从这一处**派生。
    /// 参数：`advice` = 核心 AI 给的建议（随产生这一问的那次调用一起产出）。
    fn words(&self, advice: &str) -> (String, String, String, String, String) {
        match self {
            Pending::Ask { member, question } => (
                "member".to_string(),
                member.clone(),
                format!("{} 在等你回话", member),
                "你说的话会进主会话，所有成员都看得到。".to_string(),
                question.clone(),
            ),
            Pending::ConfirmSlate => (
                "core".to_string(),
                "核心".to_string(),
                "是否按此建组？".to_string(),
                "核心已代拟名单（见转录）。".to_string(),
                advice.to_string(),
            ),
            Pending::ConfirmBegin => (
                "core".to_string(),
                "核心".to_string(),
                "现在开始讨论？".to_string(),
                "名单已定。".to_string(),
                advice.to_string(),
            ),
            Pending::PlanReview => (
                "core".to_string(),
                "核心".to_string(),
                "要不要现在开工？".to_string(),
                "方案与任务链已备好；按规则不自动开工。".to_string(),
                advice.to_string(),
            ),
            Pending::NodeBlocked { nodes } => (
                "core".to_string(),
                "核心".to_string(),
                "有节点没过验收，要不要重派？".to_string(),
                format!("没过验收的节点：{}。", nodes.join("、")),
                advice.to_string(),
            ),
        }
    }

    /// 目的：把这一关装成会话裁决通道上的一问（信封 + 三段话 + 选项 + 机制载荷 + 附言规则）。
    /// 参数：`advice` = 核心 AI 给的建议（随产生这一问的那次调用一起产出）。
    /// 约束：advice 进载荷（重建时要它）；附言规则只从 `note_requirement` 派生，不另写一份。
    pub fn ask(&self, advice: &str) -> Ask {
        let mut payload = self.payload();
        if let Some(o) = payload.as_object_mut() {
            o.insert("advice".to_string(), serde_json::json!(advice));
        }
        let options = self.options();
        let notes = options
            .iter()
            .filter_map(|o| self.note_requirement(&o.id).map(|why| (o.id.clone(), why)))
            .collect();
        let (role, name, title, body, detail) = self.words(advice);
        Ask {
            role,
            name,
            title,
            body,
            detail,
            options: options.into_iter().map(|o| (o.id, o.label)).collect(),
            kind: self.kind().to_string(),
            payload,
            notes,
        }
    }

    /// 目的：从排队回来的一问还原成类型化形态（出队处置按它分派）。
    /// 返回：认得的就是这一关与它那条建议；认不出就是 None。
    pub fn from_ask(ask: &Ask) -> Option<(Pending, String)> {
        let p = Pending::from_payload(&ask.kind, &ask.payload)?;
        let advice = ask
            .payload
            .get("advice")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Some((p, advice))
    }
}
