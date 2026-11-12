//! 目的：共享区**版本库**的落盘实现（实现 workspace 自己的 `WorkStore` 端口）。
//! 管：资产原语（主副本 `work/` 与各 agent 沙箱的流式扫描 / 写入 / 复制，落点必须真实落在给定根之内）
//!   与仓库原语（内容寻址对象、提交记录 `commits/<id>.json`、head 指针、每 agent 拉取基线 `index/<agent>.json`）。
//! 不管：三方比较、路径校验等纯逻辑（在 `domain::workstore`）；本文件只做机制。
//! 联动：由 workspace 的 `service.rs` 经 `WorkStore` 端口持有（R12）；装配在 `main.rs`。

use crate::capabilities::workspace::domain::asset::{ASSET_CHUNK_BYTES, AssetScanner, FileVersion};
use crate::capabilities::workspace::domain::workstore::{
    Commit, Index, ObjectInfo, Pending, Ref, RefKind, join_rel,
};
use crate::capabilities::workspace::ports::WorkStore;
use serde::de::DeserializeOwned;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

pub struct FsWorkStore;

impl FsWorkStore {
    pub fn new() -> FsWorkStore {
        FsWorkStore
    }
}

/// 把一个干净相对路径安全地落到 root 下：取"目标或它最近的已存在祖先"的真实路径，
/// 必须仍在 root 的真实路径之内——中间或末尾是符号链接就挡在这里。
fn safe_path(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let target = join_rel(root, rel)?;
    let root_real = root
        .canonicalize()
        .map_err(|e| format!("根目录不可达：{}（{}）", root.display(), e))?;
    let mut probe: &Path = &target;
    let real = loop {
        match probe.canonicalize() {
            Ok(r) => break r,
            Err(_) => match probe.parent() {
                Some(parent) if parent != probe => probe = parent,
                _ => return Err(format!("路径不可达：{}", target.display())),
            },
        }
    };
    if !real.starts_with(&root_real) {
        return Err(format!("路径越过允许的根（符号链接指向根外）：{}", rel));
    }
    Ok(target)
}

/// 临时文件名的序号（同一进程内不重名；配合 pid 避免跨进程碰撞）。
static TMP_SEQ: AtomicU64 = AtomicU64::new(0);

/// 目的：同目录下的临时文件名。
fn temp_path(dir: &Path) -> PathBuf {
    dir.join(format!(
        ".tmp-{}-{}",
        std::process::id(),
        TMP_SEQ.fetch_add(1, Ordering::Relaxed)
    ))
}

/// 目的：把同目录临时文件 rename 成目标文件；失败时清掉临时文件。
fn rename_temp(tmp: &Path, path: &Path) -> Result<(), String> {
    if let Err(e) = std::fs::rename(tmp, path) {
        let _ = std::fs::remove_file(tmp);
        return Err(format!("替换文件失败（{}）：{}", path.display(), e));
    }
    // Unix 上 rename 的目录项也要刷一次，才能在掉电后稳定；失败只当尽力而为。
    #[cfg(unix)]
    {
        if let Some(dir) = path.parent() {
            if let Ok(dir_file) = File::open(dir) {
                let _ = dir_file.sync_all();
            }
        }
    }
    Ok(())
}

/// 目的：同目录临时文件 + rename；半个 JSON / 半个对象比没有更糟。
/// 约束：rename 目标与临时文件必须同目录（同一卷），否则 rename 不成立。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path
        .parent()
        .ok_or_else(|| format!("落点没有父目录：{}", path.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{}", e))?;
    let tmp = temp_path(dir);
    let written = (|| -> std::io::Result<()> {
        let mut file = File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("写入临时文件失败（{}）：{}", tmp.display(), e));
    }
    rename_temp(&tmp, path)
}

/// 目的：引用名 → `store/refs/<name>.json`。
/// 约束：只接受由 `/` 分段的普通名字；空段、`.`、`..`、反斜杠、盘符一律拒绝。
fn ref_path(store: &Path, name: &str) -> Result<PathBuf, String> {
    let mut path = store.join("refs");
    let mut segments = 0usize;
    for seg in name.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." || seg.contains('\\') || seg.contains(':') {
            return Err(format!("引用名非法：{}", name));
        }
        path.push(seg);
        segments += 1;
    }
    if segments == 0 {
        return Err("引用名为空".to_string());
    }
    Ok(path.with_extension("json"))
}

