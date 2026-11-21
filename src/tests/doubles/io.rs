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
            fail_paths: Mutex::new(BTreeSet::new()),
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

    /// 目的：只让某个路径写失败（事务的"第 N 个失败"用例）。
    pub(crate) fn fail_on(&self, parts: &[&str]) {
        self.fail_paths.lock().expect("锁").insert(p(parts));
    }

    fn failure_for(&self, path: &std::path::Path) -> Option<String> {
        if let Some(m) = &self.fail {
            return Some(m.clone());
        }
        let key = path.to_string_lossy().into_owned();
        self.fail_paths
            .lock()
            .expect("锁")
            .contains(&key)
            .then(|| "磁盘故障".to_string())
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
        let hash = crate::capabilities::workspace::api::content_hash(text.as_bytes());
        Ok(FileRead {
            bytes: text.len(),
            text,
            lossy: self.lossy,
            cut: self.cut,
            hash,
        })
    }

    fn read_page(
        &self,
        path: &std::path::Path,
        offset: u32,
        limit: u32,
    ) -> Result<crate::capabilities::tools::ports::FilePage, String> {
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
        let lines: Vec<&str> = text.lines().collect();
        let total = lines.len() as u32;
        let offset = offset.max(1);
        let selected: Vec<&str> = lines
            .iter()
            .skip((offset - 1) as usize)
            .take(limit.max(1) as usize)
            .copied()
            .collect();
        Ok(crate::capabilities::tools::ports::FilePage {
            text: selected.join("\n"),
            bytes: text.len(),
            lossy: self.lossy,
            window_cut: false,
            line_cut: false,
            hash: crate::capabilities::workspace::api::content_hash(text.as_bytes()),
            total_lines: total,
        })
    }

    fn fingerprint(&self, path: &std::path::Path) -> Result<Option<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let key = path.to_string_lossy().into_owned();
        Ok(self
            .files
            .lock()
            .expect("锁")
            .get(&key)
            .map(|text| crate::capabilities::workspace::api::content_hash(text.as_bytes())))
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

    fn mutate(
        &self,
        path: &std::path::Path,
        expect: &crate::capabilities::tools::ports::Expect,
        mutation: crate::capabilities::tools::ports::Mutation,
    ) -> Result<
        crate::capabilities::tools::ports::WriteReceipt,
        crate::capabilities::tools::ports::WriteFault,
    > {
        use crate::capabilities::tools::ports::{WriteFault, WriteReceipt};
        if let Some(m) = self.failure_for(path) {
            return Err(WriteFault::io(path, &m));
        }
        let key = path.to_string_lossy().into_owned();
        let mut files = self.files.lock().expect("锁");
        let current = files.get(&key).cloned();
        check_memory_expect(path, expect, current.as_deref())?;
        let (text, changed, replaced) = memory_mutation(path, current.as_deref(), mutation)?;
        let version = crate::capabilities::workspace::api::content_hash(text.as_bytes());
        let total_lines = text.lines().count() as u32;
        files.insert(key, text);
        Ok(WriteReceipt {
            version,
            total_lines,
            changed,
            replaced,
        })
    }

    fn transact(
        &self,
        ops: Vec<crate::capabilities::tools::ports::WriteOp>,
    ) -> Result<
        Vec<crate::capabilities::tools::ports::WriteReceipt>,
        crate::capabilities::tools::ports::WriteFault,
    > {
        use crate::capabilities::tools::ports::{WriteFault, WriteReceipt};
        let mut files = self.files.lock().expect("锁");
        let mut next = files.clone();
        let mut seen = std::collections::BTreeSet::new();
        let mut receipts: Vec<WriteReceipt> = Vec::with_capacity(ops.len());
        for op in ops {
            if let Some(m) = self.failure_for(&op.path) {
                return Err(WriteFault::io(&op.path, &m));
            }
            let key = op.path.to_string_lossy().into_owned();
            if !seen.insert(key.clone()) {
                return Err(WriteFault::new(
                    "FS_DUPLICATE_PATH",
                    format!("同一个路径在一次事务里出现了两次：{}；没有写盘。", key),
                ));
            }
            let current = next.get(&key).cloned();
            check_memory_expect(&op.path, &op.expect, current.as_deref())?;
            let (text, changed, replaced) =
                memory_mutation(&op.path, current.as_deref(), op.mutation)?;
            let version = crate::capabilities::workspace::api::content_hash(text.as_bytes());
            let total_lines = text.lines().count() as u32;
            next.insert(key, text);
            receipts.push(WriteReceipt {
                version,
                total_lines,
                changed,
                replaced,
            });
        }
        *files = next;
        Ok(receipts)
    }
}

/// 目的：内存替身的 CAS 校验（与 `FsSysIo` 同一语义）。
fn check_memory_expect(
    path: &std::path::Path,
    expect: &crate::capabilities::tools::ports::Expect,
    current: Option<&str>,
) -> Result<(), crate::capabilities::tools::ports::WriteFault> {
    use crate::capabilities::tools::ports::{Expect, WriteFault};
    match expect {
        Expect::Absent => {
            if current.is_some() {
                return Err(WriteFault::already_exists(path));
            }
        }
        Expect::Version(want) => match current {
            None => return Err(WriteFault::missing(path)),
            Some(text) => {
                let now = crate::capabilities::workspace::api::content_hash(text.as_bytes());
                if &now != want {
                    return Err(WriteFault::stale(path, want, &now));
                }
            }
        },
    }
    Ok(())
}

/// 目的：内存替身的变换（与 `FsSysIo` 同一语义）。
fn memory_mutation(
    path: &std::path::Path,
    current: Option<&str>,
    mutation: crate::capabilities::tools::ports::Mutation,
) -> Result<
    (
        String,
        Vec<crate::capabilities::tools::api::LineRange>,
        usize,
    ),
    crate::capabilities::tools::ports::WriteFault,
> {
    use crate::capabilities::tools::api::LineRange;
    use crate::capabilities::tools::domain::systool::{LiteralFault, apply_literal_edit};
    use crate::capabilities::tools::ports::{Mutation, WriteFault};
    match mutation {
        Mutation::Replace(text) => {
            let total = text.lines().count() as u32;
            let changed = if total == 0 {
                Vec::new()
            } else {
                vec![LineRange::new(1, total)]
            };
            Ok((text, changed, 0))
        }
        Mutation::Edit { old, new, all } => {
            let Some(hay) = current else {
                return Err(WriteFault::missing(path));
            };
            match apply_literal_edit(hay, &old, &new, all) {
                Ok((out, changed, n)) => Ok((
                    out,
                    changed
                        .into_iter()
                        .map(|(from, to)| LineRange::new(from, to))
                        .collect(),
                    n,
                )),
                Err(LiteralFault::NotFound) => Err(WriteFault::anchor_not_found(path)),
                Err(LiteralFault::Ambiguous { lines }) => {
                    Err(WriteFault::anchor_ambiguous(path, lines.len()))
                }
            }
        }
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
) -> crate::capabilities::catalog::registry::api::ModelDraft {
    crate::capabilities::catalog::registry::api::ModelDraft {
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
