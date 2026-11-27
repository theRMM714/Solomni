//! 目的：文件写与事务的唯一实现——同一把锁、锁内比对旧版本、暂存后一次发布、失败整体回滚。
//! 管：`LockSpace`（锁目录 + 有界等待 + 按落点身份派生锁名）、`Tx`（`read` / `stage` / `commit`）、一次性 `write` / `transact`、流式 `stream_into`、`ensure_dir` / `remove_tree` / `copy_file` / `within`。
//! 不管：准不准写、写什么内容（策略在 workbench 的 `BenchDriver` 与 `SharedScope`）；版本语义与读后写门禁（在 versions）；锁文件放哪个目录（由组合根注入 `LockSpace`）。
//! 联动：tools 的写工具、versions 的存储、conversation 的 meta、workspace 的绑定与共享区写、provision 的装包与台账都经它写盘。

use crate::kernel::domain::fs::{
    Expect, FS_DUPLICATE_PATH, FS_EXISTS, FS_IO, FS_LOCKED, FS_MISSING, FS_OUTSIDE, FS_STALE, Fault,
};
use crate::kernel::domain::hash::content_hash;
use crate::kernel::domain::platform::HOST_PATH_CASE_INSENSITIVE;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// 目的：跨进程锁的落点与等待——组合根给锁目录，调用方只给要保护的路径。
/// 约束：锁名由路径的落点身份派生（同一物理路径的不同写法共用一把），不由调用方拼。
#[derive(Debug, Clone)]
pub struct LockSpace {
    dir: PathBuf,
    wait: Duration,
}

static DEFAULT_SPACE: OnceLock<LockSpace> = OnceLock::new();

/// 目的：装一次产品锁目录——此后的写事务默认按它跨进程互斥。
/// 返回：本次装上了给 true；已经装过（只认第一次）给 false。
pub fn install_lock_space(space: LockSpace) -> bool {
    DEFAULT_SPACE.set(space).is_ok()
}

/// 目的：本次事务该用哪个锁空间：调用方给的优先，否则用装好的产品锁目录。
fn space_of(space: Option<&LockSpace>) -> Option<&LockSpace> {
    space.or_else(|| DEFAULT_SPACE.get())
}

impl LockSpace {
    /// 目的：在 `dir` 下放锁文件，默认最多等 10 秒。
    /// 参数：`dir` 锁目录（不存在时按需创建）。
    pub fn new(dir: PathBuf) -> LockSpace {
        LockSpace {
            dir,
            wait: Duration::from_secs(10),
        }
    }

    /// 目的：换一个拿锁的有界等待时间（用例把等待压到几十毫秒）。
    /// 参数：`wait` 等多久算失败。
    #[cfg(test)]
    pub fn with_wait(mut self, wait: Duration) -> LockSpace {
        self.wait = wait;
        self
    }

    /// 目的：算出保护这条路径的锁文件，并按需建好锁目录。
    /// 参数：`path` 要保护的路径（可以是还不存在的新文件）。
    /// 错误：锁目录建不出来时给 `FS_LOCKED`。
    pub fn lock_file(&self, path: &Path) -> Result<PathBuf, Fault> {
        let mut key = identity(path);
        if HOST_PATH_CASE_INSENSITIVE {
            key = key.to_lowercase();
        }
        if let Err(e) = std::fs::create_dir_all(&self.dir) {
            return Err(Fault::new(
                FS_LOCKED,
                format!("创建锁目录失败（{}）：{}", self.dir.display(), e),
            ));
        }
        Ok(self
            .dir
            .join(format!("fs-{}.lock", content_hash(key.as_bytes()))))
    }
}

/// 目的：一笔事务的现场——已经拿到锁，改动一批一批暂存，最后一次发布。
/// 约束：只对 `begin` 时给过的路径动手；不 `commit` 就丢下，暂存文件会被清掉、磁盘不变。
pub struct Tx<'a> {
    _guards: Vec<MutexGuard<'a, ()>>,
    _files: Vec<std::fs::File>,
    locked: BTreeSet<PathBuf>,
    staged: Vec<Staged>,
    what: String,
}

