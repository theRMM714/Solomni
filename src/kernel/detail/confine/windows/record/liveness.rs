//! 目的：台账条目的归属与活性判定——谁挂上的、那个进程还在不在。
//! 管：进程创建时刻折算、当前进程归属、单条归属的活性与一组归属合并出的处置结论。
//! 不管：结论怎么用（release.rs 回收、reconcile.rs 对账各按自己的策略处置）。
//! 联动：归属字段 Owner 在 confine/ledger.rs；判不了时取保守结论——宁可漏回收，也不误撤在用的授权。

use super::super::super::Owner;
use super::{GrantEntry, GrantScope};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME};
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// FILETIME（自 1601 起的 100ns）折成 u64。
fn filetime_u64(ft: &FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
}

/// 目的：读一个进程的创建时刻（防 PID 复用）；进程不存在或读不到就返回 None。
fn process_start(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let (mut c, mut e, mut k, mut u) = (
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
        );
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        if ok == 0 {
            None
        } else {
            Some(filetime_u64(&c))
        }
    }
}

/// 目的：当前进程的归属（pid + 创建时刻，租约留空）——写台账时记进条目。
pub(crate) fn current_owner() -> Owner {
    let pid = std::process::id();
    Owner {
        pid,
        start: process_start(pid).unwrap_or(0),
        lease: String::new(),
    }
}

/// 归属的三种活性结论。
#[derive(Clone, Copy, PartialEq, Eq)]
enum Liveness {
    /// 进程还在（且创建时刻对得上）= 这条授权还有人用。
    Live,
    /// 进程已不在，或 pid 还在但创建时刻对不上（PID 被复用了）。
    Dead,
    /// 判不了（打不开、拒绝访问、创建时刻读不到）：保守处理，不回收。
    Unknown,
}

/// 目的：判断一条归属是否还活着；判不了时返回 Unknown（宁可漏回收，也不误回收在用的授权）。
fn owner_liveness(o: &Owner) -> Liveness {
    if o.pid == 0 || o.start == 0 {
        return Liveness::Unknown;
    }
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, o.pid);
        if h.is_null() {
            // “进程不存在”与“拒绝访问”是两回事：后者按判不了处理（保守）。
            const ERROR_ACCESS_DENIED: i32 = 5;
            let err = std::io::Error::last_os_error().raw_os_error().unwrap_or(0);
            return if err == ERROR_ACCESS_DENIED {
                Liveness::Unknown
            } else {
                Liveness::Dead
            };
        }
        let (mut c, mut e, mut k, mut u) = (
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
            std::mem::zeroed::<FILETIME>(),
        );
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        if ok == 0 {
            return Liveness::Unknown;
        }
        if filetime_u64(&c) == o.start {
            Liveness::Live
        } else {
            Liveness::Dead
        }
    }
}

/// 目的：一个条目按归属集合得出的处置结论。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum OwnersState {
    /// 目的：还有活归属——留着。
    Keep,
    /// 目的：归属全部已死（且至少有一条）——可回收。
    Reclaim,
    /// 目的：没有归属或判不了——报告后跳过，不主动回收。
    Unjudgeable,
}

/// 目的：按授权条目得出处置结论——机器级一律保留（与进程死活无关），按次才看归属集合。
/// 约束：机器级条目 owners 是空的，但这里必须**先按口径判 Keep**，不能靠"空归属 → 判不了"的副作用
///   碰巧留下来（那样审计时说不清是保留还是判不了）。
pub(crate) fn entry_state(g: &GrantEntry) -> OwnersState {
    match g.scope {
        GrantScope::Machine => OwnersState::Keep,
        GrantScope::Call => owners_state(&g.owners),
    }
}

/// 目的：按归属集合得出处置结论——任一条活则留，全都明确已死则回收，否则判不了。
pub(crate) fn owners_state(owners: &[Owner]) -> OwnersState {
    if owners.is_empty() {
        return OwnersState::Unjudgeable;
    }
    let mut unknown = false;
    for o in owners {
        match owner_liveness(o) {
            Liveness::Live => return OwnersState::Keep,
            Liveness::Unknown => unknown = true,
            Liveness::Dead => {}
        }
    }
    if unknown {
        OwnersState::Unjudgeable
    } else {
        OwnersState::Reclaim
    }
}
