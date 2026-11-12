//! 目的：内存工作区与版本库替身——目录布局与内容寻址对象树都不碰盘。
//! 管：共享区/私有区根、写入与清单、版本库的文件原语与提交记录。
//! 不管：文件系统替身（`io.rs`）、真实适配器（`src/capabilities/workspace/detail/`）。
//! 联动：`src/capabilities/workspace/ports.rs` 的 `Workdirs` 与 `WorkStore`。

use super::*;

impl InMemoryWorkspace {
    pub(crate) fn new() -> InMemoryWorkspace {
        InMemoryWorkspace {
            files: Mutex::new(BTreeMap::new()),
            fail: None,
        }
    }

    /// 目的：注入失败：prepare / roots / write_work / list 一律返回该原因（work_has 是布尔查询，不受影响）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryWorkspace {
        self.fail = Some(msg.to_string());
        self
    }
    /// 目的：直接放一个文件（模拟落盘），键与真实布局同构：<session>/work/<名字> 或 <session>/<agent>/<相对路径>。
    pub(crate) fn seed(&self, session: &str, area: &str, rel: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/{}/{}", session, area, rel), Vec::new());
    }

    /// 目的：放一个**指定大小**的文件（用量断言用）：键与真实布局同构。
    pub(crate) fn seed_bytes(&self, session: &str, area: &str, rel: &str, bytes: usize) {
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/{}/{}", session, area, rel), vec![0u8; bytes]);
    }
}

impl Workdirs for InMemoryWorkspace {
    fn is_external(&self, _session: &str) -> bool {
        false
    }

    fn lock_shared(&self, _session: &str) -> Result<Option<std::fs::File>, String> {
        Ok(None)
    }

    fn bind(&self, _session: &str, _root: Option<&std::path::Path>) -> Result<(), String> {
        // 内存实现不做布局投影；外部工作区用例走真实 FsWorkspace。
        Ok(())
    }
    fn prepare(&self, _session: &str, _agents: &[String]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(())
    }
    fn discard(&self, _session: &str, _agents: &[String]) -> Result<(), String> {
        // 内存实现里没有目录可删。
        Ok(())
    }

    fn roots(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::capabilities::workspace::api::WorkRoots, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 与 FsWorkspace 同构的**绝对**路径（以当前目录为锚；只读 env，不碰盘）。
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), abs(&[session, a]));
        }
        Ok(crate::capabilities::workspace::api::WorkRoots {
            shared: abs(&[session, "work"]),
            session_root: abs(&[session]),
            agents: map,
            store: abs(&[session, ".work"]),
        })
    }
    fn write_work(&self, session: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files
            .lock()
            .expect("锁")
            .insert(format!("{}/work/{}", session, name), bytes.to_vec());
        Ok(())
    }
    fn work_has(&self, session: &str, name: &str) -> bool {
        self.files
            .lock()
            .expect("锁")
            .contains_key(&format!("{}/work/{}", session, name))
    }
    fn list(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::capabilities::workspace::api::WorkFiles, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        // 内存实现没有目录树，直接按前缀扫键（相对路径原样返回，/ 分隔）。
        let files = self.files.lock().expect("锁");
        let pick = |prefix: &str| -> Vec<String> {
            files
                .keys()
                .filter_map(|k| k.strip_prefix(prefix).map(|s| s.to_string()))
                .collect::<Vec<_>>()
        };
        let mut work = pick(&format!("{}/work/", session));
        work.sort();
        let mut map = BTreeMap::new();
        for a in agents {
            let mut got = pick(&format!("{}/{}/", session, a));
            got.sort();
            map.insert(a.clone(), got);
        }
        Ok(crate::capabilities::workspace::api::WorkFiles { work, agents: map })
    }

    fn usage(
        &self,
        session: &str,
        agents: &[String],
    ) -> Result<crate::capabilities::workspace::api::WorkUsage, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        use crate::capabilities::workspace::api::AreaUsage;
        let files = self.files.lock().expect("锁");
        let area = |prefix: &str| -> AreaUsage {
            let mut out = AreaUsage::default();
            for (k, v) in files.iter() {
                if k.starts_with(prefix) {
                    out.files += 1;
                    out.bytes += v.len() as u64;
                }
            }
            out
        };
        let work = area(&format!("{}/work/", session));
        let mut map = BTreeMap::new();
        for a in agents {
            map.insert(a.clone(), area(&format!("{}/{}/", session, a)));
        }
        Ok(crate::capabilities::workspace::api::WorkUsage::total(
            work, map,
        ))
    }
}

