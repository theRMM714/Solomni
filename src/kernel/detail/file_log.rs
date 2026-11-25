//! 目的：文件日志适配器——kernel `Log` 端口的落盘实现。
//! 管：每次运行在 `logs/` 下按时间戳建一个文件、逐行追加；进程内全局共享同一个句柄，并对外给出路径。
//! 不管：什么时候记、记什么级别（调用方决定）；日志轮转与归档。
//! 联动：端口在 `src/kernel/ports.rs`；由入口层的组合根构造（`src/entry/`）。

use crate::kernel::detail::clock;
use crate::kernel::ports::Log;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct FileLog {
    file: Mutex<File>,
    /// 目的：本次运行的日志文件路径；清理日志时用它把这一份排除。
    path: PathBuf,
    /// 构造说明（进程参数），写在构造调用方，不占字段。
    _origin: String,
}

impl FileLog {
    /// 目的：创建本次运行的时间戳日志文件（落在 `root/logs/`）。
    /// 参数：`origin` 是这次运行的说明（进程参数），写进文件头供对账。
    pub fn new(root: &std::path::Path, origin: &str) -> Result<FileLog, String> {
        let dir = crate::kernel::domain::layout::logs(root);
        fs::create_dir_all(&dir).map_err(|e| format!("建日志目录失败：{}", e))?;
        // 时间戳：本地时区的可读名 + 毫秒（防同秒重启覆盖）。
        let now = clock::local_now();
        let name = format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
            now.year, now.month, now.day, now.hour, now.minute, now.second, now.millis
        );
        let path: PathBuf = dir.join(name).with_extension("log");
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| format!("建日志文件失败：{}", e))?;
        let mut file = file;
        let _ = writeln!(file, "=== {}（{}）===", origin, path.display());
        Ok(FileLog {
            file: Mutex::new(file),
            path,
            _origin: String::new(),
        })
    }

    /// 目的：本次运行的日志文件路径。
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn write(&self, level: &str, at: &str, msg: &str) {
        let now = clock::local_now();
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{}] {}: {}\n",
            now.year,
            now.month,
            now.day,
            now.hour,
            now.minute,
            now.second,
            now.millis,
            level,
            at,
            msg
        );
        if let Ok(mut f) = self.file.lock() {
            let _ = f.write_all(line.as_bytes());
            let _ = f.flush();
        }
    }
}

impl Log for FileLog {
    fn info(&self, at: &str, msg: &str) {
        self.write("信息", at, msg);
    }
    fn warn(&self, at: &str, msg: &str) {
        self.write("警告", at, msg);
    }
    fn error(&self, at: &str, msg: &str) {
        self.write("错误", at, msg);
    }
}