/// 目的：一条已经准备好、还没发布出去的改动。
struct Staged {
    path: PathBuf,
    tmp: Option<PathBuf>,
    previous: Option<Vec<u8>>,
    absent: bool,
    version: String,
}

impl Tx<'_> {
    /// 目的：锁里读一条已锁路径的旧内容。
    /// 参数：`path` 必须是本次事务锁过的路径。
    /// 返回：内容；目标不存在为 `None`。
    pub fn read(&self, path: &Path) -> Result<Option<Vec<u8>>, Fault> {
        self.claim(path)?;
        read_bytes(path)
    }

    /// 目的：把一条改动暂存起来；这一步在锁里做旧版本比对。
    /// 参数：`path` 已锁路径；`body` 新内容（`None` = 删除）；`expect` 期望的旧状态。
    /// 错误：期望不成立给 `FS_STALE` / `FS_EXISTS` / `FS_MISSING`，暂存失败给 `FS_IO`。
    pub fn stage(&mut self, path: &Path, body: Option<&[u8]>, expect: Expect) -> Result<(), Fault> {
        self.stage_inner(path, body, expect, false)
    }

    /// 目的：同 `stage`，但发布出去的文件只给当前用户读写（登记处的私密文件用它）。
    /// 参数：`path` 已锁路径；`body` 新内容；`expect` 期望的旧状态。
    pub fn stage_private(&mut self, path: &Path, body: &[u8], expect: Expect) -> Result<(), Fault> {
        self.stage_inner(path, Some(body), expect, true)
    }

    /// 目的：`stage` 与 `stage_private` 的共同实现——临时文件写好、权限收紧、登记待发布。
    fn stage_inner(
        &mut self,
        path: &Path,
        body: Option<&[u8]>,
        expect: Expect,
        restrict: bool,
    ) -> Result<(), Fault> {
        self.claim(path)?;
        let current = read_bytes(path)?;
        let version = current.as_deref().map(content_hash);
        check_expect(path, &expect, version.as_deref())?;
        let absent = expect == Expect::Absent;
        match body {
            Some(bytes) => {
                let tmp = stage_temp(path, bytes, current.is_some().then_some(path), restrict)?;
                self.staged.push(Staged {
                    path: path.to_path_buf(),
                    tmp: Some(tmp),
                    previous: current,
                    absent,
                    version: content_hash(bytes),
                });
            }
            None => self.staged.push(Staged {
                path: path.to_path_buf(),
                tmp: None,
                previous: current,
                absent,
                version: String::new(),
            }),
        }
        Ok(())
    }

    /// 目的：把暂存的改动一次发布出去；有一条失败就把已经发出去的按相反顺序回滚。
    /// 返回：按暂存顺序给出每条改动的新内容版本（删除给空串）。
    /// 错误：发布失败给对应码；回滚也失败时两条原因都写进文案。
    pub fn commit(&mut self) -> Result<Vec<String>, Fault> {
        let staged = std::mem::take(&mut self.staged);
        let cleanup = |items: &[Staged]| {
            for item in items {
                if let Some(tmp) = &item.tmp {
                    let _ = std::fs::remove_file(tmp);
                }
            }
        };
        for i in 0..staged.len() {
            let item = &staged[i];
            let published = match &item.tmp {
                Some(tmp) => publish(tmp, &item.path, item.absent),
                None => remove_file_if_present(&item.path),
            };
            if let Err(mut fault) = published {
                for j in (0..i).rev() {
                    if let Err(e) = restore(&staged[j].path, staged[j].previous.as_deref()) {
                        fault.message.push_str(&format!(
                            "\n回滚 {} 也失败了：{}；请手工检查。",
                            staged[j].path.display(),
                            e.message
                        ));
                    }
                }
                cleanup(&staged[i..]);
                return Err(fault);
            }
        }
        Ok(staged.into_iter().map(|item| item.version).collect())
    }

    /// 目的：确认这条路径在本次事务的锁里。
    fn claim(&self, path: &Path) -> Result<(), Fault> {
        if self.locked.contains(path) {
            return Ok(());
        }
        Err(Fault::new(
            FS_IO,
            format!(
                "{} 不在这笔{}的锁里：事务只能动开始时就锁住的路径。",
                path.display(),
                self.what
            ),
        ))
    }
}

