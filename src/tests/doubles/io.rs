//! 目的：内存文件系统与脚本会话替身——内置文件工具的读写落在内存里。
//! 管：读写与越界拒绝、删除、脚本化的会话宿主。
//! 不管：工作区（`workspace.rs`）、路径夹具（`fixtures.rs`）。
//! 联动：`src/capabilities/tools/ports.rs` 的 `SysIo` 与 `src/kernel/ports.rs` 的 `SessionHost`。

use super::*;

impl InMemorySysIo {
    pub(crate) fn new() -> InMemorySysIo {
        InMemorySysIo {
            files: Mutex::new(BTreeMap::new()),
            fail: None,
            lossy: false,
            cut: false,
            active: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
            delay_ms: 0,
            extra: Mutex::new(BTreeMap::new()),
        }
    }

    /// 目的：注入读取标注（lossy = 含非法 UTF-8；cut = 只读到开头）。
    pub(crate) fn marked(mut self, lossy: bool, cut: bool) -> InMemorySysIo {
        self.lossy = lossy;
        self.cut = cut;
        self
    }

    /// 目的：注入失败：read / write 一律返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemorySysIo {
        self.fail = Some(msg.to_string());
        self
    }

    /// 目的：让每次读取慢一点：并发（峰值 > 1）与串行（峰值恒为 1）因此可观测。
    pub(crate) fn slow(mut self, ms: u64) -> InMemorySysIo {
        self.delay_ms = ms;
        self
    }

    /// 目的：指定文件每读一次额外慢多少毫秒。
    pub(crate) fn slow_file(&self, parts: &[&str], ms: u64) {
        self.extra.lock().expect("锁").insert(p(parts), ms);
    }

    /// 目的：同时在读的峰值（并发用例的唯一证据）。
    pub(crate) fn peak_concurrent_reads(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    /// 目的：记一次"进入读取"：整个读取期间计数 +1，并如实记下峰值与耗时。
    fn enter(&self, key: &str) {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(now, Ordering::SeqCst);
        let extra = self
            .extra
            .lock()
            .expect("锁")
            .get(key)
            .copied()
            .unwrap_or(0);
        let ms = self.delay_ms + extra;
        if ms > 0 {
            std::thread::sleep(Duration::from_millis(ms));
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
    }
    pub(crate) fn seed(&self, parts: &[&str], text: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(p(parts), text.to_string());
    }
    pub(crate) fn get(&self, parts: &[&str]) -> Option<String> {
        self.files.lock().expect("锁").get(&p(parts)).cloned()
    }
}

impl SysIo for InMemorySysIo {
    fn read(&self, path: &std::path::Path) -> Result<FileRead, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let key = path.to_string_lossy().into_owned();
        self.enter(&key);
        let text = self
            .files
            .lock()
            .expect("锁")
            .get(&key)
            .cloned()
            .ok_or_else(|| format!("读取失败：{} 不存在", key))?;
        Ok(FileRead {
            bytes: text.len(),
            text,
            lossy: self.lossy,
            cut: self.cut,
        })
    }
    fn list(
        &self,
        path: &std::path::Path,
    ) -> Result<Vec<crate::capabilities::tools::ports::DirEntry>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 内存替身：把已 seed 的路径按"父目录等于该目录"筛出来（只报直接子项）。
        let dir = path.to_string_lossy().into_owned();
        let files = self.files.lock().expect("锁");
        let mut out: Vec<crate::capabilities::tools::ports::DirEntry> = Vec::new();
        for (k, v) in files.iter() {
            let Some((parent, name)) = k.rsplit_once(['/', '\\']) else {
                continue;
            };
            if parent != dir.trim_end_matches(['/', '\\']) {
                continue;
            }
            out.push(crate::capabilities::tools::ports::DirEntry {
                name: name.to_string(),
                is_dir: false,
                bytes: v.len() as u64,
            });
        }
        if out.is_empty() {
            return Err(format!("列目录失败：{} 不是目录", dir));
        }
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn within(&self, root: &std::path::Path, path: &std::path::Path) -> Result<(), String> {
        if path.starts_with(root) {
            Ok(())
        } else {
            Err(format!("路径越过允许的根：{}", path.display()))
        }
    }

    fn is_dir(&self, path: &std::path::Path) -> Result<bool, String> {
        let key = path.to_string_lossy().into_owned();
        let prefix = format!(
            "{}{}",
            key.trim_end_matches(['/', '\\']),
            std::path::MAIN_SEPARATOR
        );
        let files = self.files.lock().expect("锁");
        if files.contains_key(&key) {
            return Ok(false);
        }
        if files.keys().any(|k| k.starts_with(&prefix)) {
            return Ok(true);
        }
        Err(format!("看 {} 失败：不存在", path.display()))
    }

    fn delete(&self, path: &std::path::Path, recursive: bool) -> Result<(), String> {
        let key = path.to_string_lossy().into_owned();
        let prefix = format!(
            "{}{}",
            key.trim_end_matches(['/', '\\']),
            std::path::MAIN_SEPARATOR
        );
        let mut files = self.files.lock().expect("锁");
        if files.remove(&key).is_some() {
            return Ok(());
        }
        if !files.keys().any(|k| k.starts_with(&prefix)) {
            return Err(format!("删除 {} 失败：不存在", path.display()));
        }
        if !recursive {
            return Err(format!(
                "{} 是目录；要整棵删除，加 recursive=true",
                path.display()
            ));
        }
        files.retain(|k, _| !k.starts_with(&prefix));
        Ok(())
    }

    fn write(&self, path: &std::path::Path, content: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files
            .lock()
            .expect("锁")
            .insert(path.to_string_lossy().into_owned(), content.to_string());
        Ok(())
    }
}

