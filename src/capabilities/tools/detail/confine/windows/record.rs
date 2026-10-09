//! 目的：授权记录与撤销——ACL 改动在进程退出后仍留在盘上，所以给谁授了哪些路径要落一份台账。
//! 管：台账的读写（快照 / 根外授权摘要 / 容器 profile）、精确撤销、孤儿清扫，以及按条处置（列清单、
//!   还原一条、撤一条、删一个 profile）。
//! 不管：一次工具执行的围栏怎么装（windows/mod.rs 的 prepare_fence 与守门进程）；命令行怎么解析（src/guard/mod.rs）。
//! 联动：ACE 的读法只有一处（acl.rs 的 acl_scan / ace_parts / revoke/restore）；两条入口（--fence-clean 整体收尾、
//!   按条处置）共用同一份台账与同一套读法。

use crate::capabilities::tools::api::FenceSpec;
use std::ffi::c_void;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, FILETIME};
use windows_sys::Win32::Security::Authorization::{ConvertSidToStringSidW, ConvertStringSidToSidW};
use windows_sys::Win32::Security::Isolation::DeleteAppContainerProfile;
use windows_sys::Win32::Security::PSID;
use windows_sys::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
};

/// 文件对象（SetNamedSecurityInfoW / GetNamedSecurityInfoW 的对象类型）。
use super::super::{Ledger, LedgerEntry, Owner};
use super::*;

/// 目的：台账里一条快照（路径 + 记录时刻 + 原始安全描述符）。
/// 约束：时间戳在**新增时**记一次，之后只读——它是“这条是什么时候挂上的”这一事实。
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct SnapshotEntry {
    pub(crate) path: String,
    /// 目的：记录时刻（Unix 秒）。
    #[serde(default)]
    pub(crate) at: u64,
    pub(crate) bytes: Vec<u8>,
}

/// 目的：台账里一条授权（授给谁、在哪、哪些权限位）+ 记录时刻 + **归属集合**。
/// 约束：根内与根外都记这一条——根内另有快照存原始安全描述符；归属集合空才撤 ACE / 还原快照。
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct GrantEntry {
    pub(crate) sid: String,
    pub(crate) path: String,
    pub(crate) rights: u32,
    #[serde(default)]
    pub(crate) at: u64,
    /// 目的：还需要这条授权的进程（空 = 缺归属，按"无法判定"处理，不主动回收）。
    #[serde(default)]
    pub(crate) owners: Vec<Owner>,
}

/// 目的：台账里一个我们建过的容器 profile（名字 + 记录时刻 + 归属集合）。
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct ProfileEntry {
    pub(crate) name: String,
    #[serde(default)]
    pub(crate) at: u64,
    /// 目的：还需要这个 profile 的进程（空 = 缺归属，按"无法判定"处理，不主动回收）。
    #[serde(default)]
    pub(crate) owners: Vec<Owner>,
}

/// 目的：授权台账——记下我们给谁、在哪些路径上写了权限、建过哪些容器 profile。
/// 约束：落在产品私有区 .home/fence-grants.json（数据不出工作区）；收尾与按条处置都按它做。
#[derive(serde::Serialize, serde::Deserialize, Default)]
pub(crate) struct GrantRecord {
    #[serde(default)]
    pub(crate) profiles: Vec<ProfileEntry>,
    #[serde(default)]
    pub(crate) grants: Vec<GrantEntry>,
    #[serde(default)]
    pub(crate) snapshots: Vec<SnapshotEntry>,
}

impl GrantRecord {
    /// 目的：一行都不占的空台账（“没在本机写过权限项”的判据）。
    pub(crate) fn is_empty(&self) -> bool {
        self.profiles.is_empty() && self.grants.is_empty() && self.snapshots.is_empty()
    }

    /// 目的：台账空到一行都不剩就删掉台账文件（不留“已办”的残条）。
    pub(crate) fn drop_if_empty(&self, home: &Path) {
        if !self.is_empty() {
            return;
        }
        match std::fs::remove_file(record_path(home)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {}
        }
    }
}

/// 目的：记录时刻（Unix 秒）——台账条目自带时间，按条处置与对账都看得到“这条是什么时候挂上的”。
pub(crate) fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

pub(crate) fn record_path(home: &Path) -> PathBuf {
    home.join("fence-grants.json")
}

/// 台账写事务的跨进程锁文件名（与台账同目录，独立于台账本身）。
fn lock_path(home: &Path) -> PathBuf {
    home.join("fence-grants.lock")
}

/// 进程内串行：同一进程里可并发的工具调用会各写一次台账（见 collab 的并发批次），
/// 单靠跨进程文件锁挡不住同进程内的竞争，所以两层都要。
static LEDGER_MUTEX: Mutex<()> = Mutex::new(());

/// 目的：一次台账事务的锁（进程内互斥 + 跨进程文件锁）；drop 即释放。
pub(crate) struct LedgerGuard {
    _proc: MutexGuard<'static, ()>,
    _file: std::fs::File,
}

/// 目的：拿住台账事务锁，覆盖一次完整的“读—改—写”（调用方拿到后自己 load/save）。
/// 约束：有界等待（不无限阻塞）；拿不到就如实返回原因，调用方按“台账这一环走不动”处置。
pub(crate) fn lock_ledger(home: &Path) -> Result<LedgerGuard, String> {
    let proc = LEDGER_MUTEX
        .lock()
        .map_err(|_| "台账进程内锁已被毒化".to_string())?;
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let path = lock_path(home);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("打开台账锁失败（{}）：{}", path.display(), e))?;
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        match file.try_lock() {
            Ok(()) => {
                return Ok(LedgerGuard {
                    _proc: proc,
                    _file: file,
                })
            }
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(
                        "台账被另一个进程占用，等待超时（稍后重试或跑 --fence-reconcile）"
                            .to_string(),
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::Error(e)) => {
                return Err(format!("台账加锁失败：{}", e));
            }
        }
    }
}

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

