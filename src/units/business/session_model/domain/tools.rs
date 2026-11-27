//! 目的：工具来源端口——会话只收工具 id，谁交 id 由来源自己声明；来源只回答在场，不回答准不准。
//! 管：`ToolSource` 与它拿到的 `TurnCtx`；五个来源 id 与四个能力组名（字符串口径，不引 tools 单元的形状）。
//! 不管：工具形状与实现（在 tools / agentloop / workspace）；授权（在权限端口）；来源实例化（在组合根）。
//! 联动：会话类型表声明挂哪些来源；agentloop 的装配按 `offer` 求并集；引擎按 id 找实现。

/// 目的：读面能力组（read / list / grep）。
pub const FS_READ: &str = "fs-read";
/// 目的：工作面能力组（write / edit / patch / delete）。
pub const FS_WRITE: &str = "fs-write";
/// 目的：执行能力组（shell）。
pub const EXEC: &str = "exec";
/// 目的：共享区写入能力组（workspace 的 work_* 面）。
pub const SHARED_WRITE: &str = "shared-write";

/// 目的：来源 id——agent 自己的能力组。
pub const SOURCE_AGENT: &str = "agent";
/// 目的：来源 id——agent 装的模块工具。
pub const SOURCE_MODULE: &str = "module";
/// 目的：来源 id——当前回合所在 loop 的工具。
pub const SOURCE_LOOP: &str = "loop";
/// 目的：来源 id——共享区（工作区）声明的工具。
pub const SOURCE_WORKSPACE: &str = "workspace";
/// 目的：来源 id——会话类型展开进会话的 system 工具。
pub const SOURCE_PROFILE: &str = "profile";

/// 目的：一次回合的上下文——来源据此决定交不交自己的工具。
/// 约束：只装窄值，不传会话对象；段与角色已经由装配期拍平进各来源的实例，端口不再传它们。
pub struct TurnCtx<'a> {
    /// 目的：当前回合所在的 loop id；不在流程里为空。
    /// 约束：端口契约的字段，当前没有来源读它（段声明在装配期已拍平）——留着给按段分面的来源。
    #[allow(dead_code)]
    pub loop_id: Option<&'a str>,
    /// 目的：当前回合是哪个 agent。
    /// 约束：端口契约的字段，当前没有来源读它——留给按 agent 分面的来源。
    #[allow(dead_code)]
    pub agent_id: &'a str,
    /// 目的：这个 agent 的能力组。
    pub abilities: &'a [String],
    /// 目的：会话类型 id。
    /// 约束：端口契约的字段，当前没有来源读它——留给按会话类型分面的来源。
    #[allow(dead_code)]
    pub profile: &'a str,
    /// 目的：工作区 id；没有工作区为空。
    /// 约束：端口契约的字段，当前没有来源读它（共享区动作由工作台策略的 plan 发放）——留给按工作区策略分面的来源。
    #[allow(dead_code)]
    pub workspace: Option<&'a str>,
}

impl TurnCtx<'_> {
    /// 目的：这个回合有没有某个能力组。
    pub fn has(&self, ability: &str) -> bool {
        self.abilities.iter().any(|a| a == ability)
    }
}

/// 目的：工具来源的入站端口——组合根实现它，装配只按 id 求并集。
/// 约束：只回答「这个回合有哪些工具」；不判授权，不看别人的工具；返回的 id 必须在总表里。
pub trait ToolSource: Send + Sync {
    /// 目的：来源 id（会话类型表按这个 id 挂来源）。
    fn id(&self) -> &str;
    /// 目的：这个回合要交的工具 id，按交的顺序。
    fn offer(&self, ctx: &TurnCtx) -> Vec<String>;
}
