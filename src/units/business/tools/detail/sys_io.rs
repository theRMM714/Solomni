//! 目的：内置文件工具的读写机制（实现 tools 自己的 `SysIo` 端口）——纯 Rust 直接读写，不经任何外部进程。
//! 管：读（按 UTF-8 解码，含非法字节时按替换字符呈现并置 `lossy`）、写（一律 UTF-8，需要时建父目录，覆盖同名文件）与真实落点越界校验。
//! 不管：编码猜测（不是本程序的事）；参数校验、回执文案与放行（在 `domain`）。
//! 联动：实现 `ports::SysIo`，由 `main.rs` 构造注入 `service/`。

use crate::kernel::api::lock::lock_within;
use crate::units::business::tools::api::LineRange;
use crate::units::business::tools::ports::{
    DirEntry, Expect, FilePage, FileRead, MAX_SCAN_LINE_BYTES, Mutation, ScanSummary, SysIo,
    WalkEntry, WalkOpts, WriteFault, WriteOp, WriteReceipt,
};
use crate::units::business::versions::api::{Sha256, content_hash};
use crate::units::business::workspace::api::Rules;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock,
    atomic::{AtomicU64, Ordering},
};
use std::time::Duration;

/// 目的：流式算整份原始字节的 SHA-256；内存只保留一个块。
fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = std::fs::File::open(path).map_err(|e| format!("读取失败：{}", e))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file
            .read(&mut buf)
            .map_err(|e| format!("读取失败：{}", e))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finish_hex())
}

/// 删除链接本身：Windows 的目录链接 / junction 可能只认 `remove_dir`，先试 `remove_file` 再退。
fn remove_link(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path)
        .or_else(|_| std::fs::remove_dir(path))
        .map_err(|e| format!("删除链接 {} 失败：{}", path.display(), e))
}

/// 递归删除目录：逐项看 `symlink_metadata`，链接只删链接本身，绝不顺着链接进目标。
fn remove_tree(dir: &Path) -> Result<(), String> {
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("列目录 {} 失败：{}", dir.display(), e))?;
    for ent in entries {
        let ent = ent.map_err(|e| format!("读目录项失败：{}", e))?;
        let p = ent.path();
        let meta =
            std::fs::symlink_metadata(&p).map_err(|e| format!("看 {} 失败：{}", p.display(), e))?;
        if meta.file_type().is_symlink() {
            remove_link(&p)?;
        } else if meta.is_dir() {
            remove_tree(&p)?;
        } else {
            std::fs::remove_file(&p).map_err(|e| format!("删除 {} 失败：{}", p.display(), e))?;
        }
    }
    std::fs::remove_dir(dir).map_err(|e| format!("删除目录 {} 失败：{}", dir.display(), e))
}

/// 真实落点是否仍在 `root` 内：目标存在就 `canonicalize`；目标不存在就退到最近的已存在祖先；
/// 叶子是悬空符号链接时直接拒绝（`canonicalize` 会失败，不能靠“看父目录”绕过）。
fn real_within(root: &Path, path: &Path) -> Result<(), String> {
    let root_real = root
        .canonicalize()
        .map_err(|e| format!("根目录不可达：{}（{}）", root.display(), e))?;
    let contained = |real: &Path| {
        if real.starts_with(&root_real) {
            Ok(())
        } else {
            Err(format!(
                "路径越过允许的根（符号链接 / junction 指向根外）：{}",
                path.display()
            ))
        }
    };
    if let Ok(real) = path.canonicalize() {
        return contained(&real);
    }
    if std::fs::symlink_metadata(path)
        .map(|m| m.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(format!(
            "路径越过允许的根（符号链接 / junction 指向根外或悬空）：{}",
            path.display()
        ));
    }
    let mut probe = path;
    loop {
        match probe.parent() {
            Some(parent) if parent != probe => {
                probe = parent;
                if let Ok(real) = probe.canonicalize() {
                    return contained(&real);
                }
            }
            _ => return Err(format!("路径不可达：{}", path.display())),
        }
    }
}

