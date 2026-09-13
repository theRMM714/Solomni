//! 内置文件工具的读写机制（实现 core 的 SysIo 端口）：纯 Rust 直接读写，不经任何外部进程。
//! 读：按 UTF-8 解码；含非法字节时按替换字符呈现并置 lossy（编码猜测不是本程序的事）。
//! 写：一律 UTF-8，需要时建父目录，覆盖同名文件。

use crate::core::ports::{FileRead, SysIo};
use std::path::Path;

pub struct FsSysIo {
    /// 单次读取的字节上限（超出只读开头部分，如实标注）。
    pub max_read_bytes: usize,
}

impl Default for FsSysIo {
    fn default() -> FsSysIo {
        FsSysIo { max_read_bytes: 1_000_000 }
    }
}

impl SysIo for FsSysIo {
    fn read(&self, path: &Path) -> Result<FileRead, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("读取失败：{}", e))?;
        let total = bytes.len();
        let n = self.max_read_bytes.min(total);
        let (text, lossy) = match std::str::from_utf8(&bytes[..n]) {
            Ok(t) => (t.to_string(), false),
            Err(e) => {
                if e.error_len().is_none() && n < total {
                    // 只是切片落在多字节字符中间：退回完整字符边界，不算编码错误。
                    (String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned(), false)
                } else {
                    (String::from_utf8_lossy(&bytes[..n]).into_owned(), true)
                }
            }
        };
        Ok(FileRead { text, bytes: total, lossy, cut: n < total })
    }

    fn write(&self, path: &Path, content: &str) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("建目录失败：{}", e))?;
        }
        std::fs::write(path, content.as_bytes()).map_err(|e| format!("写入失败：{}", e))
    }
}
