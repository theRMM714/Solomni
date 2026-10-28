//! 目的：一次外部进程执行的**围栏描述符**（纯数据）与平台无关的落点判据。
//! 管：`FenceSpec`（可达范围 / 网络 / 租约）、`FencePart` / `FenceBlocked` / `fence_ask`（必要落点授不上时的裁决材料）。
//! 不管：从 `Sandbox` / 模块权限**派生**围栏（那是上层策略，见 `capabilities/tools/domain/fence.rs`）；
//!   平台机制（Landlock / seatbelt / AppContainer）在 `detail/confine`。
//! 联动：由 `src/kernel/api.rs` 重导出；守门进程协议与工具执行层消费它。

use crate::kernel::api::Ask;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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

/// 目的：守门进程要执行的命令与其环境上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FenceSpec {
    /// 目的：该 agent 的实例名（日志与审计用）。
    pub agent: String,
    /// 目的：本席所属会话的租约（会话 id；空 = 无会话，如独立模块工具）。
    ///   同名 agent 的多个并发会话共用同一容器 SID 与同一批授权，释放时靠它区分"还有哪个会话在用"。
    #[serde(default)]
    pub lease: String,
    /// 目的：可读可写的根——本次工作共享区（若这一席可写）+ 该 agent 私有沙箱 + 模块自己的 `userdata/`
    ///   （后者只随模块自身的工具 / 服务 / 构建围栏出现；agent 会话的模块目录始终在 `ro_tree`）。
    pub rw: Vec<PathBuf>,
    /// 目的：只读的根——**用户显式授权**的额外可达范围（默认空）。
    ///   只读位由各平台机制落实（Landlock 只读位 / seatbelt `file-read*` / Windows `RIGHTS_RO`），
    ///   且**必须授给该 agent 自己的容器身份**，不能像解释器基线那样授给共享组。
    #[serde(default)]
    pub ro: Vec<PathBuf>,
    /// 目的：只读**子树**（递归可读 + 可列目录）：模块目录对 agent 只读时进这里。
    ///   与 `ro` 分开的理由：Windows 上用户授权的 `ro` 不递归，而模块目录必须递归可读。
    #[serde(default)]
    pub ro_tree: Vec<PathBuf>,
    /// 目的：该 agent 私有沙箱——工具进程 HOME / TEMP 的落点（空 = 退回 cwd）。
    #[serde(default)]
    pub private: PathBuf,
    /// 目的：工具进程的工作目录（它所属模块的根目录）。
    pub cwd: PathBuf,
    /// 目的：这次执行是否放行出站网络（来自全局 `sandbox.policy.network`，不是会话字段）。
    ///   平台落点：Windows 授 `internetClient`、macOS 放行 `network-outbound`、Linux `net=false` 用 seccomp
    ///   拒绝非本地 socket（`net=true` 不装网络过滤器，不限制入站/绑定）；虚拟机档由 QEMU 网卡决定。
    /// 约束：Windows 只放行公网出站，不授 private network / loopback；Linux 的平台差异见 docs/kernel/README.md。
    pub net: bool,
}

impl FenceSpec {
    /// 目的：把这个围栏的工作目录设成某个模块的根（该模块的工具就在这里跑）。
    pub fn at(&self, module_root: &Path) -> FenceSpec {
        let mut out = self.clone();
        out.cwd = module_root.to_path_buf();
        out
    }

    /// 目的：该 agent 的私有沙箱（没有就退回工作目录）——环境里的 HOME / TEMP 落点。
    pub fn private_or_cwd(&self) -> PathBuf {
        if self.private.as_os_str().is_empty() {
            self.cwd.clone()
        } else {
            self.private.clone()
        }
    }
}

/// 目的：一次围栏执行的**落点环节**——授权装不上的时候，用户与模型都要知道缺的是哪一环。
/// 约束：**必要 / 可选的判据只在这一处**（`necessary`）：必要落点缺了这次命令在容器里起不来，
///   或围栏本身不成立；可选落点缺了命令照跑，只是可达范围小一点。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub enum FencePart {
    /// 解释器安装目录（基线，授给共享包组）：容器里连解释器都起不来。
    Interpreter,
    /// 模块目录（只读子树或可写叶子）：工具脚本与它的依赖都在这里。
    Module,
    /// 工具进程的工作目录（模块根）。
    Cwd,
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
}