/// 目的：内存版本库：文件原语 + 内容寻址对象、提交记录、head 与各 agent 的拉取基线。键一律用「根路径字符串|相对键」，与真实适配器同一份语义，供端口契约测试与协调业务用例使用。
#[derive(Default)]
pub(crate) struct InMemoryWorkStore {
    pub(crate) files: Mutex<BTreeMap<String, Vec<u8>>>,
    pub(crate) objects: Mutex<BTreeMap<String, Vec<u8>>>,
    pub(crate) commits: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Commit>>,
    pub(crate) refs: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Ref>>,
    pub(crate) index: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Index>>,
    pub(crate) pending: Mutex<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Pending>>,
    pub(crate) next_ids: Mutex<BTreeMap<String, u64>>,
    pub(crate) fail: Option<String>,
    pub(crate) fail_times: Mutex<BTreeMap<String, usize>>,
    // 按方法 + 键子串注入失败（例如只让某一条路径的 copy 失败）。
    pub(crate) fail_on: Mutex<BTreeMap<String, (String, usize)>>,
}

fn ws_key(root: &std::path::Path, rel: &str) -> String {
    format!("{}|{}", root.to_string_lossy(), rel)
}

fn scan_bytes(bytes: &[u8], preview: Option<usize>) -> FileVersion {
    let mut scanner = AssetScanner::new(preview);
    scanner.feed(bytes);
    let scanned = scanner.finish();
    FileVersion {
        hash: scanned.hash,
        size: scanned.size,
        text: scanned.text,
        truncated: scanned.truncated,
        too_large: false,
    }
}

impl InMemoryWorkStore {
    pub(crate) fn new() -> InMemoryWorkStore {
        InMemoryWorkStore::default()
    }

    /// 目的：注入失败：任何端口调用都返回该原因（端口契约测试用）。
    pub(crate) fn fail_with(mut self, msg: &str) -> InMemoryWorkStore {
        self.fail = Some(msg.to_string());
        self
    }

    /// 目的：直接放一个文件（模拟主副本 / 沙箱里已有内容）。
    pub(crate) fn seed(&self, root: &std::path::Path, rel: &str, text: &str) {
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(root, rel), text.as_bytes().to_vec());
    }

    /// 目的：按方法名注入 N 次失败，用于把提交/恢复钉在指定一步。
    pub(crate) fn fail_times(&self, method: &str, times: usize) {
        self.fail_times
            .lock()
            .expect("锁")
            .insert(method.to_string(), times);
    }

    /// 目的：只在键包含 needle 的调用上注入失败（按次数递减）。
    pub(crate) fn fail_on(&self, method: &str, needle: &str, times: usize) {
        self.fail_on
            .lock()
            .expect("锁")
            .insert(method.to_string(), (needle.to_string(), times));
    }

    fn enter(&self, method: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let mut counts = self.fail_times.lock().expect("锁");
        if let Some(n) = counts.get_mut(method) {
            if *n > 0 {
                *n -= 1;
                return Err(format!("注入失败：{}", method));
            }
        }
        Ok(())
    }

    /// 目的：按方法 + 键子串注入失败；不匹配的调用不受影响。
    fn enter_key(&self, method: &str, key: &str) -> Result<(), String> {
        let mut map = self.fail_on.lock().expect("锁");
        if let Some((needle, left)) = map.get_mut(method) {
            if *left > 0 && key.contains(needle.as_str()) {
                *left -= 1;
                return Err(format!("注入失败：{}（{}）", method, key));
            }
        }
        Ok(())
    }

    /// 目的：测试直接查看/布置沙箱内容用；生产资产路径走 scan / ingest / copy。
    pub(crate) fn read_under(
        &self,
        root: &std::path::Path,
        rel: &str,
    ) -> Result<Option<Vec<u8>>, String> {
        self.enter("read_under")?;
        Ok(self
            .files
            .lock()
            .expect("锁")
            .get(&ws_key(root, rel))
            .cloned())
    }

    /// 目的：测试直接布置沙箱内容用；生产写入走 ingest / copy。
    pub(crate) fn write_under(
        &self,
        root: &std::path::Path,
        rel: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        self.enter("write_under")?;
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(root, rel), bytes.to_vec());
        Ok(())
    }
}

