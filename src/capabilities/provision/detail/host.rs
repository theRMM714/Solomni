//! 目的：宿主文件系统落点——把 `HostFs` 的少数操作落到 std::fs。
//! 管：存在性、建目录、原子写文本、删文件 / 目录、补执行位。
//! 不管：装什么、装到哪（在 service 的编排里）；解包与镜像构造（在 install / guest_image）。
//! 联动：组合根构造 `RealHost` 注入 service；台账与安装器也直接用它。

use std::fs;
use std::path::Path;

use crate::capabilities::provision::ports::HostFs;

/// 目的：本机文件系统的 `HostFs` 实现——无状态，可随处构造一份共享。
#[derive(Debug, Default)]
pub struct RealHost;

impl HostFs for RealHost {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    fn mkdir_all(&self, path: &Path) -> Result<(), String> {
        fs::create_dir_all(path).map_err(|e| format!("建 {} 失败：{}", path.display(), e))
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        fs::remove_file(path).map_err(|e| format!("删 {} 失败：{}", path.display(), e))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), String> {
        fs::remove_dir_all(path).map_err(|e| format!("删 {} 失败：{}", path.display(), e))
    }

    fn make_executable(&self, path: &Path) {
        crate::kernel::api::make_executable(path);
    }
}

/// 目的：建软链（平台机制由 kernel 的 `fs_meta` 后端提供；Windows 如实报不支持）。
pub(crate) fn symlink(target: &str, link: &Path) -> std::io::Result<()> {
    crate::kernel::api::symlink(target, link)
}

/// 目的：跑一个外部工具并判成功；失败给一句人话。
pub(crate) fn run_tool(cmd: &mut std::process::Command, name: &str) -> Result<(), String> {
    let st = cmd
        .status()
        .map_err(|e| format!("跑不了 {}（缺这个系统工具？）：{}", name, e))?;
    if !st.success() {
        return Err(format!("{} 失败", name));
    }
    Ok(())
}

/// 目的：写一个可执行脚本并给执行位（unix 0755；别的平台只写文件）。
pub(crate) fn write_exe(path: &Path, body: &str) -> Result<(), String> {
    crate::kernel::api::write_executable(path, body)
        .map_err(|e| format!("写 {} 失败：{}", path.display(), e))
}

/// 目的：给已落盘的可执行文件补齐执行位（解归档丢过权限位的安装靠它自愈）；不是文件就什么都不做。
pub(crate) fn ensure_executable(path: &Path) {
    crate::kernel::api::ensure_executable(path);
}
