//! 目的：工作区单元的入站面——其它单元与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：`Workspace` 用例 trait（目录布局与文件寻址）与它的视图类型；共享区归属规则与过滤名单的重导出。
//! 不管：版本化用例（在 `versions` 单元的 `api`）；用例实现（在 `service/`）；落盘机制（在 `detail/`）。
//! 联动：实现见 `service.rs`；`versions` 借本面问"在哪儿"，`tools` 借 `Sandbox` / `Place` 定位读写落点。

/// 目的：工作区过滤的唯一名单与匹配规则——`tools` 的 grep 与内容面的导入 / 清单都只从这里取。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::business::workspace::domain::path_filter;
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::business::workspace::domain::path_filter::{
    MAX_RULE_BYTES, MAX_RULE_CHARS, MAX_RULE_LINES, Rules, WORKIGNORE, WORKIGNORE_SEED, check_rule,
};

/// 目的：共享区路径归属（读 / 写范围与黑名单）——\`permission\` 只管档位与网络，归属规则在这里。
#[allow(unused_imports)] // 入站面：规则类型给调用方与测试取用
pub use crate::units::business::workspace::domain::scope::{
    ScopeMode, SharedEdit, SharedScope, edit_of, edit_params, hit, normalize, scope_from,
    validate_entry,
};

/// 目的：版本库共享类型（提交 / 拉取 / 推送 / 状态 / 差异 / 回退）——`tools`、呈现层与测试只从这里取。
/// 约束：`WorkState` 等类型只在测试与呈现层命名，本 crate 的实现不直接用，故整组允许未使用告警。
#[allow(unused_imports)]
pub use crate::units::business::workspace::domain::seats::{
    ModuleDir, SeatFacts, SeatPlan, seat_sandboxes,
};

pub use crate::units::business::workspace::domain::workspace::{
    AreaUsage, Place, Sandbox, Sandboxes, WorkFiles, WorkRoots, WorkUsage, safe_file_name,
};

/// 目的：外部工作区根的策略（判定 / 创建校验 / 加载复检）——`runtime` 建会话与重开时只经这里取。
#[allow(unused_imports)] // 入站面：策略函数给调用方与测试取用
pub use crate::units::business::workspace::service::roots::{
    ambient, policy, product_root, recheck, resolve,
};

/// 目的：工作区的**用例面**（`service.rs` 实现）：目录布局与文件寻址，走这里。
/// 约束：出站端口（`Workdirs`）**只由它持有**（R12）；版本化用例在 `versions::api::Versions`。
pub trait Workspace: Send + Sync {
    /// 目的：把根会话的工作区绑定落成布局投影（`Some` = 外部工作区根；`None` = 产品内布局）。
    /// 约束：会话 meta 是绑定的唯一真相；本调用只刷新会话目录里的布局投影，不碰工作区文件。
    fn bind(&self, session: &str, root: Option<&std::path::Path>) -> Result<(), String>;
    /// 按会话铺布局：建 session/<工作名>/work 与每个 agent 的沙箱目录。
    /// 约束：只落目录与版本库种子，不产生会话 / 工作名 / 通道 / guest——建工作的唯一入口在 work。
    fn ensure_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// 撤销一次失败的铺布局：只删本产品建出的空沙箱 / 空 work/、布局投影与版本库 `.work`。
    /// 约束：调用方只在会话**从未落盘**时调用；用户文件绝不因为一次失败的创建被删。
    fn discard_layout(&self, session: &str, agents: &[String]) -> Result<(), String>;
    /// work/ 下是否已有同名文件（上传同名冲突判定）。
    fn work_has(&self, session: &str, name: &str) -> bool;
    /// 列出本工作可引用的文件（work/ 与各 agent 沙箱；相对路径、/ 分隔、排序稳定）。
    fn files(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String>;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在本单元。
    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String>;

    /// 目的：该会话是否绑定了外部工作区（只读布局投影）。
    fn is_external(&self, session: &str) -> bool;
    /// 目的：拿住该工作区根的**跨会话写闸**；`None` = 产品内布局（已有按工作 / 版本库的锁）。
    /// 约束：多个会话绑定同一个外部目录时共用同一把锁；guard 要活到这次操作结束。
    fn shared_lock_path(&self, session: &str) -> Result<Option<std::path::PathBuf>, String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
}