/// 目的：在一把锁里改一批路径——先按路径排序取锁，再让调用方读、改、提交。
/// 参数：`space` 跨进程锁的落点（`None` 只用进程内锁）；`paths` 本次要动的全部路径；`what` 只进报错文案（如「写入」）；`body` 拿到锁后执行。
/// 错误：有重复路径给 `FS_DUPLICATE_PATH`；锁拿不到给 `FS_LOCKED`；`body` 的错误原样返回。
pub fn transact_with<T>(
    space: Option<&LockSpace>,
    paths: &[&Path],
    what: &str,
    body: impl FnOnce(&mut Tx<'_>) -> Result<T, Fault>,
) -> Result<T, Fault> {
    let mut seen: BTreeSet<PathBuf> = BTreeSet::new();
    for path in paths {
        if !path.is_absolute() {
            return Err(Fault::new(
                FS_OUTSIDE,
                format!("路径必须是绝对路径：{}", path.display()),
            ));
        }
        if !seen.insert(path.to_path_buf()) {
            return Err(Fault::new(
                FS_DUPLICATE_PATH,
                format!(
                    "同一个路径在一次事务里出现了两次：{}；没有写盘。",
                    path.display()
                ),
            ));
        }
    }
    let mut order: Vec<&Path> = paths.to_vec();
    order.sort();
    let keys: Vec<String> = order.iter().map(|p| identity(p)).collect();
    let arcs: Vec<Arc<Mutex<()>>> = keys.iter().map(|k| path_lock(k)).collect();
    let space = space_of(space);
    let wait = space.map(|s| s.wait).unwrap_or(Duration::from_secs(10));
    let mut guards: Vec<MutexGuard<'_, ()>> = Vec::with_capacity(arcs.len());
    for arc in &arcs {
        // 进程内锁也要有界：另一笔写入占着同一条路径时，等 `wait` 秒就报锁定，不无限等。
        let deadline = std::time::Instant::now() + wait;
        loop {
            match arc.try_lock() {
                Ok(guard) => {
                    guards.push(guard);
                    break;
                }
                Err(std::sync::TryLockError::Poisoned(e)) => {
                    guards.push(e.into_inner());
                    break;
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return Err(Fault::new(
                            FS_LOCKED,
                            format!("{}被另一笔写入占用，等待超时（稍后重试）", what),
                        ));
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
            }
        }
    }
    let mut files: Vec<std::fs::File> = Vec::new();
    if let Some(space) = space {
        for path in &order {
            let lock = space.lock_file(path)?;
            let file = crate::kernel::detail::lock::lock_within(&lock, space.wait, what)
                .map_err(|e| Fault::new(FS_LOCKED, format!("{}：{}", path.display(), e)))?;
            files.push(file);
        }
    }
    let mut tx = Tx {
        _guards: guards,
        _files: files,
        locked: seen,
        staged: Vec::new(),
        what: what.to_string(),
    };
    let out = body(&mut tx)?;
    if !tx.staged.is_empty() {
        return Err(Fault::new(
            FS_IO,
            format!("这笔{}还有没提交的改动：事务要么全提交、要么全不写。", what),
        ));
    }
    Ok(out)
}

/// 目的：一次写完一条路径。
/// 参数：`space` 跨进程锁的落点；`path` 目标；`body` 新内容；`expect` 期望的旧状态。
/// 返回：写进去的内容版本。
pub fn write(
    space: Option<&LockSpace>,
    path: &Path,
    body: &[u8],
    expect: Expect,
) -> Result<String, Fault> {
    transact_with(space, &[path], "写入", |tx| {
        tx.stage(path, Some(body), expect)?;
        let mut versions = tx.commit()?;
        Ok(versions.pop().unwrap_or_default())
    })
}

/// 目的：一次写完一条私密文件——发布出去只给当前用户读写。
/// 参数：`space` 跨进程锁的落点；`path` 目标；`body` 新内容；`expect` 期望的旧状态。
/// 返回：写进去的内容版本。
pub fn write_private(
    space: Option<&LockSpace>,
    path: &Path,
    body: &[u8],
    expect: Expect,
) -> Result<String, Fault> {
    transact_with(space, &[path], "写入", |tx| {
        tx.stage_private(path, body, expect)?;
        let mut versions = tx.commit()?;
        Ok(versions.pop().unwrap_or_default())
    })
}

/// 目的：往文件尾部追加一段字节（不存在就建）——流水这类只追加的写入用它，不整份重写。
/// 参数：`space` 跨进程锁的落点；`path` 目标；`bytes` 追加内容。
/// 错误：拿不到锁给 `FS_LOCKED`；写不进去给 `FS_IO`。
pub fn append(space: Option<&LockSpace>, path: &Path, bytes: &[u8]) -> Result<(), Fault> {
    transact_with(space, &[path], "追加", |_tx| {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Fault::io(path, e))?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .map_err(|e| Fault::io(path, e))?;
        file.write_all(bytes).map_err(|e| Fault::io(path, e))
    })
}

/// 目的：把一次流式写入放进事务的锁里——下载续传这类不能整份缓冲的写用它。
/// 参数：`space` 跨进程锁的落点；`path` 目标；`append` 追加还是从头重写；`what` 事务名（等锁超时的文案里出现）；`body` 拿到写句柄的那一段。
/// 返回：`body` 的返回值。
/// 错误：拿不到锁给 `FS_LOCKED`；开不出句柄、写、flush、sync 失败给 `FS_IO`。
pub fn stream_into<T>(
    space: Option<&LockSpace>,
    path: &Path,
    append: bool,
    what: &str,
    body: impl FnOnce(&mut dyn Write) -> Result<T, Fault>,
) -> Result<T, Fault> {
    transact_with(space, &[path], what, |_tx| {
        if let Some(dir) = path.parent() {
            ensure_dir(dir)?;
        }
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .append(append)
            .truncate(!append)
            .open(path)
            .map_err(|e| Fault::io(path, e))?;
        let mut writer = std::io::BufWriter::new(file);
        let out = body(&mut writer)?;
        writer.flush().map_err(|e| Fault::io(path, e))?;
        let file = writer
            .into_inner()
            .map_err(|e| Fault::io(path, e.error()))?;
        file.sync_all().map_err(|e| Fault::io(path, e))?;
        Ok(out)
    })
}

/// 目的：按需建目录（含各级父目录）。
/// 错误：建不出来给 `FS_IO`。
pub fn ensure_dir(path: &Path) -> Result<(), Fault> {
    std::fs::create_dir_all(path).map_err(|e| Fault::io(path, e))
}

/// 目的：删一棵树；链接只删链接本身，绝不顺着链接进目标；本来就不存在算成功。
/// 参数：`path` 目录或文件。
pub fn remove_tree(path: &Path) -> Result<(), Fault> {
    let meta = match std::fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(Fault::io(path, e)),
    };
    if meta.file_type().is_symlink() {
        return remove_link(path);
    }
    if !meta.is_dir() {
        return std::fs::remove_file(path).map_err(|e| Fault::io(path, e));
    }
    let entries = std::fs::read_dir(path).map_err(|e| Fault::io(path, e))?;
    for entry in entries {
        let entry = entry.map_err(|e| Fault::io(path, e))?;
        remove_tree(&entry.path())?;
    }
    std::fs::remove_dir(path).map_err(|e| Fault::io(path, e))
}

