//! 目的：ShareBackend 的 QGA 复制实现——**仅开发/应急**；宿主目录与 Linux guest 之间按趟打包/解包（不需要宿主侧文件服务）。
//! 管：prepare 里的建目录脚本；sync_in / sync_out 的 tar 打包与经命令通道的文件搬运（每趟同步语义）。
//! 不管：命令通道与 QGA 细节（detail/qemu.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：实现 ports::ShareBackend；只有 `SOLOMNI_VM_SHARE=copy` 的进程会显式选它；不参与用户链。

use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount, GuestRun, sh_quote};
use crate::capabilities::vm::ports::{
    GuestChannel, NoShareGuard, ShareBackend, ShareGuard, SharePlan,
};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// 目的：QGA 复制共享后端（开发/应急）——不依赖宿主侧文件服务，靠命令通道搬 tar。
pub struct CopyShare;

impl CopyShare {
    /// 目的：组合根专用。
    pub fn new() -> CopyShare {
        CopyShare
    }
}

impl ShareBackend for CopyShare {
    fn id(&self) -> &'static str {
        "copy"
    }

    /// 约束：copy 是开发/应急通道——空候选永不自动选中它，只有显式候选才允许。
    fn is_emergency(&self) -> bool {
        true
    }

    /// 约束：只有 QEMU/QGA 这条通道实现了 put_file / get_file，所以 guest 要声明 QGA agent；
    ///   脚本是 POSIX，只对 Linux guest 成立。copy 只作开发/应急，不参与用户链。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool {
        crate::capabilities::vm::domain::exec::share_carries("copy", engine, guest)
    }

    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
        _share_tool: Option<&str>,
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        Ok((copy_plan(mounts), Box::new(NoShareGuard)))
    }

    fn sync_in(&self, channel: &mut dyn GuestChannel, mounts: &[GuestMount]) -> Result<(), String> {
        for (i, m) in mounts.iter().enumerate() {
            let tar = host_tar(&m.host)?;
            let tmp = format!("/tmp/solomni-copy-in-{}.tar", i);
            channel.put_file(&tmp, &tar)?;
            let out = channel.exec(&GuestRun {
                command: copy_in_script(&tmp, &m.guest),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            });
            if !out.ok {
                return Err(format!("把 {} 解进 guest 失败：{}", m.guest, out.output));
            }
        }
        Ok(())
    }

    fn sync_out(
        &self,
        channel: &mut dyn GuestChannel,
        mounts: &[GuestMount],
    ) -> Result<(), String> {
        for (i, m) in mounts.iter().enumerate() {
            if m.read_only {
                continue;
            }
            let tmp = format!("/tmp/solomni-copy-out-{}.tar", i);
            let out = channel.exec(&GuestRun {
                command: copy_out_script(&tmp, &m.guest),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            });
            if !out.ok {
                return Err(format!("打包 guest 的 {} 失败：{}", m.guest, out.output));
            }
            let tar = channel.get_file(&tmp)?;
            host_untar(&m.host, &tar)?;
            let _ = channel.exec(&GuestRun {
                command: copy_cleanup_script(&tmp),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::domain::guest::OutputLimit::Head,
            });
        }
        Ok(())
    }
}

/// 目的：把挂载清单翻成复制后端的共享计划——没有引擎参数，只在 guest 里把挂载点建出来（内容由 sync_in 拷进去）。
/// 约束：guest 路径可能含单引号（外部工作区镜像进 guest 后），脚本里一律走 `sh_quote`。
pub(crate) fn copy_plan(mounts: &[GuestMount]) -> SharePlan {
    let guest_scripts = mounts
        .iter()
        .map(|m| format!("mkdir -p {}", sh_quote(&m.guest)))
        .collect();
    SharePlan {
        kind: "copy",
        engine_args: Vec::new(),
        net_options: Vec::new(),
        guest_scripts,
        needs_net: false,
        needs_shared_mem: false,
    }
}

/// 目的：把一份宿主 tar 解进 guest 挂载点并删临时包的脚本（guest 路径与临时路径一律 sh_quote）。
pub(crate) fn copy_in_script(temp: &str, guest_dir: &str) -> String {
    format!(
        "tar -xf {} -C {} && rm -f {}",
        sh_quote(temp),
        sh_quote(guest_dir),
        sh_quote(temp)
    )
}

/// 目的：把 guest 挂载点打成临时 tar 的脚本。
pub(crate) fn copy_out_script(temp: &str, guest_dir: &str) -> String {
    format!("tar -cf {} -C {} .", sh_quote(temp), sh_quote(guest_dir))
}

/// 目的：删掉 guest 里临时 tar 的脚本。
pub(crate) fn copy_cleanup_script(temp: &str) -> String {
    format!("rm -f {}", sh_quote(temp))
}

/// 目的：把宿主目录打成 tar 字节（三平台都用 PATH 上的 tar：Unix 自带、Windows 10+ 自带 bsdtar）。
pub fn host_tar(dir: &Path) -> Result<Vec<u8>, String> {
    let out = Command::new("tar")
        .arg("-cf")
        .arg("-")
        .arg("-C")
        .arg(dir)
        .arg(".")
        .output()
        .map_err(|e| format!("起不了 tar：{}", e))?;
    if !out.status.success() {
        return Err(format!(
            "tar 打包 {} 失败：{}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(out.stdout)
}

/// 目的：把 tar 字节解到宿主目录。
pub fn host_untar(dir: &Path, tar: &[u8]) -> Result<(), String> {
    let mut child = Command::new("tar")
        .arg("-xf")
        .arg("-")
        .arg("-C")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("起不了 tar：{}", e))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(tar)
            .map_err(|e| format!("给 tar 送字节失败：{}", e))?;
    }
    let out = child.wait_with_output().map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "tar 解到 {} 失败：{}",
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(())
}