impl WorkStore for InMemoryWorkStore {
    fn remove_under(&self, root: &std::path::Path, rel: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.files.lock().expect("锁").remove(&ws_key(root, rel));
        Ok(())
    }

    fn list(&self, root: &std::path::Path) -> Result<Vec<String>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", root.to_string_lossy());
        let files = self.files.lock().expect("锁");
        let mut out: Vec<String> = files
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix).map(|s| s.to_string()))
            .collect();
        out.sort();
        Ok(out)
    }

    fn stat_under(&self, root: &std::path::Path, rel: &str) -> Result<Option<u64>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .files
            .lock()
            .expect("锁")
            .get(&ws_key(root, rel))
            .map(|v| v.len() as u64))
    }

    fn scan_under(
        &self,
        root: &std::path::Path,
        rel: &str,
        preview: Option<usize>,
    ) -> Result<Option<FileVersion>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let files = self.files.lock().expect("锁");
        Ok(files
            .get(&ws_key(root, rel))
            .map(|bytes| scan_bytes(bytes, preview)))
    }

    fn stat_object(&self, store: &std::path::Path, hash: &str) -> Result<u64, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.objects
            .lock()
            .expect("锁")
            .get(&ws_key(store, hash))
            .map(|v| v.len() as u64)
            .ok_or_else(|| format!("对象不存在：{}", hash))
    }

    fn scan_object(
        &self,
        store: &std::path::Path,
        hash: &str,
        preview: Option<usize>,
    ) -> Result<FileVersion, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.objects
            .lock()
            .expect("锁")
            .get(&ws_key(store, hash))
            .map(|bytes| scan_bytes(bytes, preview))
            .ok_or_else(|| format!("对象不存在：{}", hash))
    }

    fn ingest_under(
        &self,
        store: &std::path::Path,
        root: &std::path::Path,
        rel: &str,
        expected_hash: &str,
        max_bytes: u64,
    ) -> Result<u64, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("ingest_under")?;
        let bytes = self
            .files
            .lock()
            .expect("锁")
            .get(&ws_key(root, rel))
            .cloned()
            .ok_or_else(|| format!("沙箱里没有：{}", rel))?;
        if bytes.len() as u64 > max_bytes {
            return Err(format!(
                "文件超过单文件上限（{} 字节 > {} 字节）：{}",
                bytes.len(),
                max_bytes,
                rel
            ));
        }
        let scanned = scan_bytes(&bytes, None);
        if scanned.hash != expected_hash {
            return Err(format!("文件在提交过程中被改动，请重试：{}", rel));
        }
        self.objects
            .lock()
            .expect("锁")
            .entry(ws_key(store, expected_hash))
            .or_insert(bytes);
        Ok(scanned.size)
    }

    fn copy_object_to_under(
        &self,
        store: &std::path::Path,
        hash: &str,
        root: &std::path::Path,
        rel: &str,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("copy_object_to_under")?;
        self.enter_key(
            "copy_object_to_under",
            &format!("{}|{}", root.to_string_lossy(), rel),
        )?;
        let bytes = self
            .objects
            .lock()
            .expect("锁")
            .get(&ws_key(store, hash))
            .cloned()
            .ok_or_else(|| format!("对象不存在：{}", hash))?;
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(root, rel), bytes);
        Ok(())
    }

    fn copy_under_to_under(
        &self,
        from_root: &std::path::Path,
        from_rel: &str,
        to_root: &std::path::Path,
        to_rel: &str,
        expected_hash: &str,
    ) -> Result<u64, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("copy_under_to_under")?;
        self.enter_key(
            "copy_under_to_under",
            &format!("{}|{}", from_root.to_string_lossy(), from_rel),
        )?;
        let bytes = self
            .files
            .lock()
            .expect("锁")
            .get(&ws_key(from_root, from_rel))
            .cloned()
            .ok_or_else(|| format!("源文件不存在：{}", from_rel))?;
        let scanned = scan_bytes(&bytes, None);
        if scanned.hash != expected_hash {
            return Err(format!("源文件在复制过程中被改动，请重试：{}", from_rel));
        }
        self.files
            .lock()
            .expect("锁")
            .insert(ws_key(to_root, to_rel), bytes);
        Ok(scanned.size)
    }

    fn head(&self, store: &std::path::Path) -> Result<Option<u64>, String> {
        Ok(self.read_ref(store, "main")?.and_then(|r| r.target))
    }

    fn set_head(&self, store: &std::path::Path, id: u64) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("set_head")?;
        let mut r = self.read_ref(store, "main")?.unwrap_or(
            crate::capabilities::workspace::domain::workstore::Ref {
                kind: crate::capabilities::workspace::domain::workstore::RefKind::Main,
                target: None,
                heads: BTreeMap::new(),
            },
        );
        r.target = Some(id);
        self.write_ref(store, "main", &r)
    }

    fn clear_head(&self, store: &std::path::Path) -> Result<(), String> {
        self.remove_ref(store, "main")
    }

    fn read_ref(
        &self,
        store: &std::path::Path,
        name: &str,
    ) -> Result<Option<crate::capabilities::workspace::domain::workstore::Ref>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .refs
            .lock()
            .expect("锁")
            .get(&ws_key(store, name))
            .cloned())
    }

    fn write_ref(
        &self,
        store: &std::path::Path,
        name: &str,
        r: &crate::capabilities::workspace::domain::workstore::Ref,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_ref")?;
        self.refs
            .lock()
            .expect("锁")
            .insert(ws_key(store, name), r.clone());
        Ok(())
    }

    fn list_refs(
        &self,
        store: &std::path::Path,
    ) -> Result<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Ref>, String>
    {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", store.to_string_lossy());
        let refs = self.refs.lock().expect("锁");
        let mut out = BTreeMap::new();
        for (k, v) in refs.iter() {
            if let Some(name) = k.strip_prefix(&prefix) {
                out.insert(name.to_string(), v.clone());
            }
        }
        Ok(out)
    }

    fn remove_ref(&self, store: &std::path::Path, name: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.refs.lock().expect("锁").remove(&ws_key(store, name));
        Ok(())
    }

    fn read_commit(
        &self,
        store: &std::path::Path,
        id: u64,
    ) -> Result<Option<crate::capabilities::workspace::domain::workstore::Commit>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .commits
            .lock()
            .expect("锁")
            .get(&ws_key(store, &id.to_string()))
            .cloned())
    }

    fn write_commit(
        &self,
        store: &std::path::Path,
        commit: &crate::capabilities::workspace::domain::workstore::Commit,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_commit")?;
        self.commits
            .lock()
            .expect("锁")
            .insert(ws_key(store, &commit.id.to_string()), commit.clone());
        Ok(())
    }

    fn list_commits(&self, store: &std::path::Path) -> Result<Vec<u64>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", store.to_string_lossy());
        let commits = self.commits.lock().expect("锁");
        let mut ids: Vec<u64> = commits
            .keys()
            .filter_map(|k| k.strip_prefix(&prefix)?.parse::<u64>().ok())
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    fn remove_commit(&self, store: &std::path::Path, id: u64) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("remove_commit")?;
        self.commits
            .lock()
            .expect("锁")
            .remove(&ws_key(store, &id.to_string()));
        Ok(())
    }

    fn read_index(
        &self,
        store: &std::path::Path,
        agent: &str,
    ) -> Result<crate::capabilities::workspace::domain::workstore::Index, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .index
            .lock()
            .expect("锁")
            .get(&ws_key(store, agent))
            .cloned()
            .unwrap_or_default())
    }

    fn write_index(
        &self,
        store: &std::path::Path,
        agent: &str,
        index: &crate::capabilities::workspace::domain::workstore::Index,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_index")?;
        self.index
            .lock()
            .expect("锁")
            .insert(ws_key(store, agent), index.clone());
        Ok(())
    }

    fn write_object(
        &self,
        store: &std::path::Path,
        hash: &str,
        bytes: &[u8],
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_object")?;
        self.objects
            .lock()
            .expect("锁")
            .entry(ws_key(store, hash))
            .or_insert_with(|| bytes.to_vec());
        Ok(())
    }

    fn read_object(&self, store: &std::path::Path, hash: &str) -> Result<Vec<u8>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("read_object")?;
        self.objects
            .lock()
            .expect("锁")
            .get(&ws_key(store, hash))
            .cloned()
            .ok_or_else(|| format!("对象不存在：{}", hash))
    }

    fn lock(&self, _store: &std::path::Path) -> Result<Option<std::fs::File>, String> {
        self.enter("lock")?;
        Ok(None)
    }

    fn read_pending(
        &self,
        store: &std::path::Path,
    ) -> Result<Option<crate::capabilities::workspace::domain::workstore::Pending>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .pending
            .lock()
            .expect("锁")
            .get(&store.to_string_lossy().to_string())
            .cloned())
    }

    fn write_pending(
        &self,
        store: &std::path::Path,
        pending: &crate::capabilities::workspace::domain::workstore::Pending,
    ) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_pending")?;
        self.pending
            .lock()
            .expect("锁")
            .insert(store.to_string_lossy().to_string(), pending.clone());
        Ok(())
    }

    fn clear_pending(&self, store: &std::path::Path) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("clear_pending")?;
        self.pending
            .lock()
            .expect("锁")
            .remove(&store.to_string_lossy().to_string());
        Ok(())
    }

    fn read_next_id(&self, store: &std::path::Path) -> Result<Option<u64>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        Ok(self
            .next_ids
            .lock()
            .expect("锁")
            .get(&store.to_string_lossy().to_string())
            .copied())
    }

    fn write_next_id(&self, store: &std::path::Path, id: u64) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.enter("write_next_id")?;
        self.next_ids
            .lock()
            .expect("锁")
            .insert(store.to_string_lossy().to_string(), id);
        Ok(())
    }

    fn list_indexes(
        &self,
        store: &std::path::Path,
    ) -> Result<BTreeMap<String, crate::capabilities::workspace::domain::workstore::Index>, String>
    {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", store.to_string_lossy());
        let index = self.index.lock().expect("锁");
        let mut out = BTreeMap::new();
        for (k, v) in index.iter() {
            if let Some(agent) = k.strip_prefix(&prefix) {
                out.insert(agent.to_string(), v.clone());
            }
        }
        Ok(out)
    }

    fn list_objects(
        &self,
        store: &std::path::Path,
    ) -> Result<Vec<crate::capabilities::workspace::domain::workstore::ObjectInfo>, String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        let prefix = format!("{}|", store.to_string_lossy());
        let objects = self.objects.lock().expect("锁");
        let mut out = Vec::new();
        for (k, v) in objects.iter() {
            if let Some(hash) = k.strip_prefix(&prefix) {
                out.push(
                    crate::capabilities::workspace::domain::workstore::ObjectInfo {
                        hash: hash.to_string(),
                        size: v.len() as u64,
                    },
                );
            }
        }
        out.sort_by(|a, b| a.hash.cmp(&b.hash));
        Ok(out)
    }

    fn remove_object(&self, store: &std::path::Path, hash: &str) -> Result<(), String> {
        if let Some(m) = &self.fail {
            return Err(m.clone());
        }
        self.objects
            .lock()
            .expect("锁")
            .remove(&ws_key(store, hash));
        Ok(())
    }
}

/// 目的：内存文件系统：内置文件工具的读写落在这里（测试可断言内容与越界拒绝）。
#[derive(Default)]
pub(crate) struct InMemorySysIo {
    pub(crate) files: Mutex<BTreeMap<String, String>>,
    pub(crate) fail: Option<String>,
    // 读取标注：模拟"超过单次上限"与"含非法 UTF-8"的文件（工具必须如实标注，而不是照改）。
    pub(crate) lossy: bool,
    pub(crate) cut: bool,
    // 并发观测：**同时在读**的调用数与其峰值——并发用例据此断言"真的并发"（串行绝不会重叠）。
    pub(crate) active: AtomicUsize,
    pub(crate) peak: AtomicUsize,
    // 每次读取的固定耗时（毫秒）：给并发留出可观测的窗口。
    pub(crate) delay_ms: u64,
    // 指定文件的额外耗时：用来构造"后发的先完成"，验结果仍按原始顺序回填。
    pub(crate) extra: Mutex<BTreeMap<String, u64>>,
}