/// 目的：把已经在同一卷上的文件挪到目标位置（比如外部程序刚生成的结果卷）。
/// 参数：`from` 现成的文件；`to` 目标（父目录按需建）。
/// 错误：挪不过去给 `FS_IO`。
pub fn move_file(from: &Path, to: &Path) -> Result<(), Fault> {
    if let Some(dir) = to.parent() {
        ensure_dir(dir)?;
    }
    std::fs::rename(from, to).map_err(|e| Fault::io(to, e))
}

/// 目的：复制一份文件（保留权限位），父目录不存在时按需建。
/// 参数：`from` 源；`to` 目标。
pub fn copy_file(from: &Path, to: &Path) -> Result<(), Fault> {
    if let Some(dir) = to.parent() {
        ensure_dir(dir)?;
    }
    std::fs::copy(from, to).map_err(|e| Fault::io(to, e))?;
    Ok(())
}

/// 一条路径的"真实落点"，允许尾部还不存在。
enum RealPath {
    /// 目的：已存在的整段（或已存在前缀 + 还不存在的尾段）折出来的真实路径。
    Real(PathBuf),
    /// 目的：中间或末尾是悬空软链 / junction，真实落点不可信。
    Dangling,
    /// 目的：连盘符都解析不出来。
    Unreachable,
}