/// 目的：递归收集 refs 目录下的引用（名字用 `/` 拼接，不含 `.json`）。
fn walk_refs(dir: &Path, prefix: &str, out: &mut BTreeMap<String, Ref>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("列引用失败：{}", e))?;
    for entry in entries.flatten() {
        let path = entry.path();
        let file_name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            walk_refs(&path, &format!("{}{}/", prefix, file_name), out)?;
            continue;
        }
        let Some(stem) = file_name.strip_suffix(".json") else {
            continue;
        };
        let name = format!("{}{}", prefix, stem);
        let text = std::fs::read_to_string(&path).map_err(|e| format!("读引用失败：{}", e))?;
        let r: Ref =
            serde_json::from_str(&text).map_err(|e| format!("引用 {} 非法：{}", name, e))?;
        out.insert(name, r);
    }
    Ok(())
}

/// 目的：流式扫描一个文件；`preview = None` 只算指纹与大小。
fn read_scan(path: &Path, preview: Option<usize>) -> Result<FileVersion, String> {
    let mut file =
        File::open(path).map_err(|e| format!("读取失败（{}）：{}", path.display(), e))?;
    let mut scanner = AssetScanner::new(preview);
    let mut buf = vec![0u8; ASSET_CHUNK_BYTES];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("读取失败（{}）：{}", path.display(), e))?;
        if n == 0 {
            break;
        }
        scanner.feed(&buf[..n]);
    }
    let scanned = scanner.finish();
    Ok(FileVersion {
        hash: scanned.hash,
        size: scanned.size,
        text: scanned.text,
        truncated: scanned.truncated,
        too_large: false,
    })
}

/// 目的：读一个 JSON 文件；不存在 = None。
fn read_json<T: DeserializeOwned>(path: &Path, what: &str) -> Result<Option<T>, String> {
    match std::fs::read_to_string(path) {
        Ok(text) => serde_json::from_str(&text)
            .map(Some)
            .map_err(|e| format!("{}非法（{}）：{}", what, path.display(), e)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("读{}失败（{}）：{}", what, path.display(), e)),
    }
}

/// 目的：拿住该工作的跨进程文件锁；有界等待，拿不到就如实报错。
fn lock_store(store: &Path) -> Result<File, String> {
    std::fs::create_dir_all(store).map_err(|e| format!("建版本库目录失败：{}", e))?;
    let path = store.join("lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|e| format!("打开版本库锁失败（{}）：{}", path.display(), e))?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match file.try_lock() {
            Ok(()) => return Ok(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                if Instant::now() >= deadline {
                    return Err(
                        "版本库被另一个进程占用，等待超时（稍后重试或关闭另一个核心）".to_string(),
                    );
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(std::fs::TryLockError::Error(e)) => return Err(format!("版本库加锁失败：{}", e)),
        }
    }
}

/// 递归列文件（只列文件），相对路径用 / 分隔，稳定排序。
fn collect(root: &Path, out: &mut Vec<String>, prefix: &str) {
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
        let rel = if prefix.is_empty() {
            name
        } else {
            format!("{}/{}", prefix, name)
        };
        if is_dir {
            collect(&path, out, &rel);
        } else {
            out.push(rel);
        }
    }
}

