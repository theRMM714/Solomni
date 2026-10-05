//! 会话历史落盘：session/<名字>/meta.yaml + transcript.jsonl（实现本能力的 HistoryStore 端口）。
//! 布局：顶层会话 = session/<名字>/；子会话（meta.parent 非空）= session/<父>/children/<名字>/
//! —— 位置本身就是归属，删父会话 = 删一个目录。名字即目录名（conductor 已校验）。
//! 子会话可以再往下嵌套（代理建的子会话里还能有它自己的子会话），所以 children/ 的查找与遍历
//! 必须是**任意深度**的：整棵 children/ 树都属于同一个顶层 work。
//! 流水只追加；留档追加一条 rewind 记录，删除 / 恢复整体重写。

use crate::capabilities::session::api::{HistoryView, SessionMeta};
use crate::capabilities::session::ports::HistoryStore;
use std::io::Write;
use std::path::PathBuf;

pub struct FsHistory {
    dir: PathBuf,
}

impl FsHistory {
    pub fn new(dir: PathBuf) -> FsHistory {
        FsHistory { dir }
    }

    /// 顶层会话目录里专放子会话的那一层。
    const CHILDREN: &'static str = "children";

    /// 顶层会话目录。
    fn top_dir(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// 一个会话（顶层或子会话）的落点：`meta.parent` 有值就落在**父会话目录**的 children/ 下。
    /// 父会话自己也可能嵌套（`session/<顶层>/children/<父>/`），所以先按名字定位父目录再往下拼；
    /// 归属不从名字里解析分隔符（工作名本身可以含 `-`）。
    fn dir_of(&self, meta: &SessionMeta) -> PathBuf {
        match &meta.parent {
            Some(p) => match self.find(p) {
                Some(pd) => pd.join(Self::CHILDREN).join(&meta.name),
                // 父会话不在盘上（不该发生）：退回顶层同名目录，读写会如实报"无此会话"。
                None => self.top_dir(&meta.name),
            },
            None => self.top_dir(&meta.name),
        }
    }

    /// 只按名字定位（append / load / delete 用）：先当顶层找，再在**任意深度**的 children/ 树里找。
    /// 目录名就是会话名，所以不需要从名字里猜父会话。
    fn find(&self, name: &str) -> Option<PathBuf> {
        let top = self.top_dir(name);
        if top.join("meta.yaml").is_file() {
            return Some(top);
        }
        // dir = 某个会话目录；它的子会话在 dir/children/ 下，子会话还能再往下嵌套。
        fn under(dir: &std::path::Path, name: &str) -> Option<PathBuf> {
            for kid in std::fs::read_dir(dir.join(FsHistory::CHILDREN))
                .ok()?
                .flatten()
            {
                let p = kid.path();
                if !p.is_dir() {
                    continue;
                }
                if p.file_name().and_then(|n| n.to_str()) == Some(name)
                    && p.join("meta.yaml").is_file()
                {
                    return Some(p);
                }
                if let Some(found) = under(&p, name) {
                    return Some(found);
                }
            }
            None
        }
        for e in std::fs::read_dir(&self.dir).ok()?.flatten() {
            if e.path().is_dir() {
                if let Some(found) = under(&e.path(), name) {
                    return Some(found);
                }
            }
        }
        None
    }

    /// 只读一个会话的 meta.yaml（不回放流水）：拿不到就如实报"无此会话"。
    fn read_meta(dir: &std::path::Path, name: &str) -> Result<SessionMeta, String> {
        let text = std::fs::read_to_string(dir.join("meta.yaml"))
            .map_err(|_| format!("无此会话：{}", name))?;
        yaml_serde::from_str(&text).map_err(|e| format!("会话 meta.yaml 非法：{}", e))
    }

    /// 读一个会话目录的列表视图（meta 缺失或非法 = None）。
    fn read_view(dir: &std::path::Path) -> Option<HistoryView> {
        let text = std::fs::read_to_string(dir.join("meta.yaml")).ok()?;
        let meta = yaml_serde::from_str::<SessionMeta>(&text).ok()?;
        let done = std::fs::read_to_string(dir.join("transcript.jsonl"))
            .map(|t| t.contains("\"type\":\"ended\""))
            .unwrap_or(false);
        Some(HistoryView {
            name: meta.name,
            mode: meta.mode,
            ts: meta.ts,
            done,
            // 档位来自 meta 的 exec 段：列表视图据此提示"环境已变"，不拦打开。
            exec: meta.exec,
            // 编排者：子会话在侧栏里缩进挂在父会话下。
            parent: meta.parent,
            // 运行态：侧栏据此标出已暂停 / 已关闭的会话。
            run: meta.run,
        })
    }
}

impl HistoryStore for FsHistory {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        let d = self.dir_of(meta);
        std::fs::create_dir_all(&d).map_err(|e| format!("建会话目录失败：{}", e))?;
        let text = yaml_serde::to_string(meta).map_err(|e| e.to_string())?;
        std::fs::write(d.join("meta.yaml"), text).map_err(|e| format!("写会话元信息失败：{}", e))
    }