/// 目的：把路径拆成「最近的已存在祖先的真实路径」+「其后还不存在的段」。
/// 约束：写到还没建出来的目录时也要能判越界，不能因为尾段不存在就当成不可达。
fn real_path(path: &Path) -> RealPath {
    let mut probe = path;
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    loop {
        if let Ok(real) = probe.canonicalize() {
            rest.reverse();
            return RealPath::Real(rest.iter().fold(real, |path, name| path.join(name)));
        }
        if std::fs::symlink_metadata(probe)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
        {
            return RealPath::Dangling;
        }
        let Some(name) = probe.file_name() else {
            return RealPath::Unreachable;
        };
        rest.push(name.to_os_string());
        let Some(parent) = probe.parent() else {
            return RealPath::Unreachable;
        };
        probe = parent;
    }
}

/// 目的：确认真实落点仍在 `root` 内（目标不存在时退到最近的已存在祖先）。
/// 参数：`root` 允许的根；`path` 要查的路径。
/// 错误：越过根（含悬空符号链接 / junction）给 `FS_OUTSIDE`。
pub fn within(root: &Path, path: &Path) -> Result<(), Fault> {
    let root_real = match real_path(root) {
        RealPath::Real(real) => real,
        _ => {
            return Err(Fault::new(
                FS_OUTSIDE,
                format!("根目录不可达：{}", root.display()),
            ));
        }
    };
    let outside = |why: &str| {
        Fault::new(
            FS_OUTSIDE,
            format!("路径越过允许的根（{}）：{}", why, path.display()),
        )
    };
    match real_path(path) {
        RealPath::Real(real) => {
            if real.starts_with(&root_real) {
                Ok(())
            } else {
                Err(outside("符号链接 / junction 指向根外"))
            }
        }
        RealPath::Dangling => Err(outside("符号链接 / junction 指向根外或悬空")),
        RealPath::Unreachable => Err(Fault::new(
            FS_OUTSIDE,
            format!("路径不可达：{}", path.display()),
        )),
    }
}

/// 目的：锁的键——同一物理路径的不同写法（映射盘、UNC 别名、8.3 短名）算同一个键。
/// 约束：目标还不存在时取最近已存在祖先的身份再拼上后面的相对段；取不到身份就退回规范化文本。
fn identity(path: &Path) -> String {
    let mut missing: Vec<OsString> = Vec::new();
    let mut probe = path;
    loop {
        if let Ok(canonical) = probe.canonicalize() {
            let mut text = crate::kernel::detail::locus::locus_of(&canonical)
                .map(|l| l.key())
                .unwrap_or_else(|_| canonical.to_string_lossy().into_owned());
            for seg in missing.iter().rev() {
                text.push('/');
                text.push_str(&seg.to_string_lossy());
            }
            return text;
        }
        match (probe.file_name(), probe.parent()) {
            (Some(name), Some(parent)) if parent != probe => {
                missing.push(name.to_os_string());
                probe = parent;
            }
            _ => return path.to_string_lossy().into_owned(),
        }
    }
}

