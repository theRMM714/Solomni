//! 目的：**裁决通道的发起面**——流程段与工具层都经它提问；排队、校验与回答口在通道实现里（回合状态单元）。
//! 管：`AskDesk`（挂起一张卡 + 问"是不是有人在等"）这一条契约。
//! 不管：队列本体、卡号发放、回答的校验、卡片怎么画（在通道实现与转录里）。
//! 联动：流程段在 `src/units/business/collab/service/` 里只用这一面挂卡；阻塞那一面是环 0 的 `crate::kernel::ports::AskUser`
//!   （工具在会话下面，看不见会话事件），两者的实现是同一个；契约见 docs/session/session-model.md 的「请用户裁决」。

use crate::kernel::api::types::Ask;
use crate::units::business::transcript::api::SessionEvent;

/// 目的：一个会话的裁决通道——发起方只经它挂卡：**不阻塞**，卡片事件由发起方插到自己的流水里。
/// 约束：空选项集、没人答、用户停止这些事实由实现按 `crate::kernel::api::types::AskOutcome` 如实分开。
pub trait AskDesk: Send + Sync {
    /// 目的：挂一张卡（**不阻塞**）。
    /// 参数：`card` = 沿用的原卡号（重建时那张没人答的卡用它自己的号；`None` = 新发一个）；`ask` = 这一问。
    /// 返回：卡号与要外送的事件（推的与快照同源；插入位置由发起方定）。
    fn later(&self, card: Option<&str>, ask: &Ask) -> (String, Vec<SessionEvent>);

    /// 目的：这一刻是不是有人在等用户（流程据此决定停在哪一步）。
    fn pending(&self) -> bool;

    /// 目的：队首那一问（卡号 + 这一问）：流程"继续"时看的是自己发的那张。
    /// 返回：没挂起 = `None`。
    fn head(&self) -> Option<(String, Ask)>;

    /// 目的：把队首**原样出队**（流程自己消费掉那张卡时用：不记回答、不唤醒任何人）。
    fn drop_head(&self);
}
