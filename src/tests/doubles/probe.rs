//! 目的：宿主探测替身——机器能力与依赖在场与否都可控。
//! 管：文件/目录/可执行判断、可执行名解析、虚拟化能力、读文本、规范化、元数据与可写探测。
//! 不管：依赖状态（`host.rs`）、真实探测（`src/kernel/detail/`）。
//! 联动：`src/kernel/ports.rs` 的 `HostProbe`。

use super::*;

impl crate::kernel::ports::HostProbe for FixedProbe {
    fn is_file(&self, path: &std::path::Path) -> bool {
        self.files.iter().any(|p| p == path)
    }
    fn is_dir(&self, path: &std::path::Path) -> bool {
        self.dirs.iter().any(|p| p == path) || self.guests.iter().any(|g| path.starts_with(g))
    }
    fn is_executable(&self, path: &std::path::Path) -> bool {
        self.is_file(path) && !self.not_executable.iter().any(|p| p == path)
    }
    fn has_exe(&self, name: &str) -> bool {
        self.exes.iter().any(|n| n == name)
    }
    fn hypervisor_available(&self) -> bool {
        self.hypervisor
    }
    fn canonicalize(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        Ok(path.to_path_buf())
    }
    fn host_metadata(
        &self,
        path: &std::path::Path,
    ) -> std::io::Result<crate::kernel::ports::HostMetadata> {
        let is_dir =
            self.dirs.iter().any(|p| p == path) || self.guests.iter().any(|g| path.starts_with(g));
        let is_file = self.files.iter().any(|p| p == path);
        if is_dir || is_file {
            Ok(crate::kernel::ports::HostMetadata { is_dir })
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "模型里没有这个路径",
            ))
        }
    }
    fn probe_writable(&self, dir: &std::path::Path) -> std::io::Result<()> {
        if self.dirs.iter().any(|p| p == dir) || self.guests.iter().any(|g| dir.starts_with(g)) {
            Ok(())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "模型里没有这个目录",
            ))
        }
    }
}

/// 目的：测试用的宿主探测——真实事实 + 本平台共享工具视为在场。
/// 约束：虚拟机档的就绪判据现在是硬前置；测试不该因为开发机没装 smbd/virtiofsd 而少验一条业务路径。
pub(crate) struct TestHostProbe {
    // None = 用真实宿主事实；Some = 测试声明（VM 用例要 "本机能起虚拟机" 时不依赖 runner 有没有 /dev/kvm）。
    pub(crate) hypervisor: Option<bool>,
}

impl crate::kernel::ports::HostProbe for TestHostProbe {
    fn is_file(&self, path: &std::path::Path) -> bool {
        crate::kernel::detail::HostProbeAdapter.is_file(path)
    }
    fn is_dir(&self, path: &std::path::Path) -> bool {
        crate::kernel::detail::HostProbeAdapter.is_dir(path)
    }
    fn is_executable(&self, path: &std::path::Path) -> bool {
        crate::kernel::detail::HostProbeAdapter.is_executable(path)
    }
    fn has_exe(&self, name: &str) -> bool {
        matches!(name, "qemu-system-x86_64" | "smbd" | "virtiofsd")
            || crate::kernel::detail::HostProbeAdapter.has_exe(name)
    }
    fn hypervisor_available(&self) -> bool {
        self.hypervisor
            .unwrap_or_else(|| crate::kernel::detail::HostProbeAdapter.hypervisor_available())
    }
    fn canonicalize(&self, path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        crate::kernel::detail::HostProbeAdapter.canonicalize(path)
    }
    fn host_metadata(
        &self,
        path: &std::path::Path,
    ) -> std::io::Result<crate::kernel::ports::HostMetadata> {
        crate::kernel::detail::HostProbeAdapter.host_metadata(path)
    }
    fn probe_writable(&self, dir: &std::path::Path) -> std::io::Result<()> {
        crate::kernel::detail::HostProbeAdapter.probe_writable(dir)
    }
    fn read_text(&self, path: &std::path::Path) -> Option<String> {
        crate::kernel::detail::HostProbeAdapter.read_text(path)
    }
}

/// 目的：给测试组合根用的宿主探测（共享工具恒在场）。
pub(crate) fn test_host_probe() -> std::sync::Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>
{
    std::sync::Arc::new(TestHostProbe { hypervisor: None })
}

/// 目的：VM 用例的宿主探测——真实路径事实 + 共享工具在场 + 声明"本机能起虚拟机"（不依赖 runner 的 /dev/kvm）。
pub(crate) fn vm_capable_host_probe()
-> std::sync::Arc<dyn crate::kernel::ports::HostProbe + Send + Sync> {
    std::sync::Arc::new(TestHostProbe {
        hypervisor: Some(true),
    })
}

/// 目的：日志能力替身：什么都不做（呈现层的埋点不参与任何判定）。
pub(crate) struct NoopLogOps;
impl crate::capabilities::conductor::api::LogOps for NoopLogOps {
    fn info(&self, _at: &str, _msg: &str) {}
    fn warn(&self, _at: &str, _msg: &str) {}
    fn error(&self, _at: &str, _msg: &str) {}
}

/// 目的：测试用的提示词册能力：替身装载器 → 真实册子 → 能力面（与生产同一条路）。
pub(crate) fn test_prompt() -> std::sync::Arc<dyn crate::capabilities::prompt::api::Prompt> {
    crate::capabilities::prompt::service::load(&TestPrompts::ok()).expect("内置提示词册必须合法")
}
