//! 目的：登记处目录的文件系统适配器（实现 `ports::HomeFiles`）。
//! 管：读一份文本（不存在 = None）与写一份文本（原子发布、私密文件的权限收紧都在内核文件事务里）。
//! 不管：文档名与私密策略（在 domain）、yaml（在 catalog）、目录路径（组合根给）。
//! 联动：只经 `HomeService` 使用；路径来自 `kernel::domain::layout::home`。

use crate::kernel::api::fs::{self, Expect};
use crate::units::resource::home::ports::HomeFiles;
use std::path::PathBuf;

/// 目的：登记处目录的文件面（组合根构造）。
pub struct FsHome {
    dir: PathBuf,
}

impl FsHome {
    /// 目的：用登记处目录造一个文件面（组合根给 `data/.home`）。
    pub fn new(dir: PathBuf) -> FsHome {
        FsHome { dir }
    }
}

impl HomeFiles for FsHome {
    fn read(&self, name: &str) -> Result<Option<String>, String> {
        match std::fs::read_to_string(self.dir.join(name)) {
            Ok(t) => Ok(Some(t)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    }

    fn write(&self, name: &str, text: &str, private: bool) -> Result<(), String> {
        fs::ensure_dir(&self.dir)
            .map_err(|fault| format!("建登记处目录失败：{}", fault.message))?;
        let path = self.dir.join(name);
        let written = if private {
            fs::write_private(None, &path, text.as_bytes(), Expect::Any)
        } else {
            fs::write(None, &path, text.as_bytes(), Expect::Any)
        };
        written
            .map(|_| ())
            .map_err(|fault| format!("写登记处失败：{}", fault.message))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!("solomni-home-{}-{}", tag, std::process::id()))
    }

    /// 目的：缺文件不报错（首启没有设置），写后读回原样，且不留临时文件。
    #[test]
    fn missing_reads_none_and_write_round_trips() {
        let dir = tmp("roundtrip");
        let _ = std::fs::remove_dir_all(&dir);
        let files = FsHome::new(dir.clone());
        assert_eq!(files.read("settings.yaml").unwrap(), None);
        files.write("settings.yaml", "a: 1\n", true).unwrap();
        assert_eq!(
            files.read("settings.yaml").unwrap().as_deref(),
            Some("a: 1\n")
        );
        let left: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(left, vec!["settings.yaml".to_string()]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 目的：写会自建目录（首启登记处目录还不存在）。
    #[test]
    fn write_creates_the_directory() {
        let dir = tmp("mkdir");
        let _ = std::fs::remove_dir_all(&dir);
        let files = FsHome::new(dir.join("nested"));
        files.write("agents.yaml", "agents: {}\n", true).unwrap();
        assert!(dir.join("nested").join("agents.yaml").is_file());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
