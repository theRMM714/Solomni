//! 目的：围栏授权台账的跨平台形状与按条回收入口。
//! 管：Owner / ReconcileReport / Ledger 形状，对账、台账读取、按路径或 SID 精确还原。
//! 不管：平台 ACL 读写（在 windows/record/）、启动流程（在根）。
//! 联动：启动对账由 `src/main.rs` 与 `src/entry/` 调用；Windows 条目的读写与撤权在 `windows/record/`。

#[cfg(windows)]
use super::windows;
#[cfg(windows)]
use crate::kernel::api::FenceSpec;

/// 目的：启动期对账——按台账回收"归属明确已死"的陈旧授权；无法判定的报告后跳过（启动与 `--fence-reconcile` 共用）。
/// 约束：只有 Windows 写本机权限项；其它平台没有台账，如实返回空报告。
pub fn reconcile(home: &std::path::Path) -> ReconcileReport {
    #[cfg(windows)]
    {
        windows::reconcile(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        ReconcileReport {
            note: "本平台的围栏不留权限项，没有台账可对账".to_string(),
            ..Default::default()
        }
    }
}

/// 目的：孤儿授权清扫：按容器 SID 族在产品根内撤掉台账之外的残留 ACE（--fence-clean 用）。
///   只有 Windows 写本机 ACL，其它平台没有这一步（而不是"存在但空转"）。
#[cfg(windows)]
pub fn sweep_orphan_aces(root: &std::path::Path) -> Result<usize, String> {
    windows::sweep_orphan_aces(root)
}

/// 目的：台账条目的归属——哪个进程还需要这条授权（pid + 进程创建时刻，防 PID 复用）。
/// 约束：跨平台只承载数据；活性判定与写盘都在 Windows 后端（其它平台没有持久授权，也就没有归属）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Owner {
    /// 目的：归属的进程号（缺省 = 0，判不了，按"无法判定"处理，不主动回收）。
    #[serde(default)]
    pub pid: u32,
    /// 目的：进程创建时刻（Windows FILETIME，自 1601 起的 100ns；0 = 读不到，按"无法判定"处理）。
    #[serde(default)]
    pub start: u64,
    /// 目的：会话租约（会话 id；空 = 无会话，例如探针或未绑定租约的授权）。同一进程里同名 agent 的多个会话靠它区分。
    #[serde(default)]
    pub lease: String,
}

/// 目的：一次启动对账的结果（如实交代回收了什么、跳过了什么、哪里失败）。
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct ReconcileReport {
    /// 目的：一句如实说明（没有台账 / 本平台不留权限项）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub reclaimed_grants: usize,
    pub restored_snapshots: usize,
    pub deleted_profiles: usize,
    /// 目的：保留的条目数。
    pub kept: usize,
    /// 目的：没有归属或归属判不了、报告后跳过的条目数。
    pub skipped_unjudgeable: usize,
    /// 目的：本次回收前的台账备份（展示用；没有备份时为空）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub backup: Option<String>,
    /// 目的：没清掉的部分（失败保留供下次重试）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
    /// 目的：**台账之外**的本程序容器 profile（只报告，不自动删——没有归属证明，删了会误伤在跑实例）。
    /// 约束：按次 profile 之后数量随调用增长；处置入口是手动的 `--fence-clean`。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub orphan_profiles: Vec<String>,
}

impl ReconcileReport {
    /// 目的：这次对账有没有"值得说的"（真回收了、跳过了、或失败了）——没有就保持安静，不刷启动日志。
    pub fn has_activity(&self) -> bool {
        self.reclaimed_grants > 0
            || self.restored_snapshots > 0
            || self.deleted_profiles > 0
            || self.skipped_unjudgeable > 0
            || !self.errors.is_empty()
            || !self.orphan_profiles.is_empty()
    }

    /// 目的：台账外 profile 的一句提示（没有就空串）；只报告，不在这里删。
    fn orphan_note(&self) -> String {
        if self.orphan_profiles.is_empty() {
            String::new()
        } else {
            format!(
                "；台账外还有 {} 个容器 profile（不自动回收，用 --fence-clean 处理）",
                self.orphan_profiles.len()
            )
        }
    }