impl FencePart {
    /// 目的：这一环**必要**吗——缺了这次命令在容器里起不来，或这次执行根本做不了该做的事。
    /// 约束：判据只有这一处（枚举里这两条就是可选的）；可选落点授不上只记事实（见 `FencePrep`），
    ///   不牵动这次执行。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn necessary(&self) -> bool {
        !matches!(self, FencePart::AuthorizedRead | FencePart::Parent)
    }

    /// 目的：给人看的环节名（回执、裁决卡与警告都用它）。
    pub fn label(&self) -> &'static str {
        match self {
            FencePart::Interpreter => "解释器安装目录",
            FencePart::Module => "模块目录",
            FencePart::Cwd => "工具进程的工作目录",
            FencePart::Sandbox => "该 agent 的私有沙箱",
            FencePart::DataBoundary => "数据边界（这次执行要读写的根）",
            FencePart::AuthorizedRead => "用户授权的只读根",
            FencePart::Parent => "数据边界的父目录",
            FencePart::ContainerIdentity => "容器身份",
            FencePart::Ledger => "授权台账",
        }
    }

    /// 目的：这一环**怎么补**（一句可操作的话；回执与裁决卡都用它）。
    pub fn fix(&self) -> &'static str {
        match self {
            FencePart::Interpreter => {
                "把解释器装在你自己拥有的目录里（属主不是你就改不动它的权限项），或请管理员放行那个安装目录"
            }
            FencePart::Module | FencePart::Cwd => {
                "把模块目录放进你自己拥有的位置（工作区里的模块目录属主就是你）"
            }
            FencePart::Sandbox => "让这次工作的会话目录落在你自己拥有的位置（默认就在工作区里）",
            FencePart::DataBoundary => "让这次工作的目录落在你自己拥有的位置（默认就在工作区里）",
            FencePart::ContainerIdentity => "确认本机能建 AppContainer profile（受限会话里建不起来）",
            FencePart::Ledger => "确认产品私有区 .home/ 可写（授权台账要落在那里）",
            FencePart::AuthorizedRead | FencePart::Parent => {
                "这一次不影响围栏成立；下次想让它可达再补"
            }
        }
    }
}

/// 目的：一个落点授不上的如实结论——**哪一环** + 哪个目录 + 缺什么前提。
/// 约束：它是"必要落点授不上"这条路的唯一材料——裁决卡、回执与停会话警告都从它派生。
#[derive(Debug, Clone, PartialEq, Eq)]
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub struct FenceBlocked {
    /// 目的：哪一环（`necessary()` 为真）。
    pub part: FencePart,
    /// 目的：授不上的那个目录；没有具体目录的一环（容器身份 / 台账）= 空。
    pub path: PathBuf,
    /// 目的：缺什么前提（机制给的原话，例如写 DACL 的失败原因）。
    pub why: String,
}

impl FenceBlocked {
    /// 目的：这一环的一句话（哪一环、哪个目录、缺什么前提、怎么补）——给用户看的原话。
    // 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
    #[allow(dead_code)]
    pub fn line(&self) -> String {
        let where_ = if self.path.as_os_str().is_empty() {
            String::new()
        } else {
            format!("（{}）", self.path.display())
        };
        format!(
            "{}{} 授不上：{}；怎么补：{}",
            self.part.label(),
            where_,
            self.why,
            self.part.fix()
        )
    }
}

/// 目的：必要落点授不上时给用户的选项 id——**本轮无围栏跑一次**（这一次调用按无围栏执行）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub const OPT_FENCE_UNFENCED: &str = "fence_unfenced_once";
/// 目的：必要落点授不上时给用户的选项 id——**放弃这次调用**（不执行；没有回答也是它）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
#[allow(dead_code)]
pub const OPT_FENCE_ABORT: &str = "fence_abort";

/// 目的：把"必要落点授不上"变成一条裁决（消息三段 + 选项集）——问什么、几个选项由工具层自己定。
/// 参数：`name` = 谁撞上的（agent 实例名，写进信封）；`unfenced_possible` = 无围栏跑这一次
///   **真能不能跑起来**（工作目录在不在这类事实，由调用方读盘后喂进来——domain 不读盘）。
/// 返回：`None` = **构不出可用选项**（除"放弃"外没有一条真能执行的）：调用方**不发起裁决**，
///   改为停掉这个会话 + 落一条警告（契约禁止置灰）。
// 这套机制只在 Windows 的容器围栏里用（unix 没有 prepare_fence 这一步）：unix 侧无使用点，如实放行死代码。
/// 目的：全局沙箱策略对机制的要求（与 `settings.yaml` 的 `sandbox.policy.enforce` 同义）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SandboxRequirements {
    pub filesystem: bool,
    pub network: bool,
    pub process_tree: bool,
    pub env_whitelist: bool,
}

