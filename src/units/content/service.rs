//! 目的：内容盘的用例与端口持有者。
//! 管：持 `ContentFs`（R12）；把模块 id 变成私有区并确保它存在；安装/卸载期如实报缺口。
//! 不管：路径形状（在 `domain/paths.rs`）；文件系统调用（在 `detail/`）。
//! 联动：端口在 `ports.rs`；被 `api::Content` 的调用方消费（组合根、`catalog` 的模块清单）。

use crate::units::content::api::{Content, ContentInstall, InstallPlan, Installed, ModuleArea};
use crate::units::content::domain::paths::ContentPaths;
use crate::units::content::ports::ContentFs;
use std::path::PathBuf;

pub struct ContentService {
    paths: ContentPaths,
    fs: Box<dyn ContentFs + Send + Sync>,
}

impl ContentService {
    /// 目的：按产品安装根与文件系统适配器装配。
    /// 参数：`install` 产品安装根；`fs` 目录动作。
    /// 返回：用例面实现。
    pub fn new(install: &std::path::Path, fs: Box<dyn ContentFs + Send + Sync>) -> ContentService {
        ContentService {
            paths: ContentPaths::at(install),
            fs,
        }
    }
}

impl Content for ContentService {
    fn area(&self, id: &str) -> ModuleArea {
        self.paths.area(id)
    }

    fn ensure_userdata(&self, id: &str) -> bool {
        self.fs.ensure_dir(&self.paths.area(id).userdata())
    }

    fn runtimes(&self) -> PathBuf {
        self.paths.runtimes()
    }
}

impl ContentInstall for ContentService {
    fn installed(&self) -> Result<Vec<Installed>, String> {
        Err("列出已安装运行包还没做（缺口 content-install）".to_string())
    }

    fn plan_install(&self, _source: &str) -> Result<InstallPlan, String> {
        Err("安装运行包还没做（缺口 content-install）".to_string())
    }

    fn remove(&self, _id: &str) -> Result<(), String> {
        Err("卸载运行包还没做（缺口 content-remove）".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::content::detail::fs_content::FsContentFs;

    #[test]
    fn ensure_userdata_creates_the_private_area() {
        let root = std::env::temp_dir().join(format!("content-svc-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let svc = ContentService::new(&root, Box::new(FsContentFs));
        assert!(svc.ensure_userdata("harvest"));
        assert!(svc.area("harvest").userdata().is_dir());
        assert_eq!(
            svc.area("harvest").userdata(),
            root.join("data").join("modules").join("harvest").join("userdata")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn install_is_reported_as_a_gap() {
        let root = std::env::temp_dir().join(format!("content-gap-{}", std::process::id()));
        let svc = ContentService::new(&root, Box::new(FsContentFs));
        let why = svc.plan_install("anything").unwrap_err();
        assert!(why.contains("content-install"), "{}", why);
        let why = svc.remove("harvest").unwrap_err();
        assert!(why.contains("content-remove"), "{}", why);
        assert!(svc.installed().is_err());
    }
}