/// 目的：按路径取一把进程内锁：读版本、校验与发布在这把锁里串行。
/// 约束：锁表是进程级全局表，不随 `FsSysIo` 实例走（适配层可能被多次构造）。
fn path_lock(path: &Path) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<BTreeMap<PathBuf, Arc<Mutex<()>>>>> = OnceLock::new();
    let table = LOCKS.get_or_init(|| Mutex::new(BTreeMap::new()));
    let mut table = table.lock().unwrap_or_else(|e| e.into_inner());
    table
        .entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// 目的：读整份原始字节；不存在 = Ok(None)，其它错误如实报。
fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>, WriteFault> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(WriteFault::io(path, e)),
    }
}

/// 目的：写盘前的 CAS 校验：当前版本必须仍是调用方以为的那一份。
fn check_expect(
    path: &Path,
    expect: &Expect,
    current: Option<&[u8]>,
    version: Option<&str>,
) -> Result<(), WriteFault> {
    match expect {
        Expect::Absent => {
            if current.is_some() {
                return Err(WriteFault::already_exists(path));
            }
        }
        Expect::Version(want) => match version {
            None => return Err(WriteFault::missing(path)),
            Some(now) if now != want => return Err(WriteFault::stale(path, want, now)),
            Some(_) => {}
        },
    }
    Ok(())
}

/// 目的：在当前内容上执行变换；字面替换在这里重新算一遍，不信任调用方算好的结果。
fn apply_mutation(
    path: &Path,
    current: Option<&[u8]>,
    mutation: Mutation,
) -> Result<(String, Vec<LineRange>, usize), WriteFault> {
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
            let Some(bytes) = current else {
                return Err(WriteFault::missing(path));
            };
            let hay =
                String::from_utf8(bytes.to_vec()).map_err(|_| WriteFault::invalid_utf8(path))?;
            match crate::units::business::tools::domain::systool::apply_literal_edit(
                &hay, &old, &new, all,
            ) {
                Ok((out, changed, n)) => Ok((
                    out,
                    changed
                        .into_iter()
                        .map(|(from, to)| LineRange::new(from, to))
                        .collect(),
                    n,
                )),
                Err(crate::units::business::tools::domain::systool::LiteralFault::NotFound) => {
                    Err(WriteFault::anchor_not_found(path))
                }
                Err(crate::units::business::tools::domain::systool::LiteralFault::Ambiguous {
                    lines,
                }) => Err(WriteFault::anchor_ambiguous(path, lines.len())),
            }
        }
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
) -> Result<PathBuf, WriteFault> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| WriteFault::io(path, e))?;
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
            Err(e) => return Err(WriteFault::io(path, e)),
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
            Ok(())
        });
    if let Err(e) = staged {
        let _ = std::fs::remove_file(&tmp);
        return Err(WriteFault::io(path, e));
    }
    Ok(tmp)
}

/// 目的：发布临时文件：`no_replace` 时用硬链接做 no-clobber 发布，否则原子替换。
fn publish_temp(tmp: &Path, path: &Path, no_replace: bool) -> Result<(), WriteFault> {
    if !no_replace {
        return std::fs::rename(tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(tmp);
            WriteFault::io(path, e)
        });
    }
    match std::fs::hard_link(tmp, path) {
        Ok(()) => {
            let _ = std::fs::remove_file(tmp);
            Ok(())
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(tmp);
            Err(WriteFault::already_exists(path))
        }
        Err(_) => {
            // 文件系统不支持硬链接时的兜底：create_new 保证不覆盖，再复制内容。
            let mut src = std::fs::File::open(tmp).map_err(|e| WriteFault::io(path, e))?;
            let mut dst = match std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(f) => f,
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                    let _ = std::fs::remove_file(tmp);
                    return Err(WriteFault::already_exists(path));
                }
                Err(e) => {
                    let _ = std::fs::remove_file(tmp);
                    return Err(WriteFault::io(path, e));
                }
            };
            let copied = std::io::copy(&mut src, &mut dst).and_then(|_| dst.sync_all());
            let _ = std::fs::remove_file(tmp);
            if let Err(e) = copied {
                let _ = std::fs::remove_file(path);
                return Err(WriteFault::io(path, e));
            }
            Ok(())
        }
    }
}

/// 目的：把旧内容原子写回（事务回滚用）。
pub(crate) fn rollback(path: &Path, previous: Option<&[u8]>) -> Result<(), WriteFault> {
    match previous {
        Some(bytes) => {
            let tmp = stage_temp(path, bytes, Some(path))?;
            publish_temp(&tmp, path, false)
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(WriteFault::io(path, e)),
        },
    }
}