/// 目的：按锁键取一把进程内锁；锁表是进程级全局表，不随实例走。
fn path_lock(key: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let table = LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 目的：读整份内容；目标不存在给 `None`。
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, Fault> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(Fault::io(path, e)),
    }
}

/// 目的：锁内做旧版本比对。
fn check_expect(path: &Path, expect: &Expect, current: Option<&str>) -> Result<(), Fault> {
    match expect {
        Expect::Any => Ok(()),
        Expect::Absent => match current {
            None => Ok(()),
            Some(_) => Err(Fault::new(
                FS_EXISTS,
                format!(
                    "{} 在写入前已经存在（别的写者抢先建成）：这次没有覆盖它，请重新 read 后再决定。",
                    path.display()
                ),
            )),
        },
        Expect::Version(want) => match current {
            None => Err(Fault::new(
                FS_MISSING,
                format!("{} 在写入前已经不存在：这次没有写盘。", path.display()),
            )),
            Some(now) if now != want => Err(Fault::new(
                FS_STALE,
                format!(
                    "{} 在你读过之后又被改动过（内容与当时不同：期望 {}，现在是 {}）：请重新 read 一遍再决定怎么写，不要凭记忆覆盖。",
                    path.display(),
                    want,
                    now
                ),
            )),
            Some(_) => Ok(()),
        },
    }
}

/// 目的：同目录临时文件的唯一名字（前缀带点，避免被当成正常内容）。
fn temp_name(path: &Path, n: u64) -> OsString {
    let mut name: OsString = ".".into();
    match path.file_name() {
        Some(file) => name.push(file),
        None => name.push("file"),
    }
    name.push(format!(".solomni-{}-{}.tmp", std::process::id(), n));
    name
}

/// 目的：把新内容写进同目录临时文件并落盘，沿用旧文件的权限。
/// 约束：临时文件与目标同目录，发布时的 rename 才是原子的。
fn stage_temp(
    path: &Path,
    bytes: &[u8],
    permissions_from: Option<&Path>,
    restrict: bool,
) -> Result<PathBuf, Fault> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| Fault::io(path, e))?;
    }
    let (tmp, mut file) = loop {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp = path.with_file_name(temp_name(path, n));
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)
        {
            Ok(file) => break (tmp, file),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(Fault::io(path, e)),
        }
    };
    let staged = file
        .write_all(bytes)
        .and_then(|_| file.sync_all())
        .and_then(|_| {
            if let Some(from) = permissions_from
                && let Ok(meta) = std::fs::metadata(from)
            {
                std::fs::set_permissions(&tmp, meta.permissions())?;
            }
            if restrict {
                crate::kernel::detail::fs_meta::restrict_private_file(&tmp)?;
            }
            Ok(())
        });
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&tmp);
        return Err(Fault::io(path, e));
    }
    Ok(tmp)
}

/// 目的：发布临时文件：占位发布（不覆盖）时用硬链接，否则原子替换。
fn publish(tmp: &Path, path: &Path, no_replace: bool) -> Result<(), Fault> {
    if !no_replace {
        return std::fs::rename(tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(tmp);
            Fault::io(path, e)
        });
    }
    match std::fs::hard_link(tmp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(tmp);
            Err(Fault::new(
                FS_EXISTS,
                format!(
                    "{} 在写入前已经存在（别的写者抢先建成）：这次没有覆盖它，请重新 read 后再决定。",
                    path.display()
                ),
            ))
        }
        Err(_) => {
            // 文件系统不支持硬链接时的兜底：create_new 保证不覆盖，再复制内容。
            let mut src = std::fs::File::open(tmp).map_err(|e| Fault::io(path, e))?;
            let mut dst = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(tmp);
                    return Err(Fault::new(
                        FS_EXISTS,
                        format!(
                            "{} 在写入前已经存在（别的写者抢先建成）：这次没有覆盖它，请重新 read 后再决定。",
                            path.display()
                        ),
                    ));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(tmp);
                    return Err(Fault::io(path, e));
                }
            };
            let copied = std::io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all());
            let _ = std::fs::remove_file(tmp);
            if let Err(e) = copied {
                let _ = std::fs::remove_file(path);
                return Err(Fault::io(path, e));
            }
            Ok(())
        }
    }
}