/// 目的：按 SID 字符串撤一条 ACE（先转 PSID，用完即释放）。
fn revoke_one_text(sid_text: &str, path: &Path) -> Result<(), String> {
    let sid = sid_from_string(sid_text)?;
    let r = revoke_one(sid, path, true);
    free_sid(sid);
    r
}

pub(crate) fn load_record(home: &Path) -> GrantRecord {
    std::fs::read_to_string(record_path(home))
        .ok()
        .and_then(|t| serde_json::from_str::<GrantRecord>(&t).ok())
        .unwrap_or_default()
}

pub(crate) fn save_record(home: &Path, rec: &GrantRecord) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|e| format!("建私有区失败：{}", e))?;
    let text = serde_json::to_string_pretty(rec).map_err(|e| e.to_string())?;
    std::fs::write(record_path(home), text).map_err(|e| e.to_string())
}

/// 目的：记下我们建过这个容器 profile（与 prepare_fence 共用同一份台账），供收尾精确回收。
pub(crate) fn record_profile(home: &Path, name: &str) {
    let _lock = match lock_ledger(home) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "[围栏] 容器 profile 台账加锁失败（影响收尾的精确回收）：{}",
                e
            );
            return;
        }
    };
    let mut rec = load_record(home);
    if rec.profiles.iter().any(|p| p.name == name) {
        return;
    }
    rec.profiles.push(ProfileEntry {
        name: name.to_string(),
        at: now_secs(),
        owners: vec![current_owner()],
    });
    if let Err(e) = save_record(home, &rec) {
        eprintln!(
            "[围栏] 容器 profile 台账落盘失败（影响收尾的精确回收）：{}",
            e
        );
    }
}

/// 本程序建过的容器 profile 名（Windows 把包目录名转小写，按前缀不区分大小写筛）。
/// profile 可能来自没有台账的路径（探针、夹具的台账被删、旧版本），所以按名字前缀扫。
fn our_profile_names() -> Result<Vec<String>, String> {
    let root = match std::env::var_os("LOCALAPPDATA") {
        Some(v) => PathBuf::from(v).join("Packages"),
        None => return Err("取不到 LOCALAPPDATA（容器 profile 的存储根）".to_string()),
    };
    let entries = match std::fs::read_dir(&root) {
        Ok(e) => e,
        // 没有 Packages 目录 = 本机没有容器 profile。
        Err(_) => return Ok(Vec::new()),
    };
    Ok(entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .filter(|n| is_our_profile(n))
        .collect())
}

/// 这个具名 profile 还在不在（列清单时如实报“挂着的还在不在”）。
fn profile_exists(name: &str) -> bool {
    our_profile_names()
        .map(|names| names.iter().any(|n| n.eq_ignore_ascii_case(name)))
        .unwrap_or(false)
}

