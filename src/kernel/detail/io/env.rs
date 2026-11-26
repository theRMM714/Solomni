//! 目的：进程环境的唯一读取面——宿主事实与产品开关都从这里出去，别的代码不再各自摸 `std::env`。
//! 管：`var` / `os` 两个原语，以及具名的宿主事实（PATH、PATHEXT、SystemRoot、LOCALAPPDATA、临时目录、可执行文件）；
//!   `std::env::` 只许在这里、启动面（`src/main.rs` / `src/entry/`）与呈现面（`src/presentation/`，终端能力探测）出现。
//! 不管：值的含义与真假判断（`enabled(...)` 之类留在调用方）；写环境（启动面自己做，见 `src/entry/`）。
//! 联动：单元经 `src/kernel/api/env.rs` 取，内核各机制（launch / host_probe / confine / relay / ledger / i18n）直接取本机制；门禁 `structure/env-surface` 判上面那条白名单。

use std::ffi::OsString;
use std::path::PathBuf;

/// 目的：读一个环境变量的文本值（不存在或不是 UTF-8 都是 `None`）。
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// 目的：读一个环境变量的原始值（路径类用它，别先转 String 再转回来）。
pub fn os(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// 目的：本进程的 PATH 原始串；解析成目录用调用方的 `std::env::split_paths`（纯字符串处理，不算 IO）。
pub fn path_var() -> Option<OsString> {
    std::env::var_os("PATH")
}

/// 目的：可执行扩展名清单（Windows 的 `PATHEXT`；缺席时调用方按平台后缀兜底）。
pub fn pathext() -> Option<String> {
    std::env::var("PATHEXT").ok()
}

/// 目的：系统目录（Windows 用它拼 `System32`；别的平台没有就是 `None`）。
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "系统目录只有 Windows 的容器后端会取；非 Windows 保留同一面，调用方无需按平台分叉"
    )
)]
pub fn system_root() -> Option<PathBuf> {
    std::env::var_os("SystemRoot").map(PathBuf::from)
}

/// 目的：本用户的本地应用数据根（Windows 的容器 profile 落在这里）。
#[cfg_attr(
    not(windows),
    allow(
        dead_code,
        reason = "本地应用数据根只有 Windows 的残留清扫会取；非 Windows 保留同一面，调用方无需按平台分叉"
    )
)]
pub fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

/// 目的：本进程的可执行文件路径（中继要以自己为壳再跑一遍，取证也要它）。
#[cfg_attr(
    target_os = "linux",
    allow(
        dead_code,
        reason = "Linux 的父死子亡由内核信号给、不必拿自身当壳；Windows 中继与 macOS 守门进程才取自身路径"
    )
)]
pub fn current_exe() -> std::io::Result<PathBuf> {
    std::env::current_exe()
}

/// 目的：系统临时目录（自检的落点；产品数据一律不落这里）。
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

#[cfg(test)]
mod tests {
    /// 目的：原语只是薄薄一层——设过的读得回来，没设过的是 None，原始值不丢字节。
    #[test]
    fn a_set_variable_round_trips_and_an_unset_one_is_none() {
        // SAFETY：测试进程里独占一个只在本用例出现的名字，读写都在同一线程内完成。
        unsafe { std::env::set_var("SOLOMNI_ENV_SURFACE_PROBE", "值-✓") };
        assert_eq!(
            super::var("SOLOMNI_ENV_SURFACE_PROBE").as_deref(),
            Some("值-✓")
        );
        assert_eq!(
            super::os("SOLOMNI_ENV_SURFACE_PROBE").map(|v| v.to_string_lossy().into_owned()),
            Some("值-✓".to_string())
        );
        assert!(super::var("SOLOMNI_ENV_SURFACE_ABSENT").is_none());
        assert!(super::os("SOLOMNI_ENV_SURFACE_ABSENT").is_none());
        unsafe { std::env::remove_var("SOLOMNI_ENV_SURFACE_PROBE") };
        assert!(super::os("SOLOMNI_ENV_SURFACE_PROBE").is_none());
    }

    /// 目的：临时目录一定在——自检全靠它落脚。
    #[test]
    fn the_temp_dir_exists() {
        assert!(super::temp_dir().is_absolute());
    }
}