/// 目的：把旧内容原子写回（事务回滚用）。
fn restore(path: &Path, previous: Option<&[u8]>) -> Result<(), Fault> {
    match previous {
        Some(bytes) => {
            let tmp = stage_temp(path, bytes, Some(path), false)?;
            publish(&tmp, path, false)
        }
        None => remove_file_if_present(path),
    }
}

/// 目的：删一个文件；本来就不存在算成功。
fn remove_file_if_present(path: &Path) -> Result<(), Fault> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(Fault::io(path, e)),
    }
}

/// 目的：删链接本身：Windows 的目录链接 / junction 可能只认 `remove_dir`，先试 `remove_file` 再退。
fn remove_link(path: &Path) -> Result<(), Fault> {
    std::fs::remove_file(path)
        .or_else(|_| std::fs::remove_dir(path))
        .map_err(|e| Fault::io(path, e))
}

#[cfg(test)]
mod tests {
    //! 目的：文件事务的契约——同键互斥、旧版本比对、全落或全不落、越界判定（内核用真实实例）。
    //! 管：只覆盖本模块的公开契约与报错文案；夹具在本文件内。
    //! 不管：准不准写、写什么内容（策略在 workbench）。
    //! 联动：与 `src/kernel/detail/fs.rs` 同模块。

    use super::*;
    use std::time::Duration;

    /// 目的：期望的旧版本对不上就不写盘，磁盘上仍是上一版。
    #[test]
    fn a_stale_expect_never_lands() {
        let dir = crate::tests::scratch("fs-stale");
        let path = dir.join("note.txt");
        let first = write(None, &path, b"a", Expect::Any).expect("首次写");
        let err = write(None, &path, b"b", Expect::Version("deadbeef".to_string()))
            .expect_err("旧版本不符应当不写");
        assert_eq!(err.code, FS_STALE);
        assert!(
            err.message.contains("又被改动过"),
            "报错说清原因：{}",
            err.message
        );
        assert_eq!(std::fs::read_to_string(&path).expect("读回"), "a");
        write(None, &path, b"b", Expect::Version(first)).expect("版本对上才写");
        assert_eq!(std::fs::read_to_string(&path).expect("读回"), "b");
        let _ = remove_tree(&dir);
    }

    /// 目的：同一路径的第二笔写在等待上限内如实超时；放掉第一笔后能接着写。
    #[test]
    fn a_second_writer_on_the_same_path_waits_then_gives_up() {
        let dir = crate::tests::scratch("fs-mutex");
        let path = dir.join("held.txt");
        let space = LockSpace::new(dir.join("locks")).with_wait(Duration::from_millis(60));
        let (held, waiting) = std::sync::mpsc::channel::<()>();
        let (release, hold) = std::sync::mpsc::channel::<()>();
        let worker = {
            let path = path.clone();
            let space = space.clone();
            std::thread::spawn(move || {
                write(Some(&space), &path, b"first", Expect::Any).expect("持锁方写");
                transact_with(Some(&space), &[path.as_path()], "测试持锁", |_tx| {
                    held.send(()).expect("通知持锁");
                    hold.recv().expect("等放锁");
                    Ok(())
                })
                .expect("持锁段");
            })
        };
        waiting.recv().expect("等持锁");
        let err =
            write(Some(&space), &path, b"second", Expect::Any).expect_err("被占住应当等不到锁");
        assert_eq!(err.code, FS_LOCKED);
        assert!(
            err.message.contains("占用"),
            "报错说清等不到：{}",
            err.message
        );
        assert_eq!(std::fs::read_to_string(&path).expect("读回"), "first");
        release.send(()).expect("放锁");
        worker.join().expect("持锁线程");
        write(Some(&space), &path, b"second", Expect::Any).expect("放掉后能写");
        assert_eq!(std::fs::read_to_string(&path).expect("读回"), "second");
        let _ = remove_tree(&dir);
    }

