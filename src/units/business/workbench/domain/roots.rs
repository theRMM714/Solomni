//! 目的：工作区的取值形状——寻址根、文件清单、区用量与文件名净化。
//! 管：`WorkRoots`（一次工作的共享区 / 会话树 / 各 agent 私有根 / 版本库目录）、`WorkFiles`、
//!   `AreaUsage` / `WorkUsage` 与 `safe_file_name`。
//! 不管：这些根怎么落盘（在 `workspace` 的端口实现）；怎么拼成工作面（在 `workspace` 的席位组装）。
//! 联动：`workspace` 的端口与 `versions`、`work`、呈现层只经 `workbench::api` 取这些值。

use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// 目的：投喂文件名净化（策略在 runtime）：只允许单个文件名，挡掉路径分隔符与上级跳转。
pub fn safe_file_name(name: &str) -> Result<String, String> {
    let n = name.trim();
    if n.is_empty() {
        return Err("文件名不能为空".to_string());
    }
    if n.contains('/') || n.contains('\\') || n == "." || n == ".." {
        return Err("文件名不能包含路径分隔符".to_string());
    }
    if n.chars().any(|c| c.is_control()) {
        return Err("文件名不能包含控制字符".to_string());
    }
    Ok(n.to_string())
}

/// 目的：工作区寻址根（由 `Workdirs` 端口如实给出；`workspace` 的 service 据此拼装工作面）。
#[derive(Debug, Clone)]
pub struct WorkRoots {
    /// 目的：本工作共享区。
    pub shared: PathBuf,
    /// 目的：本次工作的**会话树根**（`session/<顶层工作名>`）：各 agent 私沙与 `.work` 的父目录。
    /// 约束：会话目录内的布局事实由工作区单元给出，虚拟机档不再自己从 `shared.parent()` 反推。
    pub session_root: PathBuf,
    /// 目的：agent 实例名 → 私有沙箱目录。
    pub agents: BTreeMap<String, PathBuf>,
    /// 目的：版本库目录（`session/<工作名>/.work`）：提交记录、内容寻址对象与各 agent 的拉取基线。
    pub store: PathBuf,
}

/// 目的：一次工作的文件清单（给前端的 @ 菜单用）：相对路径，**一律用 / 分隔**，排序稳定。
#[derive(Debug, Clone, Default)]
pub struct WorkFiles {
    /// 目的：work/ 共享区下的文件。
    pub work: Vec<String>,
    /// 目的：agent 实例名 → 其私有沙箱下的文件。
    pub agents: BTreeMap<String, Vec<String>>,
}

/// 目的：一个区的用量：文件数 + 总字节（真实 stat，不是清单条数）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct AreaUsage {
    pub files: usize,
    pub bytes: u64,
}

/// 目的：一次工作的**工作区用量**（删除前如实交代）：总数 + 共享区/各 agent 沙箱的分项。
///    只数文件（目录不计），字节取文件真实大小。
#[derive(Debug, Clone, Default, Serialize)]
pub struct WorkUsage {
    /// 目的：全部文件数（共享区 + 各 agent 沙箱）。
    pub files: usize,
    /// 目的：全部总字节。
    pub bytes: u64,
    /// 目的：共享区 `work/`。
    pub work: AreaUsage,
    /// 目的：agent 实例名 → 它的私有沙箱。
    pub agents: BTreeMap<String, AreaUsage>,
}

impl WorkUsage {
    /// 目的：按分项汇总总数（调用方只填分项，避免两处各算一遍）。
    pub fn total(work: AreaUsage, agents: BTreeMap<String, AreaUsage>) -> WorkUsage {
        let mut files = work.files;
        let mut bytes = work.bytes;
        for a in agents.values() {
            files += a.files;
            bytes += a.bytes;
        }
        WorkUsage {
            files,
            bytes,
            work,
            agents,
        }
    }
}
