//! 目的：构建能力的入站面——目标环境与"确保产物可用"的唯一契约。
//! 管：`Target`（本机 / 某个虚拟根）、`Built`（一次确保的结果）、`Builds` 用例 trait 与未接入的 `NoBuilds`。
//! 不管：怎么选包、怎么装围栏、怎么起 guest（在 `service.rs` 与各能力的 api）。
//! 联动：`conductor` 在创建会话前调它补编；`diagnostics --build` 是它的薄壳；契约见 docs/builds/README.md。

use crate::capabilities::modules::api::Module;

/// 目的：构建的目标环境——本机档在宿主上编，虚拟机档在指定的虚拟根里编（产物形态随目标平台）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// 宿主。
    Host,
    /// 一个虚拟根（`.vm/guest/<名>`）。
    Guest(String),
}

impl Target {
    /// 目的：给人看的目标名——本机档是 `host`，虚拟机档是那个虚拟根名。
    /// 返回：目标名（日志、构建状态与错误文案共用它）。
    pub fn label(&self) -> String {
        match self {
            Target::Host => "host".to_string(),
            Target::Guest(g) => g.clone(),
        }
    }
}

/// 目的：一次"确保产物可用"的结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Built {
    /// 目的：模块 id。
    pub module: String,
    /// 目的：目标环境名（`host` / 虚拟根名）。
    pub target: String,
    /// 目的：入口本来就在场，没跑构建。
    pub skipped: bool,
    /// 目的：构建命令的输出（跳过时为空）。
    pub output: String,
}

/// 目的：构建能力的用例面——"确保这个模块在目标环境里有能直接跑的入口"。
pub trait Builds: Send + Sync {
    /// 目的：确保模块在目标环境里有能直接跑的入口：入口与 `build-state.target` 都匹配才跳过；
    /// 不在场、或状态是别的目标就照 `build:` 编一次；没有状态视为随仓自带产物。
    /// 参数：`module` = 模块（清单事实）；`target` = 目标环境；`net` = 这次构建放不放行出站网络（全局开关与模块声明取交集）。
    /// 返回：这次的事实（是否跳过、构建输出）。
    /// 错误：入口不在场且模块没声明 `build:`、构建期能力缺失、构建命令失败、编完入口仍不在场——都如实拒绝并给原因。
    fn ensure(&self, module: &Module, target: &Target, net: bool) -> Result<Built, String>;
    /// 目的：按声明编一次（`--build` 的语义）：入口在场也看源指纹，变了就重编，没变就跳过。
    /// 参数：同 `ensure`。
    /// 返回：这次的事实（是否跳过、构建输出）。
    /// 错误：模块没声明 `build:`、构建期能力缺失、构建命令失败、编完入口仍不在场。
    fn build(&self, module: &Module, target: &Target, net: bool) -> Result<Built, String>;
}

/// 目的：未接入的构建面——测试装配用的默认值（生产组合根一律注入真实现）。
/// 约束：`ensure` 一律"跳过"（不补编，入口判据照旧由准备段执行）；`build` 如实失败。
#[cfg(test)]
pub struct NoBuilds;

#[cfg(test)]
impl Builds for NoBuilds {
    fn ensure(&self, module: &Module, target: &Target, _net: bool) -> Result<Built, String> {
        Ok(Built {
            module: module.manifest.id.clone(),
            target: target.label(),
            skipped: true,
            output: String::new(),
        })
    }
    fn build(&self, _module: &Module, _target: &Target, _net: bool) -> Result<Built, String> {
        Err("构建能力未接入".to_string())
    }
}
