//! 目的：一次围栏授权的**落点环节**——授权装不上时，用户与模型都要知道缺的是哪一环。
//! 管：环节枚举（必要 / 可选）、给人看的环节名与补法、环节翻成对外事实（`FenceBlocked`）。
//! 不管：授权本身怎么落（在 `windows.rs`）、结论怎么收尾（在 `confine::prep`）。
//! 联动：`windows.rs` 的授权步骤逐条报环节；`confine::prep::FencePrep` 只认翻好的 `FenceBlocked`，不认平台环节。

use crate::kernel::contracts::fence::FenceBlocked;
use std::path::PathBuf;

/// 目的：一次围栏执行的**落点环节**——授权装不上的时候，用户与模型都要知道缺的是哪一环。
/// 约束：**必要 / 可选的判据只在这一处**（`necessary`）：必要落点缺了这次命令在容器里起不来，
///   或围栏本身不成立；可选落点缺了命令照跑，只是可达范围小一点。
///   这是**机制词汇**（解释器基线、祖先链、容器身份这些只有平台知道），所以留在实现层，不进对外契约。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum FencePart {
    /// 解释器安装目录（基线只读，授给本产品能力 SID）：容器里连解释器都起不来。
    Interpreter,
    /// 模块目录（只读子树）：工具脚本与它的依赖都在这里。
    Module,
    /// 该 agent 的私有沙箱：工具进程 `HOME` / `TEMP` 的落点。
    Sandbox,
    /// 其余**数据边界**（共享区主副本、模块 `userdata/` 这类读写根）：这次执行要用到的根。
    DataBoundary,
    /// 用户显式授权的主机只读根（`sandbox.policy.host_read_roots`）。
    AuthorizedRead,
    /// 数据边界的父目录（只授读属性：容器里判"这个目录在不在"用它）。
    Parent,
    /// 容器身份（派生 AppContainer SID）：没有它谈不上容器围栏。
    ContainerIdentity,
    /// 授权台账（写 ACL 之前先落盘的那一份）：落不下就不能动本机权限项。
    Ledger,
    /// 落点的强制完整性标签：高于 Low 且不许向上写时，DACL 授权再对容器也写不进。
    IntegrityLabel,
    /// 本产品可执行文件（回环中继进程要在容器里跑起来，单文件只读）。
    Relay,
}

impl FencePart {
    /// 目的：这一环**必要**吗——缺了这次命令在容器里起不来，或这次执行根本做不了该做的事。
    /// 约束：判据只有这一处（枚举里这两条就是可选的）；可选落点授不上只记事实（见 `FencePrep`），
    ///   不牵动这次执行。
    pub(crate) fn necessary(&self) -> bool {
        !matches!(self, FencePart::AuthorizedRead | FencePart::Parent)
    }

    /// 目的：给人看的环节名（回执、裁决卡与警告都用它）。
    pub(crate) fn label(&self) -> &'static str {
        match self {
            FencePart::Interpreter => "解释器安装目录",
            FencePart::Module => "模块目录",
            FencePart::Sandbox => "该 agent 的私有沙箱",
            FencePart::DataBoundary => "数据边界（这次执行要读写的根）",
            FencePart::AuthorizedRead => "用户授权的只读根",
            FencePart::Parent => "数据边界的父目录",
            FencePart::ContainerIdentity => "容器身份",
            FencePart::Ledger => "授权台账",
            FencePart::IntegrityLabel => "落点的完整性标签",
            FencePart::Relay => "本产品可执行文件（回环中继）",
        }
    }

    /// 目的：这一环**怎么补**（一句可操作的话；回执与裁决卡都用它）。
    pub(crate) fn fix(&self) -> &'static str {
        match self {
            FencePart::Interpreter => {
                "把解释器装在你自己拥有的目录里（属主不是你就改不动它的权限项），或请管理员放行那个安装目录"
            }
            FencePart::Module => "把模块目录放进你自己拥有的位置（工作区里的模块目录属主就是你）",
            FencePart::Sandbox => "让这次工作的会话目录落在你自己拥有的位置（默认就在工作区里）",
            FencePart::DataBoundary => "让这次工作的目录落在你自己拥有的位置（默认就在工作区里）",
            FencePart::ContainerIdentity => {
                "确认本机能建 AppContainer profile（受限会话里建不起来）"
            }
            FencePart::Ledger => "确认台账目录 data/state/ 可写（授权台账要落在那里）",
            FencePart::IntegrityLabel => {
                "提权清掉这个目录的强制完整性标签（icacls 只能设级别、删不掉标签），或把落点换到未打标签的目录"
            }
            FencePart::Relay => {
                "确认本产品可执行文件在你拥有的位置、这次调用读得到它（它自己不能被换成别的文件）"
            }
            FencePart::AuthorizedRead | FencePart::Parent => {
                "这一次不影响围栏成立；下次想让它可达再补"
            }
        }
    }

    /// 目的：把这一环翻成对外事实（环节名与补法在这里落定，`FenceBlocked` 只带成品）。
    pub(crate) fn blocked(&self, path: PathBuf, why: String) -> FenceBlocked {
        FenceBlocked {
            what: self.label(),
            path,
            why,
            fix: self.fix(),
        }
    }
}