/// 目的：扫掉本程序建过的整族容器 profile。
/// 约束：台账只记“我们知道写过什么”，而 profile 可能来自没有台账的路径，所以按独有的名字前缀扫；
///   DeleteAppContainerProfile 连该容器的存储一起删；返回扫掉的个数。
pub fn sweep_profiles() -> Result<usize, String> {
    let mut deleted = 0usize;
    for name in our_profile_names()? {
        if delete_profile(&name) {
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// 目的：删掉一个具名的容器 profile（连该容器的存储一起删）——整族清扫与测试的定向清理共用。
pub(crate) fn delete_profile(name: &str) -> bool {
    let wide: Vec<u16> = std::ffi::OsStr::new(name)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    (unsafe { DeleteAppContainerProfile(wide.as_ptr()) }) >= 0
}

/// 目的：孤儿授权清扫——在产品根内找台账之外、我们写过的显式 ACE 并连树撤掉。
/// 约束：台账是精确回收的依据，但账会断（夹具/临时 home 被删、进程被杀、旧版本没记账），
///   断了账不代表没有残留。反向兜底两条：本程序建过的容器 profile 名派生 SID；以及任何显式、
///   非继承、SID 形如 S-1-15-2-* 的允许 ACE（排除 ALL APPLICATION PACKAGES 与 ALL RESTRICTED
///   两个基线）——profile 已删或从没建过的残留也能回收。自产品根向下在**最上层**命中处连树撤掉
///   （授权只会以某个目录为根整树写下去）。只扫产品根内：根外落点（解释器目录、根外只读根）仍
///   只由台账管。**必须在 sweep_profiles 之前调用**：profile 删了就派生不出 SID 了。
pub fn sweep_orphan_aces(root: &Path) -> Result<usize, String> {
    let mut sids: Vec<PSID> = Vec::new();
    for name in our_profile_names()? {
        match container_sid(&name) {
            Ok(s) => sids.push(s),
            Err(e) => eprintln!("[围栏] {}：该容器的孤儿扫描跳过", e),
        }
    }
    // 一个 profile 都派生不出 SID 时也要扫：台账外的残留可能来自根本没建过 profile 的路径。
    let mut swept = 0usize;
    let mut errors: Vec<String> = Vec::new();
    sweep_dir_down(root, &sids, &mut swept, &mut errors);
    for s in &sids {
        free_sid(*s);
    }
    if errors.is_empty() {
        Ok(swept)
    } else {
        Err(errors.join("；"))
    }
}

/// 自上而下扫一个目录：最上层命中就整树撤、不再下钻；没命中才继续走子目录。
/// 读不到 DACL 或撤权失败都记进 errors 并继续扫别处，最后如实返回 Err（清不掉不能当清干净）。
fn sweep_dir_down(dir: &Path, sids: &[PSID], swept: &mut usize, errors: &mut Vec<String>) {
    let known = sids.iter().any(|s| has_any_ace_for(*s, dir));
    let orphans = match orphan_package_aces(dir) {
        Ok(list) => list,
        Err(e) => {
            errors.push(e);
            Vec::new()
        }
    };
    if known || !orphans.is_empty() {
        for s in sids {
            if let Err(e) = revoke_one(*s, dir, true) {
                errors.push(format!("孤儿撤权未完成（{}）：{}", dir.display(), e));
            }
        }
        for text in orphans {
            match sid_from_string(&text) {
                Ok(sid) => {
                    if let Err(e) = revoke_one(sid, dir, true) {
                        errors.push(format!(
                            "孤儿撤权未完成（{}，{}）：{}",
                            dir.display(),
                            text,
                            e
                        ));
                    }
                    free_sid(sid);
                }
                Err(e) => errors.push(e),
            }
        }
        *swept += 1;
        return;
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        // 读不了的目录如实跳过：里面就算有残留也不再往下猜。
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let meta = match std::fs::symlink_metadata(entry.path()) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if !meta.is_dir() {
            continue;
        }
        // 重解析点（联接/符号链接）不跟：不走出产品根，也不进环。
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
        if meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            continue;
        }
        sweep_dir_down(&entry.path(), sids, swept, errors);
    }
}

/// 目的：把容器 profile 名先记进台账落盘（守门进程真正建 profile 之前，名字先有主），并记下归属。
/// 错误：台账落盘失败时返回原因。
pub(crate) fn journal_add_profile(
    home: &Path,
    rec: &mut GrantRecord,
    name: &str,
    owner: Owner,
) -> Result<(), String> {
    if let Some(p) = rec.profiles.iter_mut().find(|p| p.name == name) {
        if p.owners.contains(&owner) {
            return Ok(());
        }
        p.owners.push(owner.clone());
        if let Err(e) = save_record(home, rec) {
            if let Some(p) = rec.profiles.iter_mut().find(|p| p.name == name) {
                p.owners.retain(|o| *o != owner);
            }
            return Err(format!("授权台账落盘失败：{}", e));
        }
        return Ok(());
    }
    rec.profiles.push(ProfileEntry {
        name: name.to_string(),
        at: now_secs(),
        owners: vec![owner],
    });
    if let Err(e) = save_record(home, rec) {
        rec.profiles.retain(|p| p.name != name);
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(())
}

/// 目的：取产品根（.home 的父目录）：授权落在产品根内还是根外，决定收尾用快照还原还是精确撤权。
pub(crate) fn product_root(home: &Path) -> PathBuf {
    match home.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => home.to_path_buf(),
    }
}

/// 目的：判断一个路径是否在产品根内（逐分量比，大小写不敏感；能规范化就按规范化路径比）。
pub(crate) fn inside_root(path: &Path, root: &Path) -> bool {
    let canon = |p: &Path| {
        std::fs::canonicalize(p)
            .map(|c| super::super::strip_verbatim_prefix(&c))
            .unwrap_or_else(|_| p.to_path_buf())
    };
    let path_owned = canon(path);
    let root_owned = canon(root);
    let mut left = path_owned.components();
    for want in root_owned.components() {
        let ok = left.next().map(|got| {
            got.as_os_str()
                .to_string_lossy()
                .eq_ignore_ascii_case(&want.as_os_str().to_string_lossy())
        });
        if ok != Some(true) {
            return false;
        }
    }
    true
}

/// 目的：把一条授权先写进台账落盘（根内根外都记），并把当前归属并进它的归属集合。
/// 返回：(是否新建了条目, 是否新增了归属)——回滚时只撤**确实新增过**的那一份归属。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_claim_grant(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    rights: u32,
    owner: Owner,
) -> Result<(bool, bool), String> {
    let key = path.to_string_lossy().into_owned();
    if let Some(g) = rec
        .grants
        .iter_mut()
        .find(|g| g.sid == sid && g.path == key)
    {
        if g.owners.contains(&owner) {
            return Ok((false, false));
        }
        g.owners.push(owner.clone());
        if let Err(e) = save_record(home, rec) {
            if let Some(g) = rec
                .grants
                .iter_mut()
                .find(|g| g.sid == sid && g.path == key)
            {
                g.owners.retain(|o| *o != owner);
            }
            return Err(format!("授权台账落盘失败：{}", e));
        }
        return Ok((false, true));
    }
    rec.grants.push(GrantEntry {
        sid: sid.to_string(),
        path: key.clone(),
        rights,
        at: now_secs(),
        owners: vec![owner],
    });
    if let Err(e) = save_record(home, rec) {
        rec.grants.retain(|g| !(g.sid == sid && g.path == key));
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok((true, true))
}

/// 目的：从一条授权里撤掉一个归属；归属空了就把条目删掉。返回是否删除了条目。
/// 错误：台账落盘失败时返回原因（此时内存与盘上可能不一致，调用方按失败处置）。
pub(crate) fn journal_release_grant(
    home: &Path,
    rec: &mut GrantRecord,
    sid: &str,
    path: &Path,
    owner: Owner,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    let Some(g) = rec
        .grants
        .iter_mut()
        .find(|g| g.sid == sid && g.path == key)
    else {
        return Ok(false);
    };
    if !g.owners.contains(&owner) {
        return Ok(false);
    }
    g.owners.retain(|o| *o != owner);
    let empty = g.owners.is_empty();
    if empty {
        rec.grants.retain(|g| !(g.sid == sid && g.path == key));
    }
    if let Err(e) = save_record(home, rec) {
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(empty)
}

/// 目的：把一条根内路径的原始安全描述符先写进台账落盘；返回本次是否新增了该路径的快照。
/// 错误：台账落盘失败时返回原因（落盘失败就不得动 ACL）。
pub(crate) fn journal_add_snapshot(
    home: &Path,
    rec: &mut GrantRecord,
    path: &Path,
    bytes: Vec<u8>,
) -> Result<bool, String> {
    let key = path.to_string_lossy().into_owned();
    if rec.snapshots.iter().any(|s| s.path == key) {
        return Ok(false);
    }
    rec.snapshots.push(SnapshotEntry {
        path: key.clone(),
        at: now_secs(),
        bytes,
    });
    if let Err(e) = save_record(home, rec) {
        rec.snapshots.retain(|s| s.path != key);
        return Err(format!("授权台账落盘失败：{}", e));
    }
    Ok(true)
}

/// 目的：按“先落台账、再写 ACL、写后核对、失败回滚”完成一条授权。
/// 参数：rec 是本次准备的内存台账；sid_text 是 SID 字符串；target 是落点；root 是产品根；owner 是本次归属。
/// 错误：台账落盘、写后核对或回滚失败时返回原因（回滚失败会一并写进错误）。
/// 返回：true = 真改了 ACL；false = ACE 已够用、只补记了归属（不重复改）。
pub(crate) fn grant_one_journaled(
    home: &Path,
    rec: &mut GrantRecord,
    sid: PSID,
    sid_text: &str,
    target: &GrantTarget,
    root: &Path,
    owner: Owner,
) -> Result<bool, String> {
    let (path, rights, recursive, inherit) = (
        target.path.as_path(),
        target.rights,
        target.recursive,
        target.inherit,
    );
    let key = path.to_string_lossy().into_owned();
    let inside = inside_root(path, root);
    // 根内路径先落原始安全描述符快照（整体还原的依据）；快照不带归属，是否还需要它由该路径上的授权条目派生。
    let snap_created = if inside {
        let bytes = sd_bytes(path).map_err(|e| format!("读原始安全描述符失败：{}", e))?;
        journal_add_snapshot(home, rec, path, bytes)?
    } else {
        false
    };
    let (_, owner_added) = journal_claim_grant(home, rec, sid_text, path, rights, owner.clone())?;
    // ACE 已够用：归属已经补记（同名 agent 的另一个会话靠它保住这条授权），不重复改 DACL。
    if has_ace_for(sid, path, rights) {
        return Ok(false);
    }
    match grant_verified(sid, path, rights, recursive, inherit) {
        Ok(()) => Ok(true),
        Err(e) => {
            // 只有**真写进去**才谈回滚：写都没成功时再报一条"回滚失败"，会把"这次没动过权限项"说反。
            let rollback = if has_any_ace_for(sid, path) {
                revoke_one(sid, path, recursive)
                    .map_err(|x| format!("撤销已写入的 ACE 失败：{}", x))
            } else {
                Ok(())
            };
            // 撤掉本次**确实新增过**的归属；条目空掉后，没有授权引用的快照也一并销掉。
            let mut journal: Option<String> = None;
            if owner_added {
                if let Err(x) = journal_release_grant(home, rec, sid_text, path, owner) {
                    journal = Some(x);
                }
            }
            if inside && snap_created && !rec.grants.iter().any(|g| g.path == key) {
                rec.snapshots.retain(|s| s.path != key);
                if let Err(x) = save_record(home, rec) {
                    journal = Some(format!("台账更新失败：{}", x));
                }
            }
            Err(combine(e, rollback.err(), journal))
        }
    }
}

/// 目的：把主错误与回滚、台账更新两条次生错误拼成一条如实的失败原因。
fn combine(primary: String, rollback: Option<String>, journal: Option<String>) -> String {
    let mut out = primary;
    if let Some(r) = rollback {
        out.push_str("；回滚失败：");
        out.push_str(&r);
    }
    if let Some(j) = journal {
        out.push_str("；台账更新失败：");
        out.push_str(&j);
    }
    out
}

/// 目的：SID → 字符串（写台账用）。
pub(crate) fn sid_to_string(sid: PSID) -> String {
    let mut out: *mut u16 = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(sid, &mut out) } == 0 || out.is_null() {
        return "(未知 SID)".to_string();
    }
    let mut buf: Vec<u16> = Vec::new();
    unsafe {
        let mut i = 0isize;
        loop {
            let c = *out.offset(i);
            if c == 0 {
                break;
            }
            buf.push(c);
            i += 1;
        }
        LocalFree(out as *mut c_void);
    }
    String::from_utf16_lossy(&buf)
}

/// 目的：字符串 → SID（清理时按台账里的字符串还原）。
pub(crate) fn sid_from_string(text: &str) -> Result<PSID, String> {
    let w: Vec<u16> = std::ffi::OsStr::new(text)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let mut sid: PSID = std::ptr::null_mut();
    if unsafe { ConvertStringSidToSidW(w.as_ptr(), &mut sid) } == 0 || sid.is_null() {
        return Err(format!("SID 不合法：{}", text));
    }
    Ok(sid)
}

/// 目的：精确回收——按台账把我们写过的 ACE 逐条撤掉，并删掉我们建过的容器 profile。
/// 返回：给用户看的一句话（清理了几条、删了几个 profile）。
pub fn clean(home: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let rec = load_record(home);
    if rec.is_empty() {
        return Ok("没有台账：本程序没在本机写过权限项".to_string());
    }
    let mut errors: Vec<String> = Vec::new();
    let mut restored = 0usize;
    // 根内路径整体还原原始安全描述符：被写坏的 DACL 只有这一条路能修回来。
    for snap in &rec.snapshots {
        let p = PathBuf::from(&snap.path);
        if std::fs::symlink_metadata(&p).is_err() {
            continue;
        }
        match restore_sd(&p, &snap.bytes) {
            Ok(()) => restored += 1,
            Err(e) => errors.push(e),
        }
    }
    let mut removed = 0usize;
    for grant in &rec.grants {
        // 根内路径已由上面的快照整体还原，这里不再逐条撤（重复撤一个不存在的 ACE 没有意义）。
        if rec.snapshots.iter().any(|s| s.path == grant.path) {
            continue;
        }
        let sid = match sid_from_string(&grant.sid) {
            Ok(s) => s,
            Err(e) => {
                errors.push(e);
                continue;
            }
        };
        let p = PathBuf::from(&grant.path);
        if std::fs::symlink_metadata(&p).is_ok() {
            match revoke_one(sid, &p, true) {
                Ok(()) => removed += 1,
                Err(e) => errors.push(e),
            }
        }
        free_sid(sid);
    }
    let mut deleted = 0usize;
    for prof in &rec.profiles {
        let n: Vec<u16> = std::ffi::OsStr::new(&prof.name)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let hr = unsafe { DeleteAppContainerProfile(n.as_ptr()) };
        if hr >= 0 {
            deleted += 1;
        }
    }
    if !errors.is_empty() {
        // 还有没清掉的：台账保留，供下次重试（删了就等于把待办也删了）。
        return Err(format!(
            "台账回收未完成：{}（已还原 {} 处、已撤销 {} 条、已删 {} 个 profile；台账保留供重试）",
            errors.join("；"),
            restored,
            removed,
            deleted
        ));
    }
    // 全部清干净了才删台账；删不掉本身就是没收干净，如实报错。
    match std::fs::remove_file(record_path(home)) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(format!("权限项已清干净，但删除授权台账失败：{}", e)),
    }
    Ok(format!(
        "已还原 {} 处原始权限、撤销 {} 条授权、删除 {} 个容器 profile",
        restored, removed, deleted
    ))
}

/// 目的：撤销一次会话的授权（会话删除时经 FenceHost 端口调用）。
/// 参数：spec 是该会话各席的围栏范围；home 是产品私有区（台账落点，也是产品根的锚）。
/// 错误：撤权、还原或台账更新失败时如实返回；失败条目留在台账里，供下次重试。
pub fn release_fence(spec: &FenceSpec, home: &Path) -> Result<(), String> {
    let sid = container_sid(&container_name(spec))?;
    let sid_text = sid_to_string(sid);
    free_sid(sid);
    // 只撤**这个会话**的归属：同名 agent 的另一个会话持有另一份 lease，不能被一起撤掉。
    let mut owner = current_owner();
    owner.lease = spec.lease.clone();
    let _lock = lock_ledger(home)?;
    // 没有台账就别凭空造一份：释放只在“确实写过授权”时才动台账文件。
    let had_record = record_path(home).exists();
    let mut rec = load_record(home);
    // 要撤的全从台账派生：这个 SID 记过的每条 (SID, 路径)。没记过的一律不动——
    // 同名 agent 的另一个实例可能正用着同一条授权，凭 spec 反推落点会把它误撤。
    let mut keys: Vec<String> = rec
        .grants
        .iter()
        .filter(|g| g.sid == sid_text)
        .map(|g| g.path.clone())
        .collect();
    keys.sort();
    keys.dedup();
    let mut errors: Vec<String> = Vec::new();
    for key in &keys {
        // 先把本次归属从这条授权里撤掉；还有别的归属（活的、或判不了的）就留着给它用。
        let empty = match rec
            .grants
            .iter_mut()
            .find(|g| g.sid == sid_text && g.path == *key)
        {
            Some(g) => {
                g.owners.retain(|o| *o != owner);
                g.owners.is_empty()
            }
            None => continue,
        };
        if !empty {
            continue;
        }
        let path = PathBuf::from(key);
        // 同一个路径上还有别的 SID 的授权吗（不同 agent 共用路径）：有就只撤自己这一条。
        let has_other = rec
            .grants
            .iter()
            .any(|g| g.sid != sid_text && g.path == *key);
        let snapshot = rec.snapshots.iter().find(|s| s.path == *key).cloned();
        if std::fs::symlink_metadata(&path).is_err() {
            // 路径不在了：ACE 跟着没了，销账即可。
            rec.grants
                .retain(|g| !(g.sid == sid_text && g.path == *key));
            if !has_other {
                rec.snapshots.retain(|s| s.path != *key);
            }
            continue;
        }
        if !has_other {
            if let Some(snap) = snapshot {
                // 根内：最后一个归属走了，按快照整体还原。
                match restore_sd(&path, &snap.bytes) {
                    Ok(()) => {
                        rec.grants
                            .retain(|g| !(g.sid == sid_text && g.path == *key));
                        rec.snapshots.retain(|s| s.path != *key);
                    }
                    Err(e) => errors.push(e),
                }
                continue;
            }
        }
        // 根外（或同路径还有别的 SID）：只精确撤自己这条 ACE。
        match revoke_one_text(&sid_text, &path) {
            Ok(()) => {
                rec.grants
                    .retain(|g| !(g.sid == sid_text && g.path == *key));
            }
            Err(e) => errors.push(e),
        }
    }
    if had_record {
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("授权台账更新失败：{}", e));
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("；"))
    }
}

