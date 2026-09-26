//! HostProbe 的机制实现：读环境变量、查路径存在性、按平台判定虚拟化能力。
//! **只读事实**：不执行任何程序、不安装、不写任何东西。

use crate::kernel::host::HostProbe;
use std::path::Path;

#[cfg(windows)]
const EXE_SUFFIX: &str = ".exe";
#[cfg(not(windows))]
const EXE_SUFFIX: &str = "";

/// 在 PATH 里找一个可执行文件，返回**真实路径**（找不到就是没有，不去别处翻）。
/// 平台扩展名按 `PATHEXT` 展开（Windows 不设时用平台后缀）——**查法只有这一处**，
/// `has_exe` 与自检报告共用它，免得两处各写一遍再慢慢漂移。
pub fn find_exe(name: &str) -> Option<std::path::PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    let exts: Vec<String> = std::env::var("PATHEXT")
        .map(|v| {
            v.split(';')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect()
        })
        .unwrap_or_else(|_| vec![EXE_SUFFIX.to_string()]);
    for dir in std::env::split_paths(&path_var) {
        for ext in &exts {
            let candidate = dir.join(format!("{}{}", name, ext.to_lowercase()));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

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
        find_exe(name).is_some()
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
