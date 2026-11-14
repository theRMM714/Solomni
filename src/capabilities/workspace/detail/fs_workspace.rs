//! 目的：工作区落盘——会话目录下的共享区、各 agent 沙箱与外部工作区绑定投影。
//! 管：目录布局（`work/`、沙箱、`.work-root`）与文件清单 / 用量。
//! 不管：版本库提交语义（在 service/domain）与会话 meta 的读写（在 session）。
//! 联动：由 workspace 的 service.rs 持有（R12）；布局见 docs/session/session-model.md。

use crate::capabilities::workspace::api::{AreaUsage, WorkFiles, WorkRoots, WorkUsage};
use crate::capabilities::workspace::ports::Workdirs;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 每个根最多列这么多条（超出即截断；@ 菜单不需要无限长，也避免大目录拖慢前端）。
const MAX_LISTED: usize = 300;

/// 用量结果的内存缓存时长：前端在 `@` 菜单与删除确认里取用量，短时间重复取不必重走整棵树。
/// 写路径不做逐写失效——陈旧上界就是这个 TTL。
const USAGE_TTL: Duration = Duration::from_secs(2);

/// 工作区绑定的布局投影文件名（相对会话目录）：内容 = 外部工作区根的绝对路径。
/// 它只是布局适配层的投影，权威在会话 meta 的 `workspace` 段。
const ROOT_MARK: &str = ".work-root";

/// 递归列文件（只列文件，不列目录）；相对路径统一用 / 分隔，逐层按名字排序保证稳定。
fn collect_files(root: &Path, out: &mut Vec<String>, prefix: &str) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut items: Vec<(String, PathBuf, bool)> = entries
        .flatten()
        .map(|e| {
            let p = e.path();
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = p.is_dir();
            (name, p, is_dir)
        })
        .collect();
    items.sort_by(|a, b| a.0.cmp(&b.0));
    for (name, path, is_dir) in items {
        if out.len() >= MAX_LISTED {
            return;
        }
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{}/{}", prefix, name)
        };
        if is_dir {
            collect_files(&path, out, &rel);
        } else {
            out.push(rel);
        }
    }
}

/// 目的：路径本身是不是链接 / 联接（不跟随）。用量统计只数它本身，不递归进去。
///   跟随会重复计数（链接与真身各算一次），自指链接还会成环；`symlink_metadata` 只看链接本身。
fn is_link(path: &Path) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
        std::fs::symlink_metadata(path)
            .map(|m| m.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
            .unwrap_or(false)
    }
    #[cfg(not(windows))]
    {
        std::fs::symlink_metadata(path)
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    }
}

/// 递归累加一个区的文件数与字节（目录不计；链接按一个文件计、不跟随；单个文件 stat 失败按 0 计，不因此挡下整次统计）。
fn collect_usage(root: &Path, files: &mut usize, bytes: &mut u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if is_link(&p) {
            *files += 1;
            *bytes += std::fs::symlink_metadata(&p).map(|m| m.len()).unwrap_or(0);
            continue;
        }
        if e.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            collect_usage(&p, files, bytes);
        } else {
            *files += 1;
            *bytes += std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
        }
    }
}

fn area_usage(root: &Path) -> AreaUsage {
    let mut files = 0usize;
    let mut bytes = 0u64;
    collect_usage(root, &mut files, &mut bytes);
    AreaUsage { files, bytes }
}

pub struct FsWorkspace {
    /// 会话根（产品根下的 session/）。
    sessions: PathBuf,
    /// 用量缓存：session + agents → (取数时刻, 结果)。只读路径，过 TTL 重算。
    usage: Mutex<HashMap<String, (Instant, WorkUsage)>>,
    /// 缓存时长（测试可缩短；产品固定 `USAGE_TTL`）。
    usage_ttl: Duration,
}

impl FsWorkspace {
    pub fn new(sessions: PathBuf) -> FsWorkspace {
        FsWorkspace {
            sessions,
            usage: Mutex::new(HashMap::new()),
            usage_ttl: USAGE_TTL,
        }
    }

    /// 目的：测试专用：缩短用量缓存时长，避免用例为了过期等满 2 秒。
    #[cfg(test)]
    pub(crate) fn with_usage_ttl(mut self, ttl: Duration) -> FsWorkspace {
        self.usage_ttl = ttl;
        self
    }

    fn session_dir(&self, session: &str) -> PathBuf {
        self.sessions.join(session)
    }

    fn mark_path(&self, session: &str) -> PathBuf {
        self.session_dir(session).join(ROOT_MARK)
    }

    /// 共享区主副本根：有绑定投影就用它，否则是产品内的 session/<名>/work。
    fn shared_root(&self, session: &str) -> Result<PathBuf, String> {
        let mark = self.mark_path(session);
        match std::fs::read_to_string(&mark) {
            Ok(text) => {
                let root = crate::kernel::api::native(&PathBuf::from(text.trim()));
                if !root.is_absolute() {
                    return Err(format!("工作区绑定不是绝对路径：{}", root.display()));
                }
                // 外部根可能被用户移走 / 删除：会话记录仍可打开，但一切工作区操作如实报错。
                let meta = std::fs::metadata(&root)
                    .map_err(|e| format!("工作区已不存在：{}（{}）", root.display(), e))?;
                if !meta.is_dir() {
                    return Err(format!("工作区已不是目录：{}", root.display()));
                }
                Ok(root)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(self.session_dir(session).join("work"))
            }
            Err(e) => Err(format!("读工作区绑定失败（{}）：{}", mark.display(), e)),
        }
    }

    /// 这个会话是否绑定了外部工作区（看布局投影，不看 meta）。
    fn is_external(&self, session: &str) -> bool {
        self.mark_path(session).is_file()
    }
}