/// 读一个落点现在的样子：显式包 SID 允许 ACE + 如实记下的读不到 / 认不出。
/// 返回：ace_sids 是见到的显式包 SID；notes 是读不到 DACL 或认不出的条数（不静默吞）。
fn acl_facts(path: &Path) -> (Vec<String>, Vec<String>) {
    match acl_scan(path) {
        Ok(scan) => {
            let mut notes = Vec::new();
            if scan.unparsed > 0 {
                notes.push(format!("这个落点上有 {} 条布局认不出的 ACE", scan.unparsed));
            }
            (orphan_package_aces(path).unwrap_or_default(), notes)
        }
        Err(e) => (Vec::new(), vec![e]),
    }
}

/// 目的：列出台账现值——快照（路径 + 时间）、根外授权（SID / 路径 / 权限位）、容器 profile，
///   以及“当前实际 ACE 与台账对不对得上”的差异。
/// 约束：ACE 的读法只有一处（acl_scan / ace_parts / orphan_package_aces），此处不另写一套。
pub fn catalog(home: &Path) -> Ledger {
    // 列清单也要读一致性快照：写侧都在锁里，读侧不持锁可能读到写了一半的文件。
    // 没有台账就不建私有区、不加锁（只读场景不产生副作用）。
    let _lock = if record_path(home).exists() {
        match lock_ledger(home) {
            Ok(l) => Some(l),
            Err(e) => {
                return Ledger {
                    note: format!("台账加锁失败：{}", e),
                    entries: Vec::new(),
                }
            }
        }
    } else {
        None
    };
    let rec = load_record(home);
    if rec.is_empty() {
        return Ledger {
            note: "没有台账：本程序没在本机写过权限项".to_string(),
            entries: Vec::new(),
        };
    }
    let mut entries: Vec<LedgerEntry> = Vec::new();
    for snap in &rec.snapshots {
        let p = PathBuf::from(&snap.path);
        let exists = std::fs::symlink_metadata(&p).is_ok();
        let (ace_sids, notes) = if exists {
            acl_facts(&p)
        } else {
            (Vec::new(), vec!["这个路径现在不在了".to_string()])
        };
        entries.push(LedgerEntry {
            kind: "snapshot",
            sid: String::new(),
            path: snap.path.clone(),
            rights: None,
            at: snap.at,
            present: exists,
            owners: Vec::new(),
            ace_sids,
            notes,
        });
    }
    for grant in &rec.grants {
        let p = PathBuf::from(&grant.path);
        let exists = std::fs::symlink_metadata(&p).is_ok();
        let (ace_sids, mut notes) = if exists {
            acl_facts(&p)
        } else {
            (Vec::new(), vec!["这个路径现在不在了".to_string()])
        };
        // 差异：台账说授过这个 SID，盘上现在还挂着它的 ACE 吗（看**在不在场**，不看权限位够不够）。
        let mut present = false;
        if exists {
            match sid_from_string(&grant.sid) {
                Ok(sid) => {
                    present = has_any_ace_for(sid, &p);
                    free_sid(sid);
                }
                Err(e) => notes.push(e),
            }
            if !present {
                notes.push("台账里记着这条授权，盘上已经没有这个 SID 的允许 ACE".to_string());
            }
        }
        entries.push(LedgerEntry {
            kind: "grant",
            sid: grant.sid.clone(),
            path: grant.path.clone(),
            rights: Some(grant.rights),
            at: grant.at,
            present,
            owners: grant.owners.clone(),
            ace_sids,
            notes,
        });
    }
    for prof in &rec.profiles {
        let present = profile_exists(&prof.name);
        let notes = if present {
            Vec::new()
        } else {
            vec!["这个 profile 现在不在了".to_string()]
        };
        entries.push(LedgerEntry {
            kind: "profile",
            sid: String::new(),
            path: prof.name.clone(),
            rights: None,
            at: prof.at,
            present,
            owners: prof.owners.clone(),
            ace_sids: Vec::new(),
            notes,
        });
    }
    Ledger {
        note: String::new(),
        entries,
    }
}