impl WorkStore for FsWorkStore {
    fn remove_under(&self, root: &Path, rel: &str) -> Result<(), String> {
        let path = safe_path(root, rel)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删除失败：{}", e)),
        }
    }

    fn list(&self, root: &Path) -> Result<Vec<String>, String> {
        let mut out = Vec::new();
        collect(root, &mut out, "");
        Ok(out)
    }

    fn stat_under(&self, root: &Path, rel: &str) -> Result<Option<u64>, String> {
        let path = safe_path(root, rel)?;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => Ok(Some(meta.len())),
            Ok(_) => Err(format!("不是文件：{}", rel)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取文件大小失败（{}）：{}", path.display(), e)),
        }
    }

    fn scan_under(
        &self,
        root: &Path,
        rel: &str,
        preview: Option<usize>,
    ) -> Result<Option<FileVersion>, String> {
        let path = safe_path(root, rel)?;
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => read_scan(&path, preview).map(Some),
            Ok(_) => Err(format!("不是文件：{}", rel)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取失败（{}）：{}", path.display(), e)),
        }
    }

    fn stat_object(&self, store: &Path, hash: &str) -> Result<u64, String> {
        let path = store.join("objects").join(hash);
        match std::fs::metadata(&path) {
            Ok(meta) if meta.is_file() => Ok(meta.len()),
            Ok(_) => Err(format!("对象不是文件：{}", hash)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Err(format!("对象不存在：{}", hash))
            }
            Err(e) => Err(format!("读对象大小失败（{}）：{}", hash, e)),
        }
    }

    fn scan_object(
        &self,
        store: &Path,
        hash: &str,
        preview: Option<usize>,
    ) -> Result<FileVersion, String> {
        read_scan(&store.join("objects").join(hash), preview)
            .map_err(|e| format!("读对象 {} 失败：{}", hash, e))
    }

    fn ingest_under(
        &self,
        store: &Path,
        root: &Path,
        rel: &str,
        expected_hash: &str,
        max_bytes: u64,
    ) -> Result<u64, String> {
        let object = store.join("objects").join(expected_hash);
        if let Ok(meta) = std::fs::metadata(&object)
            && meta.is_file()
        {
            return Ok(meta.len());
        }
        let source = safe_path(root, rel)?;
        let mut src =
            File::open(&source).map_err(|e| format!("读取失败（{}）：{}", source.display(), e))?;
        let dir = store.join("objects");
        std::fs::create_dir_all(&dir).map_err(|e| format!("建对象目录失败：{}", e))?;
        let tmp = temp_path(&dir);
        let result = (|| -> Result<u64, String> {
            let mut out = File::create(&tmp)
                .map_err(|e| format!("建临时文件失败（{}）：{}", tmp.display(), e))?;
            let mut scanner = AssetScanner::new(None);
            let mut buf = vec![0u8; ASSET_CHUNK_BYTES];
            let mut size: u64 = 0;
            loop {
                let n = src
                    .read(&mut buf)
                    .map_err(|e| format!("读取失败（{}）：{}", source.display(), e))?;
                if n == 0 {
                    break;
                }
                size = size.saturating_add(n as u64);
                if size > max_bytes {
                    return Err(format!(
                        "文件超过单文件上限（{} 字节 > {} 字节）：{}",
                        size, max_bytes, rel
                    ));
                }
                scanner.feed(&buf[..n]);
                out.write_all(&buf[..n])
                    .map_err(|e| format!("写临时文件失败（{}）：{}", tmp.display(), e))?;
            }
            let scanned = scanner.finish();
            if scanned.hash != expected_hash {
                return Err(format!("文件在提交过程中被改动，请重试：{}", rel));
            }
            out.sync_all()
                .map_err(|e| format!("刷临时文件失败（{}）：{}", tmp.display(), e))?;
            drop(out);
            rename_temp(&tmp, &object)?;
            Ok(scanned.size)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    fn copy_object_to_under(
        &self,
        store: &Path,
        hash: &str,
        root: &Path,
        rel: &str,
    ) -> Result<(), String> {
        let source = store.join("objects").join(hash);
        let target = safe_path(root, rel)?;
        let dir = target
            .parent()
            .ok_or_else(|| format!("落点没有父目录：{}", target.display()))?;
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{}", e))?;
        let tmp = temp_path(dir);
        let result = (|| -> Result<(), String> {
            let mut src =
                File::open(&source).map_err(|e| format!("读对象 {} 失败：{}", hash, e))?;
            let mut out = File::create(&tmp)
                .map_err(|e| format!("建临时文件失败（{}）：{}", tmp.display(), e))?;
            std::io::copy(&mut src, &mut out)
                .map_err(|e| format!("写沙箱文件失败（{}）：{}", target.display(), e))?;
            out.sync_all()
                .map_err(|e| format!("刷临时文件失败（{}）：{}", tmp.display(), e))?;
            drop(out);
            rename_temp(&tmp, &target)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    fn copy_under_to_under(
        &self,
        from_root: &Path,
        from_rel: &str,
        to_root: &Path,
        to_rel: &str,
        expected_hash: &str,
    ) -> Result<u64, String> {
        let source = safe_path(from_root, from_rel)?;
        std::fs::create_dir_all(to_root)
            .map_err(|e| format!("建目标根失败（{}）：{}", to_root.display(), e))?;
        let target = safe_path(to_root, to_rel)?;
        let dir = target
            .parent()
            .ok_or_else(|| format!("落点没有父目录：{}", target.display()))?;
        std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{}", e))?;
        let tmp = temp_path(dir);
        let result = (|| -> Result<u64, String> {
            let mut src = File::open(&source)
                .map_err(|e| format!("读取失败（{}）：{}", source.display(), e))?;
            let mut out = File::create(&tmp)
                .map_err(|e| format!("建临时文件失败（{}）：{}", tmp.display(), e))?;
            let mut scanner = AssetScanner::new(None);
            let mut buf = vec![0u8; ASSET_CHUNK_BYTES];
            loop {
                let n = src
                    .read(&mut buf)
                    .map_err(|e| format!("读取失败（{}）：{}", source.display(), e))?;
                if n == 0 {
                    break;
                }
                scanner.feed(&buf[..n]);
                out.write_all(&buf[..n])
                    .map_err(|e| format!("写临时文件失败（{}）：{}", tmp.display(), e))?;
            }
            let scanned = scanner.finish();
            if scanned.hash != expected_hash {
                return Err(format!("源文件在复制过程中被改动，请重试：{}", from_rel));
            }
            out.sync_all()
                .map_err(|e| format!("刷临时文件失败（{}）：{}", tmp.display(), e))?;
            drop(out);
            rename_temp(&tmp, &target)?;
            Ok(scanned.size)
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        result
    }

    /// 主线指针就是 `refs/main`；旧布局的 `head` 文件读到一次就升级成引用再删掉。
    fn head(&self, store: &Path) -> Result<Option<u64>, String> {
        if let Some(r) = self.read_ref(store, "main")? {
            return Ok(r.target);
        }
        let legacy = store.join("head");
        match std::fs::read_to_string(&legacy) {
            Ok(t) => {
                let id = t
                    .trim()
                    .parse::<u64>()
                    .map_err(|e| format!("head 记录非法：{}", e))?;
                self.write_ref(
                    store,
                    "main",
                    &Ref {
                        kind: RefKind::Main,
                        target: Some(id),
                        heads: BTreeMap::new(),
                    },
                )?;
                let _ = std::fs::remove_file(&legacy);
                Ok(Some(id))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读 head 失败：{}", e)),
        }
    }

    fn set_head(&self, store: &Path, id: u64) -> Result<(), String> {
        let mut r = self.read_ref(store, "main")?.unwrap_or(Ref {
            kind: RefKind::Main,
            target: None,
            heads: BTreeMap::new(),
        });
        r.target = Some(id);
        self.write_ref(store, "main", &r)
    }

    fn clear_head(&self, store: &Path) -> Result<(), String> {
        self.remove_ref(store, "main")?;
        match std::fs::remove_file(store.join("head")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("清 head 失败：{}", e)),
        }
    }

    fn read_ref(&self, store: &Path, name: &str) -> Result<Option<Ref>, String> {
        let path = ref_path(store, name)?;
        match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t)
                .map(Some)
                .map_err(|e| format!("引用 {} 非法：{}", name, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读引用失败：{}", e)),
        }
    }

    fn write_ref(&self, store: &Path, name: &str, r: &Ref) -> Result<(), String> {
        let text = serde_json::to_string_pretty(r).map_err(|e| format!("引用序列化失败：{}", e))?;
        write_atomic(&ref_path(store, name)?, text.as_bytes())
    }

    fn list_refs(&self, store: &Path) -> Result<BTreeMap<String, Ref>, String> {
        let mut out = BTreeMap::new();
        let root = store.join("refs");
        if !root.exists() {
            return Ok(out);
        }
        walk_refs(&root, "", &mut out)?;
        Ok(out)
    }

    fn remove_ref(&self, store: &Path, name: &str) -> Result<(), String> {
        let path = ref_path(store, name)?;
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删引用失败：{}", e)),
        }
    }

    fn read_commit(&self, store: &Path, id: u64) -> Result<Option<Commit>, String> {
        let path = store.join("commits").join(format!("{}.json", id));
        match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t)
                .map(Some)
                .map_err(|e| format!("提交记录 {} 非法：{}", id, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读提交记录失败：{}", e)),
        }
    }

    fn write_commit(&self, store: &Path, commit: &Commit) -> Result<(), String> {
        let text = serde_json::to_string_pretty(commit)
            .map_err(|e| format!("提交记录序列化失败：{}", e))?;
        let path = store.join("commits").join(format!("{}.json", commit.id));
        write_atomic(&path, text.as_bytes())
    }

    fn list_commits(&self, store: &Path) -> Result<Vec<u64>, String> {
        let dir = store.join("commits");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut ids: Vec<u64> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".json")?.parse::<u64>().ok()
            })
            .collect();
        ids.sort_unstable();
        Ok(ids)
    }

    fn remove_commit(&self, store: &Path, id: u64) -> Result<(), String> {
        let path = store.join("commits").join(format!("{}.json", id));
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删提交记录失败：{}", e)),
        }
    }

    fn read_index(&self, store: &Path, agent: &str) -> Result<Index, String> {
        let path = store.join("index").join(format!("{}.json", agent));
        match std::fs::read_to_string(&path) {
            Ok(t) => serde_json::from_str(&t).map_err(|e| format!("拉取基线非法：{}", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Index::new()),
            Err(e) => Err(format!("读拉取基线失败：{}", e)),
        }
    }

    fn write_index(&self, store: &Path, agent: &str, index: &Index) -> Result<(), String> {
        let text = serde_json::to_string_pretty(index)
            .map_err(|e| format!("拉取基线序列化失败：{}", e))?;
        let path = store.join("index").join(format!("{}.json", agent));
        write_atomic(&path, text.as_bytes())
    }

    fn write_object(&self, store: &Path, hash: &str, bytes: &[u8]) -> Result<(), String> {
        let path = store.join("objects").join(hash);
        if let Ok(meta) = std::fs::metadata(&path)
            && meta.is_file()
            && meta.len() == bytes.len() as u64
        {
            return Ok(());
        }
        write_atomic(&path, bytes)
    }

    fn read_object(&self, store: &Path, hash: &str) -> Result<Vec<u8>, String> {
        std::fs::read(store.join("objects").join(hash))
            .map_err(|e| format!("读对象 {} 失败：{}", hash, e))
    }

    fn lock(&self, store: &Path) -> Result<Option<File>, String> {
        lock_store(store).map(Some)
    }

    fn read_pending(&self, store: &Path) -> Result<Option<Pending>, String> {
        read_json(&store.join("pending.json"), "待恢复事务")
    }

    fn write_pending(&self, store: &Path, pending: &Pending) -> Result<(), String> {
        let text = serde_json::to_string_pretty(pending)
            .map_err(|e| format!("待恢复事务序列化失败：{}", e))?;
        write_atomic(&store.join("pending.json"), text.as_bytes())
    }

    fn clear_pending(&self, store: &Path) -> Result<(), String> {
        match std::fs::remove_file(store.join("pending.json")) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删待恢复事务失败：{}", e)),
        }
    }

    fn read_next_id(&self, store: &Path) -> Result<Option<u64>, String> {
        match std::fs::read_to_string(store.join("next-id")) {
            Ok(text) => text
                .trim()
                .parse::<u64>()
                .map(Some)
                .map_err(|e| format!("提交号计数器非法：{}", e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读提交号计数器失败：{}", e)),
        }
    }

    fn write_next_id(&self, store: &Path, id: u64) -> Result<(), String> {
        write_atomic(&store.join("next-id"), id.to_string().as_bytes())
    }

    fn list_indexes(&self, store: &Path) -> Result<BTreeMap<String, Index>, String> {
        let dir = store.join("index");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(BTreeMap::new());
        };
        let mut out = BTreeMap::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Some(agent) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if let Some(index) = read_json::<Index>(&path, "拉取基线")? {
                out.insert(agent.to_string(), index);
            }
        }
        Ok(out)
    }

    fn list_objects(&self, store: &Path) -> Result<Vec<ObjectInfo>, String> {
        let dir = store.join("objects");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return Ok(Vec::new());
        };
        let mut out = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(hash) = path.file_name().and_then(|s| s.to_str()) else {
                continue;
            };
            let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            out.push(ObjectInfo {
                hash: hash.to_string(),
                size,
            });
        }
        out.sort_by(|a, b| a.hash.cmp(&b.hash));
        Ok(out)
    }

    fn remove_object(&self, store: &Path, hash: &str) -> Result<(), String> {
        let path = store.join("objects").join(hash);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("删对象 {} 失败：{}", hash, e)),
        }
    }
}