impl Workdirs for FsWorkspace {
    fn is_external(&self, session: &str) -> bool {
        self.mark_path(session).is_file()
    }

    fn lock_shared(&self, session: &str) -> Result<Option<std::fs::File>, String> {
        if !self.is_external(session) {
            return Ok(None);
        }
        let shared = self.shared_root(session)?;
        let canonical = shared
            .canonicalize()
            .map_err(|e| format!("工作区不可达：{}（{}）", shared.display(), e))?;
        // 锁放在产品侧（.home/locks），不写用户目录；键按**落点身份**——同一物理目录的 UNC 写法、
        // 映射盘写法与盘符写法因此共用一把锁（取不到文件 id 时退回规范化文本）。
        let key = crate::kernel::api::locus_of(&canonical)
            .map(|l| l.key())
            .unwrap_or_else(|_| canonical.to_string_lossy().into_owned());
        let product = self.sessions.parent().unwrap_or(&self.sessions);
        let dir = product.join(".home").join("locks");
        std::fs::create_dir_all(&dir).map_err(|e| format!("建工作区锁目录失败：{}", e))?;
        let hash = crate::capabilities::workspace::domain::hash::content_hash(key.as_bytes());
        let path = dir.join(format!("workspace-{}.lock", hash));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|e| format!("打开工作区锁失败（{}）：{}", path.display(), e))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(file)),
                Err(std::fs::TryLockError::WouldBlock) => {
                    if std::time::Instant::now() >= deadline {
                        return Err("工作区正被另一个会话写入，等待超时（稍后重试）".to_string());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(25));
                }
                Err(std::fs::TryLockError::Error(e)) => {
                    return Err(format!("工作区加锁失败：{}", e));
                }
            }
        }
    }

    fn bind(&self, session: &str, root: Option<&Path>) -> Result<(), String> {
        let dir = self.session_dir(session);
        std::fs::create_dir_all(&dir).map_err(|e| format!("建会话目录失败：{}", e))?;
        let mark = dir.join(ROOT_MARK);
        match root {
            Some(p) => {
                if !p.is_absolute() {
                    return Err(format!("工作区根不是绝对路径：{}", p.display()));
                }
                let p = crate::kernel::api::native(p);
                std::fs::write(&mark, p.to_string_lossy().as_bytes())
                    .map_err(|e| format!("写工作区绑定失败（{}）：{}", mark.display(), e))
            }
            None => match std::fs::remove_file(&mark) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("清工作区绑定失败（{}）：{}", mark.display(), e)),
            },
        }
    }

    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        let dir = self.session_dir(session);
        std::fs::create_dir_all(&dir).map_err(|e| format!("建会话目录失败：{}", e))?;
        if !self.is_external(session) {
            std::fs::create_dir_all(dir.join("work"))
                .map_err(|e| format!("建 work 目录失败：{}", e))?;
        }
        for a in agents {
            std::fs::create_dir_all(dir.join(a))
                .map_err(|e| format!("建 agent 沙箱失败：{}", e))?;
        }
        Ok(())
    }

    fn discard(&self, session: &str, agents: &[String]) -> Result<(), String> {
        // 撤销一次**失败的准备**：只删本产品建出的空沙箱 / 空 work/、布局投影与版本库 .work。
        // 调用方只在会话**从未落盘**时调用；.work 不清会留下"有 head 但没有会话"的残档（同名重建会跳过初始导入），用户文件绝不删。
        let dir = self.session_dir(session);
        for a in agents {
            let _ = std::fs::remove_dir(dir.join(a));
        }
        let _ = std::fs::remove_dir_all(dir.join(".work"));
        let _ = std::fs::remove_dir(dir.join("work"));
        let _ = std::fs::remove_file(dir.join(ROOT_MARK));
        let _ = std::fs::remove_dir(&dir);
        Ok(())
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        let dir = self.session_dir(session);
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), dir.join(a));
        }
        Ok(WorkRoots {
            shared: self.shared_root(session)?,
            session_root: dir.clone(),
            agents: map,
            store: dir.join(".work"),
        })
    }

    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        // name 已由 conductor 净化（策略在 conductor）；这里只负责落盘。
        let shared = self.shared_root(session)?;
        if !self.is_external(session) {
            std::fs::create_dir_all(&shared).map_err(|e| e.to_string())?;
        }
        std::fs::write(shared.join(name), bytes).map_err(|e| format!("写入 work 失败：{}", e))
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.shared_root(session)
            .map(|shared| shared.join(name).exists())
            .unwrap_or(false)
    }

    fn list(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        let dir = self.session_dir(session);
        let mut work = Vec::new();
        collect_files(&self.shared_root(session)?, &mut work, "");
        let mut map = BTreeMap::new();
        for a in agents {
            let mut files = Vec::new();
            collect_files(&dir.join(a), &mut files, "");
            map.insert(a.clone(), files);
        }
        Ok(WorkFiles { work, agents: map })
    }

    fn usage(&self, session: &str, agents: &[String]) -> Result<WorkUsage, String> {
        // 同一会话 + 同一 agent 清单在 TTL 内复用结果；前端可能同时开 `@` 菜单与删除确认。
        let key = format!("{}\u{0}{}", session, agents.join("\u{0}"));
        let cache = || self.usage.lock().unwrap_or_else(|e| e.into_inner());
        if let Some((at, value)) = cache().get(&key)
            && at.elapsed() < self.usage_ttl
        {
            return Ok(value.clone());
        }
        let dir = self.session_dir(session);
        let work = area_usage(&self.shared_root(session)?);
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), area_usage(&dir.join(a)));
        }
        let usage = WorkUsage::total(work, map);
        cache().insert(key, (Instant::now(), usage.clone()));
        Ok(usage)
    }
}