/// 目的：按路径**只还原一条**快照（其余条目与整份 DACL 不受影响）。
/// 返回：还原成功的一句话（含“台账里已没有这条”这类如实说明）。
/// 错误：台账里没有该路径、路径不在了、写回被拒，都如实返回；失败时台账**不改**（供重试）。
pub fn restore_one(home: &Path, path: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let mut rec = load_record(home);
    let key = path.to_string_lossy().into_owned();
    let Some(idx) = rec.snapshots.iter().position(|s| s.path == key) else {
        return Err(format!("台账里没有这个路径的快照：{}", key));
    };
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("这个路径现在不在了：{}", key));
    }
    restore_sd(path, &rec.snapshots[idx].bytes)
        .map_err(|e| format!("还原未完成（{}）：{}", key, e))?;
    rec.snapshots.remove(idx);
    // 整体还原会连该路径上所有我们写下的 ACE 一起撤掉：那些授权条目也一并销账。
    rec.grants.retain(|g| g.path != key);
    if let Err(e) = save_record(home, &rec) {
        return Err(format!("已还原该条，但台账更新失败：{}", e));
    }
    rec.drop_if_empty(home);
    Ok(format!("已按台账还原这一条快照的原始权限：{}", key))
}

/// 目的：按 **SID + 路径**只撤一条授权（其余条目不受影响）；**不在台账里也照撤**。
/// 返回：撤权结果的一句话（是否在台账里如实标注）；盘上本来就没有该 SID 的 ACE 时也如实说。
/// 错误：路径不在了、SID 不合法、写撤权后的 DACL 被拒，都如实返回；失败时台账不改动。
pub fn revoke_grant(home: &Path, sid_text: &str, path: &Path) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let key = path.to_string_lossy().into_owned();
    if std::fs::symlink_metadata(path).is_err() {
        return Err(format!("这个路径现在不在了：{}", key));
    }
    let sid = sid_from_string(sid_text)?;
    let present = has_any_ace_for(sid, path);
    let outcome = if present {
        revoke_one(sid, path, true)
    } else {
        Ok(())
    };
    free_sid(sid);
    outcome.map_err(|e| format!("撤销未完成（{}，{}）：{}", sid_text, key, e))?;
    // 台账里有这一条就按条删掉；没有就如实说它不在台账里（照样撤）。
    let mut rec = load_record(home);
    let had = rec
        .grants
        .iter()
        .any(|g| g.sid == sid_text && g.path == key);
    if had {
        rec.grants.retain(|g| !(g.sid == sid_text && g.path == key));
        // 该路径上再没有任何授权引用时，快照也一并销掉（没有可管理的 ACE 了）。
        if !rec.grants.iter().any(|g| g.path == key) {
            rec.snapshots.retain(|s| s.path != key);
        }
        if let Err(e) = save_record(home, &rec) {
            return Err(format!("已撤掉该条 ACE，但台账更新失败：{}", e));
        }
        rec.drop_if_empty(home);
    }
    Ok(match (present, had) {
        (true, true) => format!("已按台账撤销这一条授权：{} → {}", sid_text, key),
        (true, false) => format!(
            "已撤销这一条 ACE（它**不在台账里**——台账外残留）：{} → {}",
            sid_text, key
        ),
        (false, true) => format!(
            "盘上已经没有这个 SID 的允许 ACE；台账里那一条已删：{} → {}",
            sid_text, key
        ),
        (false, false) => format!(
            "盘上本来就没有这个 SID 的允许 ACE，台账里也没有这一条：{} → {}",
            sid_text, key
        ),
    })
}

