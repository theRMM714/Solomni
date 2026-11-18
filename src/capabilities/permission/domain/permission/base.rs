//! 目的：权限的基础词汇——reach 域、effect 类别与两者收窄时的处置序（Deny 最严、Allow 最宽）。
//! 管：`Reach` / `ReachSet` / `Effect` / `Decision`，以及 `ReachSet` 的集合运算与 `Decision` 的收窄序。
//! 不管：档位预设（`profile.rs`）、共享区范围（`scope.rs`）、一层覆盖与生效态（`layer.rs` / `set.rs`）。
//! 联动：`tools` 用它生成围栏与工具面，`workspace` 用它判落点（docs/permission/README.md）。

use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// 目的：一次操作能到的资源域。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Reach {
    /// 目的：自己的私有沙箱。
    Private,
    /// 目的：读共享区主副本。
    SharedRead,
    /// 目的：写共享区（只有 work_commit 这条路径使用）。
    SharedWrite,
    /// 目的：读成员模块目录。
    ModuleRead,
    /// 目的：写模块自己的 userdata（只给模块工具进程）。
    ModuleWrite,
    /// 目的：用户显式授权的主机只读根。
    HostRead,
    /// 目的：出站网络。
    Network,
}

/// 目的：一组 reach；序列化成稳定排序的数组，便于配置视图与审计。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ReachSet(BTreeSet<Reach>);

impl ReachSet {
    /// 目的：空集合（没有任何 reach）。
    pub fn empty() -> ReachSet {
        ReachSet(BTreeSet::new())
    }

    /// 目的：这个集合有没有这一项。
    pub fn contains(&self, r: Reach) -> bool {
        self.0.contains(&r)
    }

    /// 目的：是否为空。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// 目的：两项的交集。
    pub fn intersection(&self, other: &ReachSet) -> ReachSet {
        ReachSet(self.0.intersection(&other.0).copied().collect())
    }

    /// 目的：自己是不是 other 的子集（收窄校验用）。
    pub fn is_subset(&self, other: &ReachSet) -> bool {
        self.0.is_subset(&other.0)
    }

    /// 目的：按稳定顺序列出（配置视图、错误文案与测试）。
    pub fn to_vec(&self) -> Vec<Reach> {
        self.0.iter().copied().collect()
    }
}

impl FromIterator<Reach> for ReachSet {
    fn from_iter<T: IntoIterator<Item = Reach>>(iter: T) -> ReachSet {
        ReachSet(iter.into_iter().collect())
    }
}

impl<const N: usize> From<[Reach; N]> for ReachSet {
    fn from(items: [Reach; N]) -> ReachSet {
        ReachSet(items.into_iter().collect())
    }
}

/// 目的：这次操作属于哪一类（决定用 profile 的哪一个 decision 字段）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    /// 目的：读类。
    Read,
    /// 目的：写类。
    Write,
    /// 目的：执行类（模块工具 / 未来的 shell）。
    Exec,
    /// 目的：不碰文件、不需要权限闸门。
    None,
}

impl Effect {
    /// 目的：旧 capability 标签到 effect 的默认映射；工具显式写了 effect 时以显式值为准。
    pub fn from_capability(capability: &str) -> Effect {
        match capability.trim() {
            "fs-read" => Effect::Read,
            "fs-write" => Effect::Write,
            "exec" => Effect::Exec,
            _ => Effect::None,
        }
    }

    /// 目的：没写 reach 时按 effect 给的默认可达范围。
    pub fn default_reach(self) -> ReachSet {
        match self {
            Effect::Read => ReachSet::from([Reach::Private, Reach::SharedRead, Reach::ModuleRead]),
            Effect::Write => ReachSet::from([Reach::Private]),
            Effect::Exec => ReachSet::from([Reach::Private]),
            Effect::None => ReachSet::empty(),
        }
    }
}

/// 目的：一次调用的处置。Deny 最严、Allow 最宽；收窄时取更严的一边。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    /// 目的：不执行。
    Deny,
    /// 目的：进统一裁决队。
    Ask,
    /// 目的：直接执行。
    #[default]
    Allow,
}

impl Decision {
    /// 目的：两者中更严的那个。
    pub fn stricter(self, other: Decision) -> Decision {
        self.min(other)
    }

    /// 目的：是不是拒绝。
    pub fn is_deny(self) -> bool {
        self == Decision::Deny
    }

    /// 目的：是不是要问。
    pub fn is_ask(self) -> bool {
        self == Decision::Ask
    }

    /// 目的：内部排序（0 = 最严）。
    pub(super) fn rank(self) -> u8 {
        match self {
            Decision::Deny => 0,
            Decision::Ask => 1,
            Decision::Allow => 2,
        }
    }
}
