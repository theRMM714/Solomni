//! 内置文件工具的读写机制（实现 tools 自己的 SysIo 端口）：纯 Rust 直接读写，不经任何外部进程。
//! 读：按 UTF-8 解码；含非法字节时按替换字符呈现并置 lossy（编码猜测不是本程序的事）。
//! 写：一律 UTF-8，需要时建父目录，覆盖同名文件。

use crate::capabilities::tools::ports::{DirEntry, FileRead, SysIo};
use std::io::Read;
use std::path::Path;

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
        let meta = std::fs::symlink_metadata(&p)
            .map_err(|e| format!("看 {} 失败：{}", p.display(), e))?;
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

pub struct FsSysIo {
    /// 单次读取的字节上限（超出只读开头部分，如实标注）。
    pub max_read_bytes: usize,
}

impl Default for FsSysIo {
    fn default() -> FsSysIo {
        FsSysIo {
            max_read_bytes: 1_000_000,
        }
    }
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
        Ok(FileRead {
            text,
            bytes: total,
            lossy,
            cut: n < total,
        })
    }

    fn write(&self, path: &Path, content: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{}", e))?;
        }
        std::fs::write(path, content.as_bytes()).map_err(|e| format!("写入失败：{}", e))
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
}