/// 目的：按名**只删一个**容器 profile（连该容器的存储一起删）。
/// 返回：删除结果的一句话（是否在台账里如实标注）；盘上本来就没有这个 profile 时也如实说。
/// 错误：删除被拒时如实返回；失败时台账不改动。
pub fn remove_profile_one(home: &Path, name: &str) -> Result<String, String> {
    let _lock = lock_ledger(home)?;
    let existed = profile_exists(name);
    if existed && !delete_profile(name) {
        return Err(format!("删不掉这个容器 profile：{}", name));
    }
    let mut rec = load_record(home);
    let had = rec.profiles.iter().any(|p| p.name == name);
    if had {
        rec.profiles.retain(|p| p.name != name);
        if let Err(e) = save_record(home, &rec) {
            return Err(format!("已删掉该 profile，但台账更新失败：{}", e));
        }
        rec.drop_if_empty(home);
    }
    Ok(match (existed, had) {
        (true, true) => format!("已按台账删掉这个容器 profile：{}", name),
        (true, false) => format!("已删掉这个容器 profile（它**不在台账里**）：{}", name),
        (false, true) => format!("这个 profile 已经不在了；台账里那一条已删：{}", name),
        (false, false) => format!("这个 profile 本来就不在，台账里也没有这一条：{}", name),
    })
}

