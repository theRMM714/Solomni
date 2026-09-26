//! HostProbe 的机制实现：读环境变量、查路径存在性、按平台判定虚拟化能力。
//! **只读事实**：不执行任何程序、不安装、不写任何东西。

use crate::core::ports::HostProbe;
use std::path::Path;

#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

/// 本机事实探测（生产实现）。
pub struct HostProbeAdapter;

impl HostProbe for HostProbeAdapter {
    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn is_dir(&self, path: &Path) -> bool {
        path.is_dir()
    }

    /// 在 PATH 里找可执行文件（只问事实，不执行它）。
    fn has_exe(&self, name: &str) -> bool {
        let Some(paths) = std::env::var_os("PATH") else {
            return false;
        };
        std::env::split_paths(&paths)
            .any(|dir| dir.join(format!("{}{}", name, EXE_SUFFIX)).is_file())
    }

    /// 虚拟机监视器在场吗（只问事实，不起任何虚拟机）。
    fn hypervisor_available(&self) -> bool {
        if cfg!(windows) {
            std::env::var_os("SystemRoot")
                .map(|root| {
                    Path::new(&root)
                        .join("System32")
                        .join("WinHvPlatform.dll")
                        .is_file()
                })
                .unwrap_or(false)
        } else if cfg!(target_os = "linux") {
            Path::new("/dev/kvm").exists()
        } else {
            // macOS（11+ 都能起虚拟机）与其它平台：只问事实，不起任何虚拟机。
            cfg!(target_os = "macos")
        }
    }
}
