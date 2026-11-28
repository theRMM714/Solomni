//! 目的：回合状态与裁决机制的入站面——别的单元与呈现层只准用这里（不许碰 `domain`）。
//! 管：在世会话与回合状态（`Turn` / `Session`）、裁决队端口、流程插件契约与工厂、提问桥、
//!   落盘与队列两个端口面（`History` / `HistoryOps`）。
//! 不管：`domain` 的内部实现、回合驱动与生成（在 `turn`）、落盘实现（在 `conversation::session`）；
//!   装配一席的端口参数对象随装配机留在 `agentloop`（`SessionPorts`），这里不再转出。
//! 联动：`turn` 的驱动与生成线程按 `Flow` 推进任何流程；`collab` 实现 `Flow`；`runtime` 持有 `Turn`。

use std::sync::Arc;

use crate::kernel::ports::AskUser;
use crate::units::business::session_model::api::{AskDesk, HistoryView, SessionMeta};
use crate::units::business::transcript::api::{Content, SessionEvent};

#[allow(unused_imports)]
pub use crate::units::business::turn_state::domain::decisions::{
    Answered, Decisions, GateTicket, Gates, SlotWake, decisions, issued_in,
};
// 流程面：驱动只认这几个名字（推一步 / 让出一回合 / 交回后的处置 / 可派的活）。
pub use crate::units::business::turn_state::domain::flow::{
    Flow, Handback, Identity, MemberAsk, Settle, WorkPlan,
};
// 流程工厂面：组合根把实现注入 turn / work，装配点只按 id 取段。
pub use crate::units::business::turn_state::domain::flows::{FlowSetup, Flows, sequence_of};
// 流程实现注册面：组合根登记实现，装配点只按段声明查表，不认识任何具体流程。
pub use crate::units::business::turn_state::domain::builder::{
    BuildRequest, FlowBuilder, FlowBuilders,
};
/// 目的：在世会话形态（宿主与测试只经它取用）。
///   回合状态本身（`TurnState`）是内部结构：它只被 `Turn` 持有，不出单元。
pub use crate::units::business::turn_state::domain::live::Session;
/// 目的：复合流程：装配期把若干段流程串成一条。
#[allow(unused_imports)]
pub use crate::units::business::turn_state::domain::sequence::Sequence;
/// 目的：回合状态本体（在世表 + 运行态 + 装配一席要用的入站面）的出口。
pub use crate::units::business::turn_state::domain::turn::Turn;

/// 目的：把这条会话的**裁决通道收窄成流程段的发起面**：挂起一张卡 + 问"有没有人在等"。
/// 参数：`door` = 该会话的裁决通道（与工具层共用同一条）。
pub fn desk_face(door: Arc<dyn Gates>) -> Arc<dyn AskDesk> {
    crate::units::business::turn_state::domain::ask::desk_face(door)
}

/// 目的：造这个会话的**工具层提问端口**（登记一张卡、外送并落盘、停会话都走同一会话那条道）。
/// 参数：`door` = 该会话的裁决通道；`sid` = 会话名；`emit` = 推一条事件；`stop` = 停这个会话。
/// 返回：可跨线程用的提问端口。
pub fn session_ask(
    door: Arc<dyn Gates>,
    sid: &str,
    emit: Box<dyn Fn(SessionEvent) + Send + Sync>,
    stop: Box<dyn Fn(&str) + Send + Sync>,
) -> Arc<dyn AskUser> {
    Arc::new(
        crate::units::business::turn_state::domain::ask::SessionAsk::new(door, sid, emit, stop),
    )
}

/// 目的：落盘会话的**队列面**：呈现层经 runtime 的队列代理调它（列表 / 打开 / 删除）。
///   "在世会话 × 历史的并集"（`SessionView`）**不在本面里**：那要同时认识会话中心与历史，
///   归会话中心（`runtime::api::WorkOps::session_views`）。
pub trait HistoryOps: Send + Sync {
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    fn open(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String>;
    fn delete(&self, name: &str) -> Result<bool, String>;
}

/// 目的：会话的**直连面**（`conversation::session` 的 `service.rs` 实现）：别的单元要造会话、追流水、
///   读元信息、删会话，走这里；出站端口 `HistoryStore`（目录布局与 append-only 文件格式）**只由它持有**（R12）。
///   它与 `HistoryOps` 的分工是**接收者不同**，不是重复：`HistoryOps` 由队列代理（`Handle`）实现、
///   面向呈现层；`History` 由本单元自己实现、面向别的单元。这里的四个写操作（create / save_meta /
///   append / delete）**不开放给呈现层**——呈现层要写就经协调业务的用例。
pub trait History: Send + Sync {
    /// 建一个会话目录（meta + 空流水）。
    fn create(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 写回会话元信息（配置界面的编辑：会话身份的**唯一真相**在 meta.yaml）。
    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String>;
    /// 追加若干条目（留档只走这条；删除 / 恢复走 `replace`）。
    fn append(&self, name: &str, content: &Content) -> Result<(), String>;
    /// 整体重写内容（只有版本控制的删除 / 恢复会真的截断）。
    fn replace(&self, name: &str, content: &Content) -> Result<(), String>;
    /// 列出全部落盘会话（列表页按它渲染）。
    fn list(&self) -> Result<Vec<HistoryView>, String>;
    /// **只读元信息**（不回放流水）：运行态（暂停 / 关闭）判定走它，不为此读整份转录。
    fn meta(&self, name: &str) -> Result<SessionMeta, String>;
    /// 打开一个会话：元信息 + 内容日志（调用方按它回放状态）。
    fn load(&self, name: &str) -> Result<(SessionMeta, Content), String>;
    /// 删除一个会话目录；false = 本来就不存在。
    fn delete(&self, name: &str) -> Result<bool, String>;
}