    /// 目的：一笔多路径改动里有一条没做成，已经发布的那些也要回到原样。
    #[test]
    fn a_batch_that_fails_partway_leaves_every_path_as_it_was() {
        let dir = crate::tests::scratch("fs-batch");
        let good = dir.join("aaa.txt");
        let wall = dir.join("bbb");
        std::fs::write(&good, "old").expect("铺旧内容");
        std::fs::create_dir_all(&wall).expect("铺一堵墙");
        let err = transact_with(None, &[good.as_path(), wall.as_path()], "批量写", |tx| {
            tx.stage(&good, Some(b"new"), Expect::Any)?;
            tx.stage(&wall, Some(b"boom"), Expect::Any)?;
            tx.commit()
        })
        .expect_err("写到目录上应当失败");
        assert_eq!(err.code, FS_IO);
        assert_eq!(
            std::fs::read_to_string(&good).expect("读回"),
            "old",
            "已经发布的那条要撤回"
        );
        assert!(wall.is_dir(), "墙没被换掉");
        let _ = remove_tree(&dir);
    }

    /// 目的：同一笔事务里同一路径出现两次直接拒绝，不猜他要哪一条。
    #[test]
    fn one_batch_names_a_path_once() {
        let dir = crate::tests::scratch("fs-dup");
        let path = dir.join("same.txt");
        let err = transact_with(None, &[path.as_path(), path.as_path()], "重复", |_tx| {
            Ok(())
        })
        .expect_err("同一路径两次应当拒绝");
        assert_eq!(err.code, FS_DUPLICATE_PATH);
        let _ = remove_tree(&dir);
    }

    /// 目的：悬空软链指向的路径不算在根内（越界判定不跟随到"不存在"）。
    #[test]
    fn a_dangling_link_is_not_inside_the_root() {
        let dir = crate::tests::scratch("fs-dangling");
        let root = dir.join("root");
        std::fs::create_dir_all(&root).expect("铺根");
        let link = root.join("escape");
        if crate::kernel::detail::fs_meta::symlink("../../nowhere", &link).is_err() {
            let _ = remove_tree(&dir);
            return;
        }
        let err = within(&root, &link.join("child.txt")).expect_err("悬空软链不算根内");
        assert_eq!(err.code, FS_OUTSIDE);
        within(&root, &root.join("plain.txt")).expect("根内正常路径");
        let _ = remove_tree(&dir);
    }

    /// 目的：根还没建出来时也要能写进去并且照样判越界（留档备份目录是第一次写才出现的）。
    /// 约束：越界仍给 FS_OUTSIDE，不因为根不存在就把一切当成不可达。
    #[test]
    fn a_root_that_does_not_exist_yet_is_still_a_boundary() {
        let dir = crate::tests::scratch("fs-missing-root");
        let root = dir.join("lost").join("agent");
        assert!(!root.exists(), "这根还没建出来");
        let target = root.join("note.txt");
        within(&root, &target).expect("根不存在也算目标在根内");
        write(None, &target, b"hi", Expect::Any).expect("写进去顺带把根建出来");
        assert_eq!(std::fs::read_to_string(&target).expect("读回"), "hi");
        let outside = dir.join("elsewhere.txt");
        let err = within(&root, &outside).expect_err("根外的路径仍要挡");
        assert_eq!(err.code, FS_OUTSIDE);
        let _ = remove_tree(&dir);
    }
}