    fn save_meta(&self, meta: &SessionMeta) -> Result<(), String> {
        // 与 create 同一落点：meta.parent 是位置的唯一判据。
        self.create(meta)
    }

    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        let d = self
            .find(name)
            .ok_or_else(|| format!("无此会话：{}", name))?;
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(d.join("transcript.jsonl"))
            .map_err(|e| e.to_string())?;
        for ev in events {
            let line = serde_json::to_string(ev).map_err(|e| e.to_string())?;
            writeln!(f, "{}", line).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn replace(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        let d = self
            .find(name)
            .ok_or_else(|| format!("无此会话：{}", name))?;
        let path = d.join("transcript.jsonl");
        let tmp = d.join("transcript.jsonl.tmp");
        let mut text = String::new();
        for ev in events {
            text.push_str(&serde_json::to_string(ev).map_err(|e| e.to_string())?);
            text.push('\n');
        }
        if let Err(e) = std::fs::write(&tmp, text) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("重写流水失败：{}", e));
        }
        if let Err(e) = std::fs::rename(&tmp, &path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(format!("替换流水失败：{}", e));
        }
        Ok(())
    }

    fn list(&self) -> Result<Vec<HistoryView>, String> {
        // 子会话在 <会话目录>/children/ 下，并可再往下嵌套：整棵 children/ 树都收进来。
        fn walk(dir: &std::path::Path, out: &mut Vec<HistoryView>) {
            if let Some(v) = FsHistory::read_view(dir) {
                out.push(v);
            }
            if let Ok(kids) = std::fs::read_dir(dir.join(FsHistory::CHILDREN)) {
                for kid in kids.flatten() {
                    if kid.path().is_dir() {
                        walk(&kid.path(), out);
                    }
                }
            }
        }
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.to_string()),
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                walk(&p, &mut out);
            }
        }
        out.sort_by_key(|a| std::cmp::Reverse(a.ts));
        Ok(out)
    }

    fn meta(&self, name: &str) -> Result<SessionMeta, String> {
        let d = self
            .find(name)
            .ok_or_else(|| format!("无此会话：{}", name))?;
        Self::read_meta(&d, name)
    }

    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let d = self
            .find(name)
            .ok_or_else(|| format!("无此会话：{}", name))?;
        let meta: SessionMeta = Self::read_meta(&d, name)?;
        let mut events = Vec::new();
        if let Ok(t) = std::fs::read_to_string(d.join("transcript.jsonl")) {
            for line in t.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
                    events.push(v);
                }
            }
        }
        Ok((meta, events))
    }

    fn delete(&self, name: &str) -> Result<bool, String> {
        let Some(d) = self.find(name) else {
            return Ok(false);
        };
        std::fs::remove_dir_all(&d).map_err(|e| e.to_string())?;
        Ok(true)
    }
}
