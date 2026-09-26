//! 工作区落盘：session/<工作名>/ 下的 work 与各 agent 沙箱（实现 core 的 Workspace 端口）。
//! 目录布局机制集中在这里；路径一律用路径组件拼接（交给运行环境）。

use crate::core::ports::Workspace;
use crate::core::workspace::{WorkFiles, WorkRoots};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// 每个根最多列这么多条（超出即截断；@ 菜单不需要无限长，也避免大目录拖慢前端）。
const MAX_LISTED: usize = 300;

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
}

impl Workspace for FsWorkspace {
    fn prepare(&self, session: &str, agents: &[String]) -> Result<(), String> {
        let dir = self.session_dir(session);
        std::fs::create_dir_all(dir.join("work"))
            .map_err(|e| format!("建 work 目录失败：{}", e))?;
        for a in agents {
            std::fs::create_dir_all(dir.join(a))
                .map_err(|e| format!("建 agent 沙箱失败：{}", e))?;
        }
        Ok(())
    }

    fn roots(&self, session: &str, agents: &[String]) -> Result<WorkRoots, String> {
        let dir = self.session_dir(session);
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), dir.join(a));
        }
        Ok(WorkRoots {
            shared: dir.join("work"),
            agents: map,
        })
    }

    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        // name 已由 core 净化（策略在 core）；这里只负责落盘。
        let dir = self.session_dir(session).join("work");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("写入 work 失败：{}", e))
    }

    fn work_has(&self, session: &str, name: &str) -> bool {
        self.session_dir(session).join("work").join(name).exists()
    }

    fn list(&self, session: &str, agents: &[String]) -> Result<WorkFiles, String> {
        let dir = self.session_dir(session);
        let mut work = Vec::new();
        collect_files(&dir.join("work"), &mut work, "");
        let mut map = BTreeMap::new();
        for a in agents {
            let mut files = Vec::new();
            collect_files(&dir.join(a), &mut files, "");
            map.insert(a.clone(), files);
        }
        Ok(WorkFiles { work, agents: map })
    }
}
