//! 目的：内置文件工具的读写机制（实现 tools 自己的 `SysIo` 端口）——读、遍历与扫描直接做，写与删交给内核的文件事务。
//! 管：读（按 UTF-8 解码，含非法字节时按替换字符呈现并置 `lossy`）、遍历与扫描、真实落点越界校验；写与删只经 `crate::kernel::api::fs` 的事务。
//! 不管：编码猜测（不是本程序的事）；参数校验、回执文案与放行（在 `domain`）；拿锁、暂存、发布与回滚（在内核）。
//! 联动：实现 `ports::SysIo`，由 `main.rs` 构造注入 `service/`。

use crate::kernel::api::fs::{self, Fault, LockSpace};
use crate::kernel::api::hash::{Sha256, content_hash};
use crate::units::business::tools::api::LineRange;
use crate::units::business::tools::ports::{
    DirEntry, Expect, FilePage, FileRead, MAX_SCAN_LINE_BYTES, Mutation, ScanSummary, SysIo,
    WalkEntry, WalkOpts, WriteFault, WriteOp, WriteReceipt,
};
use crate::units::business::workbench::api::Rules;
use std::io::Read;
use std::path::{Path, PathBuf};

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

/// 目的：内置文件工具的机制——读、遍历与扫描在本文件，写与删一律走内核的文件事务。
/// 约束：`space` 决定跨进程锁落在哪个目录；`None` 时只用进程内锁（测试与没有产品根的场景）。
pub struct FsSysIo {
    /// 目的：单页收集的字节缓冲上限（只影响一次读最多收集多少内容，不是模型可见的额度）。
    pub max_read_bytes: usize,
    /// 目的：跨进程锁的落点与等待；`None` 只用进程内锁。
    pub space: Option<LockSpace>,
}

impl Default for FsSysIo {
    fn default() -> FsSysIo {
        FsSysIo {
            max_read_bytes: 1_000_000,
            space: None,
        }
    }
}

impl FsSysIo {
    /// 目的：带产品锁目录构造：写路径按落点身份跨进程互斥。
    pub fn with_lock_dir(lock_dir: PathBuf) -> FsSysIo {
        FsSysIo {
            space: Some(LockSpace::new(lock_dir)),
            ..FsSysIo::default()
        }
    }

    /// 目的：换一个拿锁的有界等待（测试里把等待压到几十毫秒）。
    #[cfg(test)]
    pub fn with_wait(mut self, wait: std::time::Duration) -> FsSysIo {
        self.space = self.space.map(|space| space.with_wait(wait));
        self
    }
}

/// 目的：把内核的失败码翻成写工具自己的码（模型与呈现层认后者）。
fn fault_of(fault: Fault) -> WriteFault {
    let code = match fault.code {
        fs::FS_STALE => "FS_STALE_VERSION",
        fs::FS_EXISTS => "FS_ALREADY_EXISTS",
        fs::FS_MISSING => "FS_NOT_FOUND",
        other => other,
    };
    WriteFault::new(code, fault.message)
}

/// 目的：把写工具自己的失败码带进事务闭包（锚点类失败也从这条路回给模型）。
fn as_fault(fault: WriteFault) -> Fault {
    Fault::new(fault.code, fault.message)
}

/// 目的：把写工具期望的旧状态翻成内核的期望。
fn expect_of(expect: &Expect) -> fs::Expect {
    match expect {
        Expect::Absent => fs::Expect::Absent,
        Expect::Version(version) => fs::Expect::Version(version.clone()),
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
        fs::transact_with(self.space.as_ref(), &[path], "文件写入", |tx| {
            let current = tx.read(path)?;
            let (text, changed, replaced) =
                apply_mutation(path, current.as_deref(), mutation).map_err(as_fault)?;
            let version = content_hash(text.as_bytes());
            let total_lines = text.lines().count() as u32;
            tx.stage(path, Some(text.as_bytes()), expect_of(expect))?;
            tx.commit()?;
            Ok(WriteReceipt {
                version,
                total_lines,
                changed,
                replaced,
            })
        })
        .map_err(fault_of)
    }

    fn transact(&self, ops: Vec<WriteOp>) -> Result<Vec<WriteReceipt>, WriteFault> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        let paths: Vec<&Path> = ops.iter().map(|op| op.path.as_path()).collect();
        fs::transact_with(self.space.as_ref(), &paths, "文件写入", |tx| {
            let mut receipts = Vec::with_capacity(ops.len());
            for op in &ops {
                let current = tx.read(&op.path)?;
                let (text, changed, replaced) =
                    apply_mutation(&op.path, current.as_deref(), op.mutation.clone())
                        .map_err(as_fault)?;
                let version = content_hash(text.as_bytes());
                let total_lines = text.lines().count() as u32;
                tx.stage(&op.path, Some(text.as_bytes()), expect_of(&op.expect))?;
                receipts.push(WriteReceipt {
                    version,
                    total_lines,
                    changed,
                    replaced,
                });
            }
            tx.commit()?;
            Ok(receipts)
        })
        .map_err(fault_of)
    }

    fn within(&self, root: &Path, path: &Path) -> Result<(), String> {
        fs::within(root, path).map_err(|fault| fault.message)
    }

    fn is_dir(&self, path: &Path) -> Result<bool, String> {
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| format!("看 {} 失败：{}", path.display(), e))?;
        Ok(meta.file_type().is_dir())
    }

    fn delete(&self, path: &Path, recursive: bool) -> Result<(), String> {
        fs::transact_with(self.space.as_ref(), &[path], "删除", |_tx| {
            let meta = match std::fs::symlink_metadata(path) {
                Ok(meta) => meta,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(e) => return Err(Fault::io(path, e)),
            };
            if meta.is_dir() && !meta.file_type().is_symlink() && !recursive {
                return Err(Fault::new(
                    fs::FS_IO,
                    format!("{} 是目录；要整棵删除，加 recursive=true", path.display()),
                ));
            }
            fs::remove_tree(path)
        })
        .map_err(|fault| fault.message)
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
