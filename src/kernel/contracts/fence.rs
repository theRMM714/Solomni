//! 目的：围栏的**意图**——谁（agent + 租约）要在哪些路径上得到什么访问、网络要不要；平台无关的纯数据。
//! 管：`FencePlan` / `Grant` / `Access` / `Reach` / `NetPolicy`、执行宽严 `FenceRunPolicy`、
//!   落点授不上的中性事实 `FenceBlocked`、守门进程的失败退出码 `FENCE_FAILED`。
//! 不管：意图怎么从沙箱与模块事实派生（`capabilities/tools/domain/fence.rs`）；
//!   意图怎么落到本机机制（`detail/confine` 的平台实现）；按次租约与全局沙箱要求（`kernel/domain/fence.rs`）。
//! 联动：由 `src/kernel/api/fence.rs` 重导出；守门进程协议与工具执行层消费它。

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 目的：围栏装不上时守门进程的退出码——工具执行据此如实报错，不静默。
/// 约束：这是**守门进程协议的**退出码（守门进程返它，调用方按它判失败），改它等于改协议。
pub const FENCE_FAILED: i32 = 111;

/// 目的：守门进程执行时的全局沙箱要求——机制装不上时是降级还是拒绝。
/// 约束：`allow_degraded` 只来自本进程的启动问答；不落盘。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct FenceRunPolicy {
    #[serde(default)]
    pub require_fs: bool,
    #[serde(default)]
    pub require_net: bool,
    #[serde(default = "default_allow_degraded")]
    pub allow_degraded: bool,
}

fn default_allow_degraded() -> bool {
    true
}

impl Default for FenceRunPolicy {
    fn default() -> Self {
        FenceRunPolicy {
            require_fs: false,
            require_net: false,
            allow_degraded: true,
        }
    }
}

/// 目的：一份授予的**访问档**——只有两档：执行位随读写一起给。
/// 约束：三平台都不支持单独收窄执行权（Windows 的 `RIGHTS_RW` / `RIGHTS_RO` 都含 `FILE_GENERIC_EXECUTE`，
///   Linux 的 `RW_ALL` / `RO_ALL` 都并进 `FS_EXECUTE`，macOS 由 `(allow process*)` 全局放行），
///   所以不设 `Exec` 档——设了也落实不了，只会变成一句假话。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Access {
    /// 只读：能读、能列目录、能执行其中已有的文件。
    Read,
    /// 读写：读之上再能写、删、改权限。
    Write,
}

/// 目的：一份授予的**落点形状**。
/// 约束：`Dir` 是"只这一层、不含子孙"，只有 Windows 的 DACL 能如实表达（用户授权的主机只读根不递归）；
///   Landlock（path-beneath）与 seatbelt（subpath）只有子树粒度，在那里按 `Tree` 落实——如实放宽，不假装收窄。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Reach {
    /// 只有这一层。
    Dir,
    /// 整棵子树。
    Tree,
}

/// 目的：一条"某个路径上给到什么访问、给到多深"的意图；平台按自己的机制落实它。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    pub path: PathBuf,
    pub access: Access,
    pub reach: Reach,
}

impl Grant {
    /// 目的：只这一层可读（用户显式授权的主机只读根：不递归——用户可能授很大的目录）。
    pub fn read(path: impl Into<PathBuf>) -> Grant {
        Grant {
            path: path.into(),
            access: Access::Read,
            reach: Reach::Dir,
        }
    }

    /// 目的：整棵子树可读（模块目录：工具脚本与依赖都在里面，必须递归可读）。
    pub fn read_tree(path: impl Into<PathBuf>) -> Grant {
        Grant {
            path: path.into(),
            access: Access::Read,
            reach: Reach::Tree,
        }
    }

    /// 目的：整棵子树可读写（私有沙箱、模块自己的 `userdata/`）。
    pub fn write_tree(path: impl Into<PathBuf>) -> Grant {
        Grant {
            path: path.into(),
            access: Access::Write,
            reach: Reach::Tree,
        }
    }
}