/// 目的：按行窗口扫描整份文件时的选区状态：只留请求区间的行，同时数完整份行数。
struct PageScan {
    offset: u32,
    end: u32,
    max_bytes: usize,
    text: String,
    started: bool,
    used: usize,
    lossy: bool,
    window_cut: bool,
    line_cut: bool,
    collecting: bool,
}

impl PageScan {
    fn new(offset: u32, limit: u32, max_bytes: usize) -> PageScan {
        let offset = offset.max(1);
        PageScan {
            offset,
            end: offset.saturating_add(limit.max(1)),
            max_bytes: max_bytes.max(1),
            text: String::new(),
            started: false,
            used: 0,
            lossy: false,
            window_cut: false,
            line_cut: false,
            collecting: true,
        }
    }

    fn want(&self, line: u32) -> bool {
        self.collecting && line >= self.offset && line < self.end
    }

    /// 目的：收下一整行（`truncated` 表示这一行只拿到了字节上限内的前缀）。
    fn push(&mut self, line: u32, raw: &[u8], truncated: bool) {
        if !self.want(line) {
            return;
        }
        let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
        let (text, bad) = if truncated {
            (String::from_utf8_lossy(raw).into_owned(), false)
        } else {
            match std::str::from_utf8(raw) {
                Ok(t) => (t.to_string(), false),
                Err(_) => (String::from_utf8_lossy(raw).into_owned(), true),
            }
        };
        self.lossy |= bad;
        self.line_cut |= truncated;
        // 用 started 而不是 text.is_empty()：开头的空行也是选中行，不能被当成"还没开始"。
        if self.started {
            self.text.push('\n');
        }
        self.started = true;
        self.used = self.used.saturating_add(text.len() + 1);
        self.text.push_str(&text);
        if self.used >= self.max_bytes && line + 1 < self.end {
            self.window_cut = true;
            self.collecting = false;
        }
    }
}

pub struct FsSysIo {
    /// 目的：单页收集的字节缓冲上限（只影响一次读最多收集多少内容，不是模型可见额度）。
    pub max_read_bytes: usize,
    /// 目的：跨进程文件锁的落点；None 只用进程内锁（测试与无产品根的场景）。
    pub lock_dir: Option<PathBuf>,
    /// 目的：拿不到跨进程锁时的有界等待。
    pub lock_wait: Duration,
}

impl Default for FsSysIo {
    fn default() -> FsSysIo {
        FsSysIo {
            max_read_bytes: 1_000_000,
            lock_dir: None,
            lock_wait: Duration::from_secs(10),
        }
    }
}

impl FsSysIo {
    /// 目的：带产品锁目录构造，写路径按文件路径跨进程互斥。
    pub fn with_lock_dir(lock_dir: PathBuf) -> FsSysIo {
        FsSysIo {
            lock_dir: Some(lock_dir),
            ..FsSysIo::default()
        }
    }

    /// 目的：拿到保护 path 的跨进程锁；未配置锁目录 = Ok(None)。
    fn lock_path(&self, path: &Path) -> Result<Option<std::fs::File>, WriteFault> {
        let Some(dir) = &self.lock_dir else {
            return Ok(None);
        };
        std::fs::create_dir_all(dir).map_err(|e| {
            WriteFault::new(
                "FS_LOCKED",
                format!("创建锁目录失败（{}）：{}", dir.display(), e),
            )
        })?;
        let mut key = lock_key(path).to_string_lossy().into_owned();
        if crate::kernel::api::platform::HOST_PATH_CASE_INSENSITIVE {
            key = key.to_lowercase();
        }
        let name = format!("file-{}.lock", content_hash(key.as_bytes()));
        let lock = dir.join(name);
        lock_within(&lock, self.lock_wait, "文件写入")
            .map(Some)
            .map_err(|e| WriteFault::new("FS_LOCKED", format!("{}：{}", path.display(), e)))
    }

    /// 目的：测试用——直接持有某路径的跨进程锁，验证第二个实例会被挡下。
    #[cfg(test)]
    pub(crate) fn hold_lock_for_test(
        &self,
        path: &Path,
    ) -> Result<Option<std::fs::File>, WriteFault> {
        self.lock_path(path)
    }
}

