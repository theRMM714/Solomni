//! 文件日志适配器：实现 core 的 Log 端口。
//! 机制：每次运行在 logs/ 下按时间戳创建一个文件；逐行追加；进程内全局共享。
use crate::core::ports::Log;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

pub struct FileLog {
    file: Mutex<File>,
    /// 构造说明（进程参数），写在构造调用方，不占字段。
    _origin: String,
}

impl FileLog {
    /// 在 root/logs/ 下创建本次运行的时间戳日志文件。
    pub fn new(root: &std::path::Path, origin: &str) -> Result<FileLog, String> {
        let dir = root.join("logs");
        fs::create_dir_all(&dir).map_err(|e| format!("建日志目录失败：{}", e))?;
        // 时间戳：本地可读 + 毫秒（防同秒重启覆盖）。
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs();
        let ms = now.subsec_millis();
        // 简单 UTC+8 折算（本地环境）；日志仅为对账用，不追求时区完美。
        let (y, mo, d, h, mi, s) = stamp(secs + 8 * 3600);
        let name = format!(
            "{:04}{:02}{:02}-{:02}{:02}{:02}-{:03}",
            y, mo, d, h, mi, s, ms
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
            _origin: String::new(),
        })
    }

    fn write(&self, level: &str, at: &str, msg: &str) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let (y, mo, d, h, mi, s) = stamp(now.as_secs() + 8 * 3600);
        let line = format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} [{}] {}: {}\n",
            y,
            mo,
            d,
            h,
            mi,
            s,
            now.subsec_millis(),
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

/// 秒（偏移后）→ (年,月,日,时,分,秒)。民用时间换算，不引库。
fn stamp(secs: u64) -> (u64, u64, u64, u64, u64, u64) {
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, mi, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 1970-01-01 起的民用年月日（Howard Hinnant 算法简化）。
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u64;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u64;
    let y = if m <= 2 { y + 1 } else { y } as u64;
    (y, m, d, h, mi, s)
}