impl ScriptedHost {
    pub(crate) fn new(replies: Vec<&str>) -> ScriptedHost {
        ScriptedHost {
            sent: Arc::new(Mutex::new(Vec::new())),
            replies: Mutex::new(replies.into_iter().map(str::to_string).collect()),
        }
    }
}

struct ScriptedSession {
    sent: Arc<Mutex<Vec<String>>>,
    replies: Mutex<std::collections::VecDeque<String>>,
}

impl crate::kernel::ports::Session for ScriptedSession {
    fn send(&mut self, line: &str) -> Result<(), String> {
        self.sent.lock().expect("锁").push(line.to_string());
        Ok(())
    }
    fn recv(&mut self) -> Result<String, String> {
        self.replies
            .lock()
            .expect("锁")
            .pop_front()
            .ok_or_else(|| "脚本没有更多应答".to_string())
    }
    fn kill(&mut self) {
        self.sent.lock().expect("锁").push("<kill>".to_string());
    }
}

impl crate::kernel::ports::SessionHost for ScriptedHost {
    fn open(
        &self,
        _spec: &crate::kernel::ports::SessionSpec,
    ) -> Result<Box<dyn crate::kernel::ports::Session>, String> {
        let replies: std::collections::VecDeque<String> =
            self.replies.lock().expect("锁").clone().into();
        Ok(Box::new(ScriptedSession {
            sent: Arc::clone(&self.sent),
            replies: Mutex::new(replies),
        }))
    }
}

/// 目的：测试里登记模型的最小字段（思考强度默认空；要测思考的用例再改字段）。
pub(crate) fn model_draft(
    id: &str,
    name: &str,
    api_model: &str,
    provider: &str,
    note: &str,
    context: u64,
) -> crate::capabilities::registry::api::ModelDraft {
    crate::capabilities::registry::api::ModelDraft {
        id: id.to_string(),
        name: name.to_string(),
        api_model: api_model.to_string(),
        provider: provider.to_string(),
        note: note.to_string(),
        tools: crate::capabilities::llm::api::ToolMode::Envelope,
        context,
        thinking: Default::default(),
    }
}