    /// 目的：一行如实摘要（启动打印与手动入口共用）。
    pub fn summary(&self) -> String {
        if !self.errors.is_empty() {
            let skipped = if self.skipped_unjudgeable > 0 {
                format!("；另有 {} 条无法判定已跳过", self.skipped_unjudgeable)
            } else {
                String::new()
            };
            return format!(
                "对账未完成：{}（已还原 {} 处、撤销 {} 条、删 {} 个 profile；台账保留供重试{}）{}",
                self.errors.join("；"),
                self.restored_snapshots,
                self.reclaimed_grants,
                self.deleted_profiles,
                skipped,
                self.orphan_note()
            );
        }
        if !self.note.is_empty() {
            return format!("{}{}", self.note, self.orphan_note());
        }
        if self.reclaimed_grants == 0 && self.restored_snapshots == 0 && self.deleted_profiles == 0
        {
            let base = if self.skipped_unjudgeable > 0 {
                format!(
                    "没有可回收的陈旧授权；{} 条无法判定已跳过（用 --fence-clean 处置）",
                    self.skipped_unjudgeable
                )
            } else {
                "没有陈旧授权，无需回收".to_string()
            };
            return format!("{}{}", base, self.orphan_note());
        }
        format!(
            "已回收陈旧授权：还原快照 {} 处、撤销授权 {} 条、删除 profile {} 个；保留 {} 条{}{}",
            self.restored_snapshots,
            self.reclaimed_grants,
            self.deleted_profiles,
            self.kept,
            if self.skipped_unjudgeable > 0 {
                format!("（另有 {} 条无法判定已跳过）", self.skipped_unjudgeable)
            } else {
                String::new()
            },
            self.orphan_note()
        )
    }
}

/// 目的：台账现值里的**一条**（机器可读，与平台无关的形状）：哪一类、谁、哪个路径、什么权限、什么时候记的。
/// 约束：present = 这一条现在还在不在（路径在不在、ACE 还在不在、profile 还在不在）；
///   owners = 还需要这条授权的进程；ace_sids = 该路径上**现在**看得到的显式包 SID 允许 ACE；
///   notes = 读不到 DACL、认不出布局、路径不在这类如实记下的事实。
#[derive(Debug, serde::Serialize)]
pub struct LedgerEntry {
    /// 目的：这一条属于哪一类——snapshot（根内快照）/ grant（授权）/ profile（容器 profile）。
    pub kind: &'static str,
    /// 目的：授给谁（只有 grant 有；其余为空串）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub sid: String,
    /// 目的：哪个路径（profile 条目里是 profile 名，其余是路径）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub path: String,
    /// 目的：权限位（只有 grant 有）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rights: Option<u32>,
    /// 目的：记录时刻（Unix 秒）。
    pub at: u64,
    /// 目的：这一条现在还在不在。
    pub present: bool,
    /// 目的：还需要这条授权的归属（快照没有归属：它的去留由该路径上的授权条目派生）。
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub owners: Vec<Owner>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub ace_sids: Vec<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<String>,
}

/// 目的：台账现值（机器可读的数字面）：条目清单 + 一句如实说明。
#[derive(Debug, serde::Serialize)]
pub struct Ledger {
    /// 目的：一句如实说明（没有台账 / 本平台不留权限项）。
    #[serde(skip_serializing_if = "String::is_empty")]
    pub note: String,
    pub entries: Vec<LedgerEntry>,
}

/// 目的：台账现值（清单 + "当前实际 ACE 与台账对不对得上"的差异）。
/// 约束：只有 Windows 写本机权限项，所以台账只在 Windows 上有内容；其它平台如实说"本平台不留权限项"。
pub fn ledger(home: &std::path::Path) -> Ledger {
    #[cfg(windows)]
    {
        windows::catalog(home)
    }
    #[cfg(not(windows))]
    {
        let _ = home;
        Ledger {
            note: "本平台的围栏不留权限项，没有台账".to_string(),
            entries: Vec::new(),
        }
    }
}

/// 目的：按路径**只还原一条**快照（其余条目与整份 DACL 不受影响）。
/// 错误：台账里没有该路径、路径不在了、写回被拒都如实返回；失败时台账不改（供重试）。
pub fn restore_one(home: &std::path::Path, path: &std::path::Path) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::restore_one(home, path)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, path);
        Err("本平台的围栏不留权限项，没有可还原的快照".to_string())
    }
}

/// 目的：按 **SID + 路径**只撤一条授权（其余条目不受影响）；**不在台账里也照撤**（台账外残留走这条）。
/// 错误：路径不在了、SID 不合法、写撤权后的 DACL 被拒都如实返回；失败时台账不改动。
pub fn revoke_grant(
    home: &std::path::Path,
    sid: &str,
    path: &std::path::Path,
) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::revoke_grant(home, sid, path)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, sid, path);
        Err("本平台的围栏不留权限项，没有可撤销的授权".to_string())
    }
}

/// 目的：按名**只删一个**容器 profile（连该容器的存储一起删）。
/// 错误：删除被拒时如实返回；失败时台账不改动。
pub fn remove_profile_one(home: &std::path::Path, name: &str) -> Result<String, String> {
    #[cfg(windows)]
    {
        windows::remove_profile_one(home, name)
    }
    #[cfg(not(windows))]
    {
        let _ = (home, name);
        Err("本平台没有容器 profile".to_string())
    }
}

/// 目的：撤销一次调用的围栏授权（Windows 的按次释放用；其它平台没有持久授权这一步）。
/// 约束：只在 Windows 编译——非 Windows 没有任何调用方，留着就是死代码。
#[cfg(windows)]
pub fn release_fence(spec: &FenceSpec, home: &std::path::Path) -> Result<(), String> {
    windows::release_fence(spec, home)
}