/// 目的：前 8 KiB 里出现 NUL 就当二进制（与 rg / git 的判据同源）。
fn probe_binary(head: &[u8]) -> bool {
    head.contains(&0)
}

/// 目的：从流里读一行，最多 `cap` 字节；超出的部分读到换行前丢掉。
/// 返回（是否读到内容, 是否被截断）。行尾的 \n 会留在 buffer 里，由调用方裁掉。
fn read_capped_line<R: std::io::BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    cap: usize,
) -> Result<(bool, bool), String> {
    let mut got = false;
    let mut cut = false;
    loop {
        let available = reader.fill_buf().map_err(|e| format!("读取失败：{}", e))?;
        if available.is_empty() {
            return Ok((got, cut));
        }
        let (take, done) = match available.iter().position(|b| *b == b'\n') {
            Some(i) => (i + 1, true),
            None => (available.len(), false),
        };
        let room = cap.saturating_sub(buf.len());
        if take <= room {
            buf.extend_from_slice(&available[..take]);
        } else {
            buf.extend_from_slice(&available[..room]);
            cut = true;
        }
        reader.consume(take);
        got = true;
        if done {
            return Ok((got, cut));
        }
        if cut {
            // 已经到上限：把这一行剩下的字节读到换行为止，不进 buffer。
            loop {
                let rest = reader.fill_buf().map_err(|e| format!("读取失败：{}", e))?;
                if rest.is_empty() {
                    return Ok((got, cut));
                }
                let (n, end) = match rest.iter().position(|b| *b == b'\n') {
                    Some(i) => (i + 1, true),
                    None => (rest.len(), false),
                };
                reader.consume(n);
                if end {
                    return Ok((got, cut));
                }
            }
        }
    }
}

/// 目的：把一行字节裁成模型看的字符串：去掉行尾换行与回车，非法 UTF-8 按替换字符呈现。
fn line_text(raw: &[u8], lossy: &mut bool) -> String {
    let body = raw.strip_suffix(b"\n").unwrap_or(raw);
    let body = body.strip_suffix(b"\r").unwrap_or(body);
    match std::str::from_utf8(body) {
        Ok(s) => s.to_string(),
        Err(_) => {
            *lossy = true;
            String::from_utf8_lossy(body).into_owned()
        }
    }
}

/// 目的：把路径归一成锁键——存在取真实路径，不存在取真实父目录加文件名，再退回原路径。
/// 约束：同一文件经不同符号链接路径访问时应落到同一把锁。
fn lock_key(path: &Path) -> PathBuf {
    path.canonicalize()
        .ok()
        .or_else(|| {
            let parent = path.parent()?.canonicalize().ok()?;
            Some(parent.join(path.file_name()?))
        })
        .unwrap_or_else(|| path.to_path_buf())
}

impl SysIo for FsSysIo {
    fn read(&self, path: &Path) -> Result<FileRead, String> {
        // 先看大小、只读开头：大文件不因为一次 read 就整份进内存。
        let total = std::fs::metadata(path)
            .map(|m| m.len() as usize)
            .map_err(|e| format!("读取失败：{}", e))?;
        let n = self.max_read_bytes.min(total);
        let mut bytes = Vec::with_capacity(n);
        std::fs::File::open(path)
            .map_err(|e| format!("读取失败：{}", e))?
            .take(n as u64)
            .read_to_end(&mut bytes)
            .map_err(|e| format!("读取失败：{}", e))?;
        let (text, lossy) = match std::str::from_utf8(&bytes[..]) {
            Ok(t) => (t.to_string(), false),
            Err(e) => {
                if e.error_len().is_none() && n < total {
                    // 只是切片落在多字节字符中间：退回完整字符边界，不算编码错误。
                    (
                        String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(),
                        false,
                    )
                } else {
                    (String::from_utf8_lossy(&bytes[..]).into_owned(), true)
                }
            }
        };
        let hash = hash_file(path)?;
        Ok(FileRead {
            text,
            bytes: total,
            lossy,
            cut: n < total,
            hash,
        })
    }