/// 目的：本进程的沙箱模式——全局要求 + 本机能力 + 是否已同意本进程降级。
/// 约束：降级只活在内存里，不落盘；重启后重新评估、重新询问。
#[derive(Debug)]
pub struct SandboxMode {
    pub requirements: SandboxRequirements,
    pub filesystem_ok: bool,
    pub network_ok: bool,
    pub tree_ok: bool,
    pub env_ok: bool,
    /// 目的：`on_unavailable=ask` 时为真；`refuse` 时为假（不允许通过问答降级）。
    pub ask_allowed: bool,
    degraded: std::sync::atomic::AtomicBool,
}

impl SandboxMode {
    /// 目的：按全局要求与本机能力构造；初始未降级。
    pub fn new(
        requirements: SandboxRequirements,
        filesystem_ok: bool,
        network_ok: bool,
        tree_ok: bool,
        env_ok: bool,
        ask_allowed: bool,
    ) -> SandboxMode {
        SandboxMode {
            requirements,
            filesystem_ok,
            network_ok,
            tree_ok,
            env_ok,
            ask_allowed,
            degraded: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// 目的：测试替身/无策略场景——什么都不要求，机制视为可用。
    pub fn permissive() -> SandboxMode {
        SandboxMode::new(
            SandboxRequirements::default(),
            true,
            true,
            true,
            true,
            false,
        )
    }

    pub fn degraded(&self) -> bool {
        self.degraded.load(std::sync::atomic::Ordering::Relaxed)
    }

    pub fn set_degraded(&self, value: bool) {
        self.degraded
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }

    /// 目的：启动期缺哪些必要机制（不看 `prepared`，那是执行期事实）。
    pub fn missing(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.requirements.filesystem && !self.filesystem_ok {
            out.push("文件系统围栏");
        }
        if self.requirements.network && !self.network_ok {
            out.push("网络围栏");
        }
        if self.requirements.process_tree && !self.tree_ok {
            out.push("进程树围栏");
        }
        if self.requirements.env_whitelist && !self.env_ok {
            out.push("环境白名单");
        }
        out
    }

    /// 目的：执行前检查——要求强制但机制（或 Windows 的本机授权）不可用时拒绝；已同意降级则放行。
    /// 约束：`spec.net=true` 时不需要“网络围栏”来拦出站；只有要求断网时才检查。
    pub fn check(&self, spec: &FenceSpec, prepared: bool) -> Result<(), String> {
        let paths_effective = !cfg!(windows) || prepared;
        let mut missing: Vec<&'static str> = Vec::new();
        if self.requirements.filesystem && (!self.filesystem_ok || !paths_effective) {
            missing.push("文件系统围栏");
        }
        if self.requirements.network && !spec.net && (!self.network_ok || !paths_effective) {
            missing.push("网络围栏");
        }
        if self.requirements.process_tree && !self.tree_ok {
            missing.push("进程树围栏");
        }
        if self.requirements.env_whitelist && !self.env_ok {
            missing.push("环境白名单");
        }
        if missing.is_empty() || self.degraded() {
            return Ok(());
        }
        Err(format!(
            "沙箱要求{}，但本机机制不可用或未获授权；按全局策略拒绝执行（启动时可选择本进程降级，重启后重新询问）",
            missing.join("、")
        ))
    }
}

#[allow(dead_code)]
pub fn fence_ask(block: &FenceBlocked, name: &str, unfenced_possible: bool) -> Option<Ask> {
    if !unfenced_possible {
        return None;
    }
    Some(Ask {
        role: "tools".to_string(),
        name: name.to_string(),
        title: format!("围栏的{}装不上，这次调用怎么跑？", block.part.label()),
        body: "按规则不许悄悄按无围栏执行。要么这一轮破例跑一次（这次就没有容器那层强制），要么放弃这次调用。"
            .to_string(),
        detail: block.line(),
        options: vec![
            (
                OPT_FENCE_UNFENCED.to_string(),
                "本轮无围栏跑一次".to_string(),
            ),
            (OPT_FENCE_ABORT.to_string(), "放弃这次调用".to_string()),
        ],
        // 没人答时按发起方自己的选项收场：当我选了"放弃这次调用"（= 不执行，fail-closed）。
        on_unanswered: Some(OPT_FENCE_ABORT.to_string()),
    })
}
