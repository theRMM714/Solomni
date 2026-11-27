//! 目的：宿主文件系统落点——把 `HostFs` 的少数操作落到内核的文件事务面。
//! 管：存在性、建目录、删文件 / 目录、补执行位、跑外部工具并判成功。
//! 不管：装什么、装到哪（在 service 的编排里）；解包与镜像构造（在 install / guest_image）。
//! 联动：组合根构造 `RealHost` 注入 service；台账与安装器也直接用它。

use crate::kernel::api::fs;
use std::path::Path;

use crate::units::business::provision::ports::HostFs;

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
        fs::ensure_dir(path)
            .map_err(|fault| format!("建 {} 失败：{}", path.display(), fault.message))
    }

    fn remove_file(&self, path: &Path) -> Result<(), String> {
        fs::remove_tree(path)
            .map_err(|fault| format!("删 {} 失败：{}", path.display(), fault.message))
    }

    fn remove_dir_all(&self, path: &Path) -> Result<(), String> {
        fs::remove_tree(path)
            .map_err(|fault| format!("删 {} 失败：{}", path.display(), fault.message))
    }

    fn make_executable(&self, path: &Path) {
        crate::kernel::api::fs::make_executable(path);
    }

    fn try_lock(&self, path: &Path) -> Result<Option<Box<dyn Send>>, String> {
        let file = crate::kernel::api::lock::try_lock(path, "构建缓存卷")?;
        Ok(file.map(|f| Box::new(f) as Box<dyn Send>))
    }
}

/// 目的：建软链（平台机制由 kernel 的 `fs_meta` 后端提供；Windows 如实报不支持）。
pub(crate) fn symlink(target: &str, link: &Path) -> std::io::Result<()> {
    crate::kernel::api::fs::symlink(target, link)
}

/// 目的：跑一个宿主工具到结束——命令是数据、不经 shell；超时与缺工具都折成一句人话。
/// 参数：`program` 走 PATH；`args` 逐项；`cwd` 省缺用本进程目录；`timeout` 到点连根杀树。
/// 返回：退出码与两路原始输出；成败由调用方按 `done.ok()` 与 `done.stderr_text()` 判。
pub(crate) fn run_tool(
    program: &str,
    args: Vec<String>,
    cwd: Option<&Path>,
    timeout: std::time::Duration,
) -> Result<crate::kernel::api::process::Done, String> {
    let run = crate::kernel::api::process::Run {
        program: Path::new(program),
        args: &args,
        env: &[],
        cwd,
        timeout,
        stdin: None,
    };
    crate::kernel::api::process::run_to_end(&run)
        .map_err(|e| format!("跑不了 {program}（缺这个系统工具？）：{e}"))
}

/// 目的：写一个可执行脚本并给执行位（unix 0755；别的平台只写文件）。
pub(crate) fn write_exe(path: &Path, body: &str) -> Result<(), String> {
    crate::kernel::api::fs::write_executable(path, body)
        .map_err(|e| format!("写 {} 失败：{}", path.display(), e))
}

/// 目的：给已落盘的可执行文件补齐执行位（解归档丢过权限位的安装靠它自愈）；不是文件就什么都不做。
pub(crate) fn ensure_executable(path: &Path) {
    crate::kernel::api::fs::ensure_executable(path);
}