    fn read_page(&self, path: &Path, offset: u32, limit: u32) -> Result<FilePage, String> {
        let meta = std::fs::metadata(path).map_err(|e| format!("读取失败：{}", e))?;
        if !meta.is_file() {
            return Err(format!("不是一个普通文件：{}", path.display()));
        }
        let bytes = meta.len() as usize;
        let mut file = std::fs::File::open(path).map_err(|e| format!("读取失败：{}", e))?;
        let mut scan = PageScan::new(offset, limit, self.max_read_bytes);
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 64 * 1024];
        let mut line: Vec<u8> = Vec::new();
        let mut line_truncated = false;
        let mut line_no: u32 = 0;
        let mut read_any = false;
        let mut last_byte = 0u8;
        loop {
            let n = file
                .read(&mut buf)
                .map_err(|e| format!("读取失败：{}", e))?;
            if n == 0 {
                break;
            }
            read_any = true;
            last_byte = buf[n - 1];
            hasher.update(&buf[..n]);
            for &b in &buf[..n] {
                if b == b'\n' {
                    line_no = line_no.saturating_add(1);
                    scan.push(line_no, &line, line_truncated);
                    line.clear();
                    line_truncated = false;
                } else if scan.want(line_no + 1) {
                    if line.len() < scan.max_bytes {
                        line.push(b);
                    } else {
                        line_truncated = true;
                    }
                }
            }
        }
        // 最后一段没有换行符时也是一行；文件以换行结尾时不再补一行空行。
        if !line.is_empty() || (read_any && last_byte != b'\n') {
            line_no = line_no.saturating_add(1);
            scan.push(line_no, &line, line_truncated);
        }
        Ok(FilePage {
            text: scan.text,
            bytes,
            lossy: scan.lossy,
            window_cut: scan.window_cut,
            line_cut: scan.line_cut,
            hash: hasher.finish_hex(),
            total_lines: line_no,
        })
    }

    fn fingerprint(&self, path: &Path) -> Result<Option<String>, String> {
        match std::fs::metadata(path) {
            Ok(meta) if meta.is_file() => hash_file(path).map(Some),
            Ok(_) => Err(format!("不是一个普通文件：{}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取失败：{}", e)),
        }
    }

    fn mutate(
        &self,
        path: &Path,
        expect: &Expect,
        mutation: Mutation,
    ) -> Result<WriteReceipt, WriteFault> {
        // 跨进程锁优先；未配置锁目录时退回进程内锁，两者不同时持有，避免同进程二次加锁。
        let lock = self.lock_dir.is_none().then(|| path_lock(path));
        let _guard = lock
            .as_ref()
            .map(|lock| lock.lock().unwrap_or_else(|e| e.into_inner()));
        let _file = self.lock_path(path)?;
        let current = read_bytes(path)?;
        let version = current.as_deref().map(content_hash);
        check_expect(path, expect, current.as_deref(), version.as_deref())?;
        let (text, changed, replaced) = apply_mutation(path, current.as_deref(), mutation)?;
        let new_version = content_hash(text.as_bytes());
        let total_lines = text.lines().count() as u32;
        let no_replace = matches!(expect, Expect::Absent);
        let tmp = stage_temp(path, text.as_bytes(), current.is_some().then_some(path))?;
        publish_temp(&tmp, path, no_replace)?;
        Ok(WriteReceipt {
            version: new_version,
            total_lines,
            changed,
            replaced,
        })
    }

    fn transact(&self, ops: Vec<WriteOp>) -> Result<Vec<WriteReceipt>, WriteFault> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen: BTreeSet<&Path> = BTreeSet::new();
        for op in &ops {
            if !seen.insert(op.path.as_path()) {
                return Err(WriteFault::new(
                    "FS_DUPLICATE_PATH",
                    format!(
                        "同一个路径在一次事务里出现了两次：{}；没有写盘。",
                        op.path.display()
                    ),
                ));
            }
        }
        // 按路径排序取锁，避免两个事务交叉加锁死锁；执行顺序仍按 ops 原顺序。
        let mut order: Vec<usize> = (0..ops.len()).collect();
        order.sort_by(|&a, &b| ops[a].path.cmp(&ops[b].path));
        let locks: Vec<Arc<Mutex<()>>> = if self.lock_dir.is_some() {
            Vec::new()
        } else {
            order.iter().map(|&i| path_lock(&ops[i].path)).collect()
        };
        let _guards: Vec<MutexGuard<'_, ()>> = locks
            .iter()
            .map(|lock| lock.lock().unwrap_or_else(|e| e.into_inner()))
            .collect();
        // 同一顺序拿跨进程锁，两个事务不会交叉等待。
        let mut _files: Vec<std::fs::File> = Vec::new();
        if self.lock_dir.is_some() {
            for &i in &order {
                match self.lock_path(&ops[i].path) {
                    Ok(Some(file)) => _files.push(file),
                    Ok(None) => {}
                    Err(fault) => return Err(fault),
                }
            }
        }

        struct Staged {
            path: PathBuf,
            tmp: PathBuf,
            previous: Option<Vec<u8>>,
            absent: bool,
            receipt: WriteReceipt,
        }
        let cleanup = |staged: &[Staged]| {
            for item in staged {
                let _ = std::fs::remove_file(&item.tmp);
            }
        };
        let mut staged: Vec<Staged> = Vec::with_capacity(ops.len());
        for op in ops {
            let current = match read_bytes(&op.path) {
                Ok(c) => c,
                Err(fault) => {
                    cleanup(&staged);
                    return Err(fault);
                }
            };
            let version = current.as_deref().map(content_hash);
            if let Err(fault) =
                check_expect(&op.path, &op.expect, current.as_deref(), version.as_deref())
            {
                cleanup(&staged);
                return Err(fault);
            }
            let (text, changed, replaced) =
                match apply_mutation(&op.path, current.as_deref(), op.mutation) {
                    Ok(x) => x,
                    Err(fault) => {
                        cleanup(&staged);
                        return Err(fault);
                    }
                };
            let new_version = content_hash(text.as_bytes());
            let total_lines = text.lines().count() as u32;
            let absent = matches!(op.expect, Expect::Absent);
            let tmp = match stage_temp(
                &op.path,
                text.as_bytes(),
                current.is_some().then_some(op.path.as_path()),
            ) {
                Ok(tmp) => tmp,
                Err(fault) => {
                    cleanup(&staged);
                    return Err(fault);
                }
            };
            staged.push(Staged {
                path: op.path,
                tmp,
                previous: current,
                absent,
                receipt: WriteReceipt {
                    version: new_version,
                    total_lines,
                    changed,
                    replaced,
                },
            });
        }

        for i in 0..staged.len() {
            if let Err(mut fault) = publish_temp(&staged[i].tmp, &staged[i].path, staged[i].absent)
            {
                for j in (0..i).rev() {
                    if let Err(e) = rollback(&staged[j].path, staged[j].previous.as_deref()) {
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
        Ok(staged.into_iter().map(|item| item.receipt).collect())
    }

    fn within(&self, root: &Path, path: &Path) -> Result<(), String> {
        real_within(root, path)
    }

    fn is_dir(&self, path: &Path) -> Result<bool, String> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| format!("看 {} 失败：{}", path.display(), e))?;
        Ok(meta.file_type().is_dir())
    }

    fn delete(&self, path: &Path, recursive: bool) -> Result<(), String> {
        let lock = self.lock_dir.is_none().then(|| path_lock(path));
        let _guard = lock
            .as_ref()
            .map(|lock| lock.lock().unwrap_or_else(|e| e.into_inner()));
        let _file = self.lock_path(path).map_err(|fault| fault.message)?;
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| format!("看 {} 失败：{}", path.display(), e))?;
        if meta.file_type().is_symlink() {
            return remove_link(path);
        }
        if meta.is_dir() {
            if !recursive {
                return Err(format!(
                    "{} 是目录；要整棵删除，加 recursive=true",
                    path.display()
                ));
            }
            return remove_tree(path);
        }
        std::fs::remove_file(path).map_err(|e| format!("删除 {} 失败：{}", path.display(), e))
    }

    fn list(&self, path: &Path) -> Result<Vec<DirEntry>, String> {
        let mut out: Vec<DirEntry> = Vec::new();
        for ent in std::fs::read_dir(path).map_err(|e| format!("列目录失败：{}", e))? {
            let ent = ent.map_err(|e| format!("列目录失败：{}", e))?;
            let meta = ent.metadata().map_err(|e| format!("读目录项失败：{}", e))?;
            out.push(DirEntry {
                name: ent.file_name().to_string_lossy().into_owned(),
                is_dir: meta.is_dir(),
                bytes: if meta.is_dir() { 0 } else { meta.len() },
            });
        }
        // 名字排序：回执稳定，模型两次列同一目录看到同样的顺序。
        out.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(out)
    }

    fn read_small(&self, path: &Path) -> Result<Option<String>, String> {
        match std::fs::read(path) {
            Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("读取失败（{}）：{}", path.display(), e)),
        }
    }

    fn walk(
        &self,
        root: &Path,
        rules: Option<&str>,
        opts: WalkOpts,
    ) -> Result<Vec<WalkEntry>, String> {
        let meta = std::fs::metadata(root).map_err(|e| format!("读取失败：{}", e))?;
        if !meta.is_dir() {
            return Err(format!("不是目录：{}", root.display()));
        }
        let rules = match (opts.ignore, rules) {
            (true, Some(text)) => Some(std::sync::Arc::new(Rules::compile(root, text)?)),
            _ => None,
        };
        let mut builder = ignore::WalkBuilder::new(root);
        builder
            .hidden(!opts.hidden)
            // 忽略文件与 .gitignore 同进同退；不读全局 ignore、不向上找父目录的规则：
            // 一次遍历只看这棵树与这份工作区规则，结果与机器上的全局配置无关。
            .ignore(opts.ignore)
            .git_ignore(opts.ignore)
            .git_exclude(opts.ignore)
            .git_global(false)
            .parents(false)
            .require_git(false)
            .follow_links(false)
            .sort_by_file_path(|a, b| a.cmp(b));
        if let Some(rules) = rules {
            let base = root.to_path_buf();
            builder.filter_entry(move |ent| {
                let Ok(rel) = ent.path().strip_prefix(&base) else {
                    return true;
                };
                if rel.as_os_str().is_empty() {
                    return true;
                }
                let is_dir = ent.file_type().map(|t| t.is_dir()).unwrap_or(false);
                !rules.is_ignored(&crate::kernel::api::path::slash(rel), is_dir)
            });
        }
        let mut out: Vec<WalkEntry> = Vec::new();
        for item in builder.build() {
            // 读不了的目录项直接跳过（遍历不因为一处权限问题整次失败）；文件清单里只留文件。
            let Ok(ent) = item else { continue };
            if ent.depth() == 0 {
                continue;
            }
            if !ent.file_type().map(|t| t.is_file()).unwrap_or(false) {
                continue;
            }
            let rel = ent
                .path()
                .strip_prefix(root)
                .map_err(|_| format!("遍历结果不在根内：{}", ent.path().display()))?;
            out.push(WalkEntry {
                rel: crate::kernel::api::path::slash(rel),
                bytes: ent.metadata().map(|m| m.len()).unwrap_or(0),
            });
        }
        out.sort_by(|a, b| a.rel.cmp(&b.rel));
        Ok(out)
    }

    fn scan_lines(
        &self,
        path: &Path,
        each: &mut dyn FnMut(u32, &str),
    ) -> Result<ScanSummary, String> {
        let mut file = std::fs::File::open(path).map_err(|e| format!("读取失败：{}", e))?;
        let mut head = vec![0u8; 8 * 1024];
        let n = file
            .read(&mut head)
            .map_err(|e| format!("读取失败：{}", e))?;
        head.truncate(n);
        if probe_binary(&head) {
            return Ok(ScanSummary {
                binary: true,
                ..ScanSummary::default()
            });
        }
        // 已经读掉的头部接回流前：二进制判定不额外读第二遍，也不丢开头。
        let mut reader =
            std::io::BufReader::with_capacity(64 * 1024, std::io::Cursor::new(head).chain(file));
        let mut summary = ScanSummary::default();
        let mut buf: Vec<u8> = Vec::new();
        loop {
            buf.clear();
            let (got, cut) = read_capped_line(&mut reader, &mut buf, MAX_SCAN_LINE_BYTES)?;
            if !got {
                return Ok(summary);
            }
            summary.lines += 1;
            if cut {
                summary.long_lines += 1;
            }
            let text = line_text(&buf, &mut summary.lossy);
            each(summary.lines as u32, &text);
        }
    }
}