/// 目的：出站网络的意图（全局 `sandbox.policy.network` 与会话权限的合成结果）。
/// 约束：Windows 只放行公网出站，不授 private network / loopback；
///   Linux 的 `Off` 用 seccomp 拒绝非本地 socket（`Public` 不装网络过滤器）；macOS 放行 `network-outbound`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum NetPolicy {
    /// 断网。
    #[default]
    Off,
    /// 放行公网出站。
    Public,
}

impl From<bool> for NetPolicy {
    fn from(on: bool) -> NetPolicy {
        NetPolicy::from_flag(on)
    }
}

impl NetPolicy {
    /// 目的：把布尔开关翻成策略（派生点手里通常只是一个 `bool`）。
    pub fn from_flag(on: bool) -> NetPolicy {
        if on {
            NetPolicy::Public
        } else {
            NetPolicy::Off
        }
    }

    /// 目的：要不要放行出站网络。
    pub fn on(&self) -> bool {
        matches!(self, NetPolicy::Public)
    }
}

/// 目的：把三组路径摊成授予清单——写子树、单层只读、只读子树。
/// 约束：这是"三组路径"这一常见输入形状的便捷口（派生点与测试夹具用），不是对外的意图模型：
///   需要逐条表达时直接用 `Grant::read` / `read_tree` / `write_tree` 推 `grants`。
pub fn grants_of(
    write: Vec<PathBuf>,
    read_dir: Vec<PathBuf>,
    read_tree: Vec<PathBuf>,
) -> Vec<Grant> {
    let mut out: Vec<Grant> = Vec::with_capacity(write.len() + read_dir.len() + read_tree.len());
    out.extend(write.into_iter().map(Grant::write_tree));
    out.extend(read_dir.into_iter().map(Grant::read));
    out.extend(read_tree.into_iter().map(Grant::read_tree));
    out
}

/// 目的：一次外部进程执行的**围栏意图**（纯数据）：谁、在哪个目录、拿哪些路径的什么访问、网络要不要。
/// 约束：`home` 隐含一份"整棵子树可读写"的授予，不必重复写进 `grants`；
///   其余授予互不推导（不收窄、不合并），由平台各自如实落实。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FencePlan {
    /// 目的：该 agent 的实例名（日志与审计用）。
    pub agent: String,
    /// 目的：本席所属会话的租约（会话 id；空 = 无会话，如独立模块工具）。
    ///   同名 agent 的多个并发会话共用同一容器 SID 与同一批授权，释放时靠它区分"还有哪个会话在用"。
    #[serde(default)]
    pub lease: String,
    /// 目的：该 agent 私有沙箱——工具进程 HOME / TEMP 的落点（空 = 退回 cwd），隐含可读写。
    #[serde(default)]
    pub home: PathBuf,
    /// 目的：工具进程的工作目录（它所属模块的根目录）。
    pub cwd: PathBuf,
    /// 目的：除 `home` 之外的授予清单。
    #[serde(default)]
    pub grants: Vec<Grant>,
    /// 目的：这次执行要不要放行出站网络。
    pub net: NetPolicy,
}

impl FencePlan {
    /// 目的：把这个围栏的工作目录设成某个模块的根（该模块的工具就在这里跑）。
    /// 约束：只改工作目录，不改授予——cwd 可读由覆盖它的那条授予保证（模块目录恒在只读子树里）。
    pub fn at(&self, module_root: &Path) -> FencePlan {
        let mut out = self.clone();
        out.cwd = module_root.to_path_buf();
        out
    }

    /// 目的：该 agent 的私有沙箱（没有就退回工作目录）——环境里的 HOME / TEMP 落点。
    pub fn home_or_cwd(&self) -> PathBuf {
        if self.home.as_os_str().is_empty() {
            self.cwd.clone()
        } else {
            self.home.clone()
        }
    }

