//! 一次工具执行的围栏（纯数据）：由该 agent 的沙箱与执行档位派生，机制在 adapters（confine）。
//! 权限方位类以 agent 为界：可读可写的只有本次工作的共享区与该 agent 的私有沙箱，
//! 成员模块目录随它（读写，回执里如实提示）；其余一律不可达——这是**策略**，装在哪个平台用什么机制由适配层定。
//! `ro` 是**用户显式授权**的只读根（`.home/settings.yaml` 的 `fence_read`）：只读、不继承写，
//! 默认空 = 一个都不放行（与 `fence_write` 同一套哲学：没经用户同意就不动本机任何权限项）。
//! 本机档与虚拟机档共用这份围栏：虚拟机档的 guest 内视图由装配阶段按同一批根组装。

use crate::capabilities::workspace::api::Sandbox;
use crate::kernel::api::Ask;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 守门进程要执行的命令与其环境上下文。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FenceSpec {
    /// 该 agent 的实例名（日志与审计用）。
    pub agent: String,
    /// 可读可写的根：本次工作共享区（若这一席可写）+ 该 agent 私有沙箱 + 已授权的模块目录 + 模块 `userdata/`。
    pub rw: Vec<PathBuf>,
    /// 只读的根：**用户显式授权**的额外可达范围（默认空）。
    /// 只读位由各平台机制落实（Landlock 只读位 / seatbelt `file-read*` / Windows `RIGHTS_RO`），
    /// 且**必须授给该 agent 自己的容器身份**，不能像解释器基线那样授给共享组（那等于把用户数据开放给机器上任意容器）。
    #[serde(default)]
    pub ro: Vec<PathBuf>,
    /// 只读**子树**（递归可读 + 可列目录）：模块目录默认只读时进这里。
    /// 与 `ro` 分开的理由：Windows 上用户授权的 `ro` 不递归（用户可能授很大的目录），
    /// 而模块目录必须递归可读（工具脚本就在目录里），两者落成不同的 ACL。
    #[serde(default)]
    pub ro_tree: Vec<PathBuf>,
    /// 该 agent 私有沙箱：工具进程 HOME / TEMP 的落点（空 = 退回 cwd）。
    #[serde(default)]
    pub private: PathBuf,
    /// 工具进程的工作目录（它所属模块的根目录）。
    pub cwd: PathBuf,
    /// 是否放行出站网络（默认否）。
    pub net: bool,
}

impl FenceSpec {
    /// 从该 agent 的沙箱派生（模块目录按模块 id 升序，顺序稳定；同一模块不会同属两个 agent）。
    pub fn from_sandbox(sb: &Sandbox, net: bool) -> FenceSpec {
        // 共享主副本只有在**这一席可写**时才进 rw：agent 会话默认只读，
        // 于是模块外部工具进程也读不到 / 写不了未拉取进沙箱的主副本内容。
        let mut rw = Vec::new();
        let mut ro_tree = Vec::new();
        if sb.shared_writable {
            rw.push(sb.shared.clone());
        }
        rw.push(sb.private.clone());
        // 模块目录默认**只读**：只有 module_write 命中该模块才整块可写。
        // <module>/userdata 派不派，看工作区扫描注入的事实（domain 不读盘）。
        for (id, root) in &sb.modules {
            if sb.permissions.module_write_ok(id) {
                rw.push(root.clone());
            } else {
                ro_tree.push(root.clone());
                if sb.modules_with_userdata.contains(id) {
                    rw.push(root.join("userdata"));
                }
            }
        }
        rw.sort();
        rw.dedup();
        ro_tree.sort();
        ro_tree.dedup();
        FenceSpec {
            agent: sb.agent.clone(),
            rw,
            ro: Vec::new(),
            ro_tree,
            private: sb.private.clone(),
            cwd: PathBuf::new(),
            net,
        }
    }

    /// 挂上用户显式授权的只读根（策略层只带事实；只读位怎么落由适配层定）。
    pub fn with_read_only(mut self, ro: Vec<PathBuf>) -> FenceSpec {
        self.ro = ro;
        self
    }

    /// 目的：**无会话**（人直接跑一个模块工具）的围栏——模块目录递归只读 + 它自己的 `userdata/`（事实说有才）可写，外加用户指定的工作目录（缺省 = `userdata/`，没有则退回模块根）。
    /// 参数：has_userdata 是工作区扫描给出的"该模块有没有 `userdata/`"事实（domain 不读盘）。
    /// 约束：与 agent 会话同一条围栏口径——不装机制时只留进程树与环境白名单，如实降级。
    pub fn standalone(
        module_root: &Path,
        work_root: Option<&Path>,
        has_userdata: bool,
    ) -> FenceSpec {
        let userdata = module_root.join("userdata");
        // 缺省工作目录 = 模块的 userdata（事实说有才用）；否则退回 cwd（模块根），不派不存在的落点。
        let private = match work_root {
            Some(p) => p.to_path_buf(),
            None if has_userdata => userdata.clone(),
            None => PathBuf::new(),
        };
        let mut rw: Vec<PathBuf> = Vec::new();
        if !private.as_os_str().is_empty() {
            rw.push(private.clone());
        }
        if has_userdata && private != userdata {
            rw.push(userdata);
        }
        rw.sort();
        rw.dedup();
        FenceSpec {
            agent: "user".to_string(),
            rw,
            ro: Vec::new(),
            ro_tree: vec![module_root.to_path_buf()],
            private,
            cwd: module_root.to_path_buf(),
            net: false,
        }
    }

    /// 把这个围栏的工作目录设成某个模块的根（该模块的工具就在这里跑）。
    pub fn at(&self, module_root: &std::path::Path) -> FenceSpec {
        let mut out = self.clone();
        out.cwd = module_root.to_path_buf();
        out
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
    /// 用户显式授权的只读根（`fence_read`）。
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

/// 目的：一个落点授不上的如实结论：**哪一环** + 哪个目录 + 缺什么前提。
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
