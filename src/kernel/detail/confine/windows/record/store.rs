//! 目的：授权台账的盘上格式与写事务——台账长什么样、怎么在锁内读改存、写前备份留几份。
//! 管：台账三段（容器 profile / 根外授权摘要 / 根内文件快照）的单一格式，单锁文件 + 进程内互斥，
//!   一次事务的 load→mutate→save，以及备份与保留份数。
//! 不管：某条授权该不该建立或撤销（journal.rs / release.rs 按归属与活性决定），也不管 ACL 怎么读写（acl.rs）。
//! 联动：windows/mod.rs 的 prepare_fence 与守门进程都经 lock_ledger 拿事务；内存视图 Ledger/LedgerEntry 在 confine/ledger.rs。

use super::super::super::Owner;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

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
    /// 目的：当次授权是否递归（撤权要按它来：父目录的只读属性只授了根，撤权**不能**整树扫）。
    /// 约束：缺省 true——旧条目没记这一位，按"递归"撤更彻底（宁可多撤自己的 ACE，也不留残留）。
    #[serde(default = "recursive_by_default")]
    pub(crate) recursive: bool,
    /// 目的：还需要这条授权的进程（空 = 缺归属，按"无法判定"处理，不主动回收）。
    #[serde(default)]
    pub(crate) owners: Vec<Owner>,
}

/// 目的：旧台账条目没记 "recursive" 时的缺省（按递归撤，宁可多扫也不留残留）。
fn recursive_by_default() -> bool {
    true
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

/// 目的：回收前把整份台账拷一份到 .home/fence-backups/（审计 + 想重跑/核对的依据）；保留最近 N 份。
pub(crate) fn backup_record(home: &Path) -> Result<Option<PathBuf>, String> {
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
