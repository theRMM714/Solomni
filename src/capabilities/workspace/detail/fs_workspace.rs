//! 目的：工作区落盘——会话目录下的共享区、各 agent 沙箱与外部工作区绑定投影。
//! 管：目录布局（`work/`、沙箱、`.work-root`）与文件清单 / 用量。
//! 不管：版本库提交语义（在 service/domain）与会话 meta 的读写（在 session）。
//! 联动：由 workspace 的 service.rs 持有（R12）；布局见 docs/session/session-model.md。

use crate::capabilities::workspace::api::{AreaUsage, WorkFiles, WorkRoots, WorkUsage};
use crate::capabilities::workspace::ports::Workdirs;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 每个根最多列这么多条（超出即截断；@ 菜单不需要无限长，也避免大目录拖慢前端）。
const MAX_LISTED: usize = 300;

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

/// 递归累加一个区的文件数与字节（目录不计；单个文件 stat 失败按 0 计，不因此挡下整次统计）。
fn collect_usage(root: &Path, files: &mut usize, bytes: &mut u64) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
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
}

impl FsWorkspace {
    pub fn new(sessions: PathBuf) -> FsWorkspace {
        FsWorkspace { sessions }
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
        // 锁放在产品侧（.home/locks），不写用户目录；同一 canonical 根共用一个文件名。
        let product = self.sessions.parent().unwrap_or(&self.sessions);
        let dir = product.join(".home").join("locks");
        std::fs::create_dir_all(&dir).map_err(|e| format!("建工作区锁目录失败：{}", e))?;
        let hash = crate::capabilities::workspace::domain::hash::content_hash(
            canonical.to_string_lossy().as_bytes(),
        );
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
                    return Err(format!("工作区加锁失败：{}", e))
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
        // 只删**空**目录与本产品写下的布局投影：有任何内容都留着（用户的东西绝不因为一次失败的创建被删）。
        let dir = self.session_dir(session);
        for a in agents {
            let _ = std::fs::remove_dir(dir.join(a));
        }
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
        let dir = self.session_dir(session);
        let work = area_usage(&self.shared_root(session)?);
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), area_usage(&dir.join(a)));
        }
        Ok(WorkUsage::total(work, map))
    }
}
