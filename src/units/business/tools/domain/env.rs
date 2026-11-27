//! 目的：一席工具运行环境的派生规则——围栏、PATH 入口、工具链根与模块注入项。
//! 管：`member_env`（把沙箱与包事实算成一席的运行环境）、`host_read_roots`（用户显式授权的主机只读根）。
//! 不管：沙箱怎么来（在工作台单元）、取事实（在 `turn`）、围栏机制（在 kernel）。
//! 联动：`turn` 取好沙箱与包计划后转调；包事实以「包根 + PATH」两个值进来，本单元不认识包计划类型。

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::PathBuf;

use crate::kernel::api::fence::FencePlan;
use crate::units::business::workbench::api::Bench;
use crate::units::pure::exec_model::api::Side;

/// 目的：派生一席工具环境所需的全部已取事实。
pub struct MemberEnvFacts<'a> {
    /// 目的：该 agent 的沙箱（可达范围与权限面）。
    pub bench: &'a Bench,
    /// 目的：用户显式授权的主机只读根。
    pub host_read_roots: &'a [PathBuf],
    /// 目的：该会话是否放行出站网络。
    pub net: bool,
    /// 目的：执行档位（宿主 / 虚拟机）。
    pub side: Side,
    /// 目的：基础工具链根（只在宿主档放行）。
    pub base_roots: &'a [PathBuf],
    /// 目的：本机运行包的包根（并进只读树与工具链根）。
    pub package_roots: &'a [PathBuf],
    /// 目的：本机运行包换过的 PATH 入口（没换过就是 None）。
    pub package_path: Option<OsString>,
    /// 目的：各模块已解析的隐私注入项。
    pub module_vars: &'a BTreeMap<String, Vec<(String, String)>>,
}

/// 目的：一席工具环境里由事实派生的部分。
pub struct MemberEnv {
    /// 目的：该 agent 的围栏（可达范围 + 断网 + 环境白名单落点）。
    pub fence: FencePlan,
    /// 目的：工具进程的 PATH 入口（本机运行包换过 PATH 时才有）。
    pub runtime_path: Option<OsString>,
    /// 目的：工具链根：宿主档 = 基础根 + 包根，虚拟机档 = 包根。
    pub runtime_roots: Vec<PathBuf>,
    /// 目的：各模块隐私字段的注入项（模块 id → 变量对）。
    pub module_env: BTreeMap<String, Vec<(String, String)>>,
}

/// 目的：派生一席的围栏、运行路径、工具链根与模块注入项；纯函数，不取事实、不落盘。
/// 参数：`facts` = 调用方已取好的事实（沙箱、只读根、网络、档位、基础根、包根、模块机密）。
/// 返回：围栏（只读树并进包根、去重保序）、PATH 入口、工具链根、模块注入项。
pub fn member_env(facts: &MemberEnvFacts<'_>) -> MemberEnv {
    let mut fence = FencePlan::for_session(facts.bench, facts.host_read_roots, facts.net);
    fence.extend_read_tree(facts.package_roots.iter().cloned());
    let mut runtime_roots = if facts.side == Side::Host {
        facts.base_roots.to_vec()
    } else {
        Vec::new()
    };
    runtime_roots.extend(facts.package_roots.iter().cloned());
    runtime_roots.sort();
    runtime_roots.dedup();
    MemberEnv {
        fence,
        runtime_path: facts.package_path.clone(),
        runtime_roots,
        module_env: facts.module_vars.clone(),
    }
}

/// 目的：用户显式授权的主机只读根（设置里的 `sandbox.policy.host_read_roots`）。
/// 参数：`declared` = 设置里声明的字符串清单。
/// 约束：逐条去空白、丢掉空项再转路径——空串不是「根」，一个都不放行时给空表
///   （只读位怎么落由适配层定，这里只给事实）。
pub fn host_read_roots(declared: &[String]) -> Vec<PathBuf> {
    declared
        .iter()
        .map(|p| p.trim())
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        .collect()
}