/// 目的：回收前把整份台账拷一份到 .home/fence-backups/（审计 + 想重跑/核对的依据）；保留最近 N 份。
fn backup_record(home: &Path) -> Result<Option<PathBuf>, String> {
    let src = record_path(home);
    if !src.exists() {
        return Ok(None);
    }
    let dir = home.join("fence-backups");
    std::fs::create_dir_all(&dir).map_err(|e| format!("建台账备份目录失败：{}", e))?;
    let dst = dir.join(format!("fence-grants.{}.json", now_secs()));
    std::fs::copy(&src, &dst).map_err(|e| format!("备份台账失败：{}", e))?;
    prune_backups(&dir, 10);
    Ok(Some(dst))
}

/// 目的：备份目录只保留最近 keep 份（按文件名里递增的时间戳排序）；清不掉就如实打印。
fn prune_backups(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("fence-grants.") && n.ends_with(".json") {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    names.sort();
    if names.len() > keep {
        for n in &names[..names.len() - keep] {
            if let Err(e) = std::fs::remove_file(dir.join(n)) {
                eprintln!("[围栏] 旧台账备份未清理（{}）：{}", n, e);
            }
        }
    }
}

/// 目的：启动期对账——按台账回收"归属明确已死"的陈旧授权；无法判定的报告后跳过。
/// 约束：不问用户；只回收归属明确已死的条目（活着的不动）；失败保留台账供下次重试。
///   **不做**孤儿清扫（sweep_orphan_aces / sweep_profiles）——那两条无归属，会误伤在跑实例，只归手动 --fence-clean。
pub fn reconcile(home: &Path) -> super::super::ReconcileReport {
    let mut rep = super::super::ReconcileReport::default();
    let _lock = match lock_ledger(home) {
        Ok(l) => l,
        Err(e) => {
            rep.errors.push(e);
            return rep;
        }
    };
    let mut rec = load_record(home);
    if rec.is_empty() {
        rep.note = "没有台账：本程序没在本机写过权限项".to_string();
        return rep;
    }
    // 归属分类
    let grant_state: Vec<OwnersState> =
        rec.grants.iter().map(|g| owners_state(&g.owners)).collect();
    let profile_state: Vec<OwnersState> = rec
        .profiles
        .iter()
        .map(|p| owners_state(&p.owners))
        .collect();
    // 快照：路径上只要还有一条"非可回收"的授权就保留；否则可回收（含没有任何授权引用的孤儿快照）。
    let snap_reclaim: Vec<bool> = rec
        .snapshots
        .iter()
        .map(|s| {
            !rec.grants
                .iter()
                .zip(&grant_state)
                .any(|(g, st)| g.path == s.path && *st != OwnersState::Reclaim)
        })
        .collect();
    rep.skipped_unjudgeable = grant_state
        .iter()
        .filter(|s| **s == OwnersState::Unjudgeable)
        .count()
        + profile_state
            .iter()
            .filter(|s| **s == OwnersState::Unjudgeable)
            .count();
    rep.kept = grant_state
        .iter()
        .filter(|s| **s != OwnersState::Reclaim)
        .count()
        + snap_reclaim.iter().filter(|b| !**b).count()
        + profile_state
            .iter()
            .filter(|s| **s != OwnersState::Reclaim)
            .count();
    let to_do = grant_state.contains(&OwnersState::Reclaim)
        || profile_state.contains(&OwnersState::Reclaim)
        || snap_reclaim.iter().any(|b| *b);
    if !to_do {
        return rep;
    }
    // 清理前先备份整份台账（审计 + 想重跑/核对的依据）。
    match backup_record(home) {
        Ok(p) => rep.backup = p.map(|x| x.to_string_lossy().into_owned()),
        Err(e) => rep.errors.push(format!("台账备份失败（继续回收）：{}", e)),
    }
    let mut errors: Vec<String> = Vec::new();
    // ① 逐条撤"根外 / 同路径还有保留授权"的授权；根内且是最后一个归属的交给快照整体还原。
    for (i, g) in rec.grants.iter().enumerate() {
        if grant_state[i] != OwnersState::Reclaim {
            continue;
        }
        let path = PathBuf::from(&g.path);
        if std::fs::symlink_metadata(&path).is_err() {
            // 路径不在了：ACE 跟着没了，销账即可。
            rep.reclaimed_grants += 1;
            continue;
        }
        let kept_here = rec
            .grants
            .iter()
            .zip(&grant_state)
            .any(|(o, st)| o.path == g.path && *st != OwnersState::Reclaim);
        let has_snap = rec.snapshots.iter().any(|s| s.path == g.path);
        if has_snap && !kept_here {
            continue; // 交给下面快照整体还原
        }
        match revoke_one_text(&g.sid, &path) {
            Ok(()) => rep.reclaimed_grants += 1,
            Err(e) => errors.push(e),
        }
    }
    // ② 快照整体还原（路径上已无保留授权，或本来就是孤儿快照）。
    for (i, s) in rec.snapshots.iter().enumerate() {
        if !snap_reclaim[i] {
            continue;
        }
        let path = PathBuf::from(&s.path);
        if std::fs::symlink_metadata(&path).is_err() {
            rep.restored_snapshots += 1;
            continue;
        }
        match restore_sd(&path, &s.bytes) {
            Ok(()) => rep.restored_snapshots += 1,
            Err(e) => errors.push(e),
        }
    }
    // ③ 删掉归属明确已死的容器 profile。
    for (i, p) in rec.profiles.iter().enumerate() {
        if profile_state[i] != OwnersState::Reclaim {
            continue;
        }
        if !profile_exists(&p.name) {
            rep.deleted_profiles += 1;
            continue;
        }
        if delete_profile(&p.name) {
            rep.deleted_profiles += 1;
        } else {
            errors.push(format!("删不掉容器 profile：{}", p.name));
        }
    }
    if errors.is_empty() {
        // 全部成功：把可回收的条目从台账里销掉。
        rec.grants = rec
            .grants
            .iter()
            .zip(&grant_state)
            .filter(|(_, st)| **st != OwnersState::Reclaim)
            .map(|(g, _)| g.clone())
            .collect();
        rec.snapshots = rec
            .snapshots
            .iter()
            .zip(&snap_reclaim)
            .filter(|(_, r)| !**r)
            .map(|(s, _)| s.clone())
            .collect();
        rec.profiles = rec
            .profiles
            .iter()
            .zip(&profile_state)
            .filter(|(_, st)| **st != OwnersState::Reclaim)
            .map(|(p, _)| p.clone())
            .collect();
        if let Err(e) = save_record(home, &rec) {
            errors.push(format!("台账更新失败：{}", e));
        } else {
            rec.drop_if_empty(home);
        }
    }
    rep.errors.extend(errors);
    rep
}
