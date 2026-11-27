//! 目的：执行环境另一侧的 OS 与架构——沙箱事实的一部分（哪一侧、什么系统、什么架构）。
//! 管：`GuestOs`（三种系统 + 路径方言 + 平台写法 + 解析）、`GuestArch`（宿主能虚拟化的架构）。
//! 不管：镜像里装了什么（`vm` 的 `GuestImage`）；路径怎么拼（`kernel::api::path_bridge`）。
//! 联动：`SandboxFacts` 的 `guest_os` / `guest_arch` 由策略按本类型取；`vm` 与 `provision` 经本单元的 `api` 使用。

use crate::kernel::api::path_bridge::PathDialect;

/// 目的：guest 的处理器架构（镜像架构要与宿主能虚拟化的架构一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestArch {
    Amd64,
    Arm64,
}

impl GuestArch {
    /// 目的：平台名里用的架构写法（amd64 → x86_64；arm64 → aarch64）。
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestArch::Amd64 => "x86_64",
            GuestArch::Arm64 => "aarch64",
        }
    }

    /// 目的：本机能虚拟化的架构（x86_64 → amd64；aarch64 → arm64）。
    pub fn host() -> GuestArch {
        if crate::kernel::api::platform::HOST_ARCH == "aarch64" {
            GuestArch::Arm64
        } else {
            GuestArch::Amd64
        }
    }
}

/// 目的：guest 的 OS——决定它认识的路径拼法（路径桥按它选方言），也是以后接非 Linux guest 的钥匙。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuestOs {
    Linux,
    Macos,
    Windows,
}

impl GuestOs {
    /// 目的：这个 OS 的路径方言。
    pub fn dialect(&self) -> PathDialect {
        match self {
            GuestOs::Windows => PathDialect::Windows,
            GuestOs::Linux | GuestOs::Macos => PathDialect::Posix,
        }
    }

    /// 目的：平台名里用的 os 写法（也是 guest.yaml 的 os 取值与工作环境册 env_guests 的键）。
    pub fn as_str(&self) -> &'static str {
        match self {
            GuestOs::Linux => "linux",
            GuestOs::Macos => "macos",
            GuestOs::Windows => "windows",
        }
    }

    /// 目的：解析 guest.yaml 的 os；不认识如实拒收。
    pub fn parse(s: &str) -> Result<GuestOs, String> {
        match s {
            "linux" => Ok(GuestOs::Linux),
            "macos" => Ok(GuestOs::Macos),
            "windows" => Ok(GuestOs::Windows),
            other => Err(format!(
                "guest.yaml 里的 os 不认识：{}（认 linux / macos / windows）",
                other
            )),
        }
    }
}
