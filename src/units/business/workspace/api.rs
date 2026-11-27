//! 目的：工作区单元的入站面——其它单元与呈现层只准用这里（不许碰 `domain` / `ports`）。
//! 管：`Workspace` 用例 trait（目录布局与文件寻址）、席位工作面的组装（`seat_benches` 与它的参数对象）、
//!   共享区归属规则（`edit_of` / `edit_params` / `scope_from`）、@ 改写用的引用根（`ref_roots`）、外部根策略（`roots`）。
//! 不管：工作面本身的形状与寻址语义（环 2 契约单元 `workbench`）；版本化用例（`versions`）；落盘机制（`detail/`）。
//! 联动：实现见 `service.rs`；`tools` / `agentloop` 只认 `workbench` 的形状，`turn` 用本面组装席位工作面。

/// 目的：把会话落盘的策略参数解释成生效归属——值类型与规则在 `permission`，参数形状只在本单元解释。
#[allow(unused_imports)] // 入站面：参数函数给调用方与测试取用
pub use crate::units::business::workspace::domain::scope::{edit_of, edit_params, scope_from};

/// 目的：席位工作面的组装入口与它的参数对象——`turn` 取好事实后转调，工作面清单的唯一构造点。
#[allow(unused_imports)] // 入站面：组装函数给调用方与测试取用
pub use crate::units::business::workspace::domain::seats::{
    ModuleDir, SeatFacts, SeatPlan, seat_benches,
};

/// 目的：外部工作区根的策略（判定 / 创建校验 / 加载复检）——`runtime` 建会话与重开时只经这里取。
#[allow(unused_imports)] // 入站面：策略函数给调用方与测试取用
pub use crate::units::business::workspace::service::roots::{
    ambient, policy, product_root, recheck, resolve,
};

/// 目的：不版本化的工作区策略（单树直写）——组合根接策略表时与 `versions` 的 versioned 一起接。
#[allow(unused_imports)] // 入站面：策略给组合根取用
pub use crate::units::business::workspace::service::plain::plain_policy;

/// 目的：用户输入里 @ 引用要的根（共享区 + 本会话的路径翻译器）——协作 / 主持人的话用它改写。
/// 参数：`boxes` = 这次工作的席位工作面清单。
/// 约束：private 恒为 None——这些入口没有"自己的沙箱"（与说话人无关的共读文本）。
pub fn ref_roots(
    boxes: &crate::units::business::workbench::api::Benches,
) -> crate::units::business::prompt::api::RefRoots {
    crate::units::business::prompt::api::RefRoots {
        work: boxes.shared.clone(),
        private: None,
        path_codec: boxes.codec.clone(),
    }
}

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
    fn files(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workbench::api::WorkFiles, String>;
    /// 统计工作区用量（共享区 + 各 agent 沙箱的文件数与总字节）：删除前如实交代用。
    fn usage(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workbench::api::WorkUsage, String>;
    /// 沙箱寻址根（work 与各 agent 私有区）：布局机制在适配层，拼接与越界校验在工作台契约单元。
    fn roots(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::units::business::workbench::api::WorkRoots, String>;

    /// 目的：该会话是否绑定了外部工作区（只读布局投影）。
    fn is_external(&self, session: &str) -> bool;
    /// 目的：拿住该工作区根的**跨会话写闸**；`None` = 产品内布局（已有按工作 / 版本库的锁）。
    /// 约束：多个会话绑定同一个外部目录时共用同一把锁；guard 要活到这次操作结束。
    fn shared_lock_path(&self, session: &str) -> Result<Option<std::path::PathBuf>, String>;
    /// 界面投喂：把文件写进本工作的 work/（文件名由调用方净化）。
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String>;
}
