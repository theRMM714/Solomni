//! 会话历史落盘：session/<名字>/meta.yaml + transcript.jsonl（实现 core 的 HistoryStore 端口）。
//! 名字即目录名（core 已校验）；流水只追加，回档将来以 rewind 记录追加，不物理删行。

use crate::core::history::{HistoryView, SessionMeta};
use crate::core::ports::HistoryStore;
use std::io::Write;
use std::path::PathBuf;

pub struct FsHistory {
    dir: PathBuf,
}

impl FsHistory {
    pub fn new(dir: PathBuf) -> FsHistory {
        FsHistory { dir }
    }

    fn session_dir(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }
}

impl HistoryStore for FsHistory {
    fn create(&self, meta: &SessionMeta) -> Result<(), String> {
        let d = self.session_dir(&meta.name);
        std::fs::create_dir_all(&d).map_err(|e| format!("建会话目录失败：{}", e))?;
        let text = serde_yaml::to_string(meta).map_err(|e| e.to_string())?;
        std::fs::write(d.join("meta.yaml"), text).map_err(|e| format!("写会话元信息失败：{}", e))
    }

    fn append(&self, name: &str, events: &[serde_json::Value]) -> Result<(), String> {
        let d = self.session_dir(name);
        std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
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

    fn list(&self) -> Result<Vec<HistoryView>, String> {
        let mut out = Vec::new();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(e.to_string()),
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if !p.is_dir() {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(p.join("meta.yaml")) else { continue };
            let Ok(meta) = serde_yaml::from_str::<SessionMeta>(&text) else { continue };
            let done = std::fs::read_to_string(p.join("transcript.jsonl"))
                .map(|t| t.contains("\"type\":\"ended\""))
                .unwrap_or(false);
            out.push(HistoryView { name: meta.name, mode: meta.mode, ts: meta.ts, done });
        }
        out.sort_by(|a, b| b.ts.cmp(&a.ts));
        Ok(out)
    }

    fn load(&self, name: &str) -> Result<(SessionMeta, Vec<serde_json::Value>), String> {
        let d = self.session_dir(name);
        let text = std::fs::read_to_string(d.join("meta.yaml")).map_err(|_| format!("无此会话：{}", name))?;
        let meta: SessionMeta = serde_yaml::from_str(&text).map_err(|e| format!("会话 meta.yaml 非法：{}", e))?;
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
        let d = self.session_dir(name);
        if !d.is_dir() {
            return Ok(false);
        }
        std::fs::remove_dir_all(&d).map_err(|e| e.to_string())?;
        Ok(true)
    }
}
