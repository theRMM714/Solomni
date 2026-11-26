//! 目的：登记处目录的文件系统适配器（实现 `ports::HomeFiles`）。
//! 管：读一份文本（不存在 = None）、建目录、临时文件 + rename 的原子写、unix 下收紧权限。
//! 不管：文档名与私密策略（在 domain）、yaml（在 catalog）、目录路径（组合根给）。
//! 联动：只经 `HomeService` 使用；路径来自 `kernel::domain::layout::home`。

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
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("建登记处目录失败：{}", e))?;
        // 临时文件与目标同目录：rename 不跨卷；文件名带进程号，避免两个实例撞车。
        let tmp = self
            .dir
            .join(format!(".{}.tmp-{}", name, std::process::id()));
        std::fs::write(&tmp, text).map_err(|e| format!("写登记处失败：{}", e))?;
        if private {
            let _ = crate::kernel::api::fs_meta::restrict_private_file(&tmp);
        }
        std::fs::rename(&tmp, self.dir.join(name)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("写登记处失败：{}", e)
        })
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