    /// 目的：某一档访问 + 某种落点形状下的全部路径（按清单序）——平台落实与测试断言都按它取。
    pub fn paths(&self, access: Access, reach: Reach) -> Vec<PathBuf> {
        self.grants
            .iter()
            .filter(|g| g.access == access && g.reach == reach)
            .map(|g| g.path.clone())
            .collect()
    }

    /// 目的：这次执行**可写**的根（`home` + 所有写授予）——撤销授权、容器身份指纹与挂载都按它算。
    pub fn write_roots(&self) -> Vec<PathBuf> {
        let mut out: Vec<PathBuf> = Vec::new();
        if !self.home.as_os_str().is_empty() {
            out.push(self.home.clone());
        }
        out.extend(
            self.grants
                .iter()
                .filter(|g| g.access == Access::Write)
                .map(|g| g.path.clone()),
        );
        out.sort();
        out.dedup();
        out
    }

    /// 目的：把若干根并进"整棵子树只读"的授予（运行包 / 工具链根这类派生根），随后收一遍重复与覆盖。
    pub fn extend_read_tree(&mut self, roots: impl IntoIterator<Item = PathBuf>) {
        self.grants.extend(roots.into_iter().map(Grant::read_tree));
        self.normalize();
    }

    /// 目的：收掉被写授予覆盖的读授予与完全重复的授予，并让清单顺序与平台无关地确定下来。
    /// 约束：写授予排在读授予之前（同一路径上先给宽的那一条）；被 `home` 或某条写子树覆盖的读授予直接去掉。
    pub fn normalize(&mut self) {
        let home = self.home.clone();
        let writes: Vec<PathBuf> = self
            .grants
            .iter()
            .filter(|g| g.access == Access::Write && g.reach == Reach::Tree)
            .map(|g| g.path.clone())
            .collect();
        let under = |path: &Path, root: &Path| {
            !root.as_os_str().is_empty() && (path == root || path.starts_with(root))
        };
        let mut kept: Vec<Grant> = Vec::with_capacity(self.grants.len());
        for grant in self.grants.drain(..) {
            if grant.access == Access::Read
                && (under(&grant.path, &home) || writes.iter().any(|w| under(&grant.path, w)))
            {
                continue;
            }
            kept.push(grant);
        }
        kept.sort_by(|a, b| {
            let wide = |g: &Grant| matches!(g.access, Access::Write);
            wide(b)
                .cmp(&wide(a))
                .then_with(|| a.path.cmp(&b.path))
                .then_with(|| a.reach.cmp(&b.reach))
        });
        kept.dedup();
        self.grants = kept;
    }
}

/// 目的：一个落点授不上的如实结论——**哪一环** + 哪个目录 + 缺什么前提 + 怎么补。
/// 约束：它是"必要落点授不上"这条路的唯一材料——裁决卡、回执与停会话警告都从它派生；
///   措辞由发现它的平台给（"解释器安装目录""祖先链"这类是机制词汇，不属于意图层）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceBlocked {
    /// 目的：哪一环授不上（给用户看的名字）。
    pub what: &'static str,
    /// 目的：授不上的那个目录；没有具体目录的一环（容器身份 / 台账）= 空。
    pub path: PathBuf,
    /// 目的：缺什么前提（机制给的原话，例如写权限项的失败原因）。
    pub why: String,
    /// 目的：怎么补（一句可操作的话）。
    pub fix: &'static str,
}

impl FenceBlocked {
    /// 目的：这一环的一句话（哪一环、哪个目录、缺什么前提、怎么补）——给用户看的原话。
    pub fn line(&self) -> String {
        let where_ = if self.path.as_os_str().is_empty() {
            String::new()
        } else {
            format!("（{}）", self.path.display())
        };
        format!(
            "{}{} 授不上：{}；怎么补：{}",
            self.what, where_, self.why, self.fix
        )
    }
}
