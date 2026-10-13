//! 目的：ShareBackend 的 QGA 复制实现——宿主目录与 guest 之间按需打包/解包（三平台通用，不需要宿主侧文件服务）。
//! 管：prepare 里的建目录脚本；sync_in / sync_out 的 tar 打包与经命令通道的文件搬运（每趟同步语义）。
//! 不管：命令通道与 QGA 细节（detail/qemu.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有；实时共享方案见 tests/gaps.yaml。

use crate::capabilities::vm::domain::guest::{GuestAgent, GuestImage, GuestMount, GuestRun};
use crate::capabilities::vm::ports::{
    GuestChannel, NoShareGuard, ShareBackend, ShareGuard, SharePlan,
};
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

/// 目的：QGA 复制共享后端——不依赖宿主侧文件服务，靠命令通道搬 tar。
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

    /// 约束：只有 QEMU/QGA 这条通道实现了 put_file / get_file，所以 guest 要声明 QGA agent。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool {
        engine == "qemu" && matches!(guest.agent, GuestAgent::Qga)
    }

    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        // 复制后端没有引擎参数；只需在 guest 里把挂载点建出来（内容由 sync_in 拷进去）。
        let guest_scripts = mounts
            .iter()
            .map(|m| format!("mkdir -p '{}'", m.guest))
            .collect();
        Ok((
            SharePlan {
                kind: "copy",
                engine_args: Vec::new(),
                guest_scripts,
                needs_net: false,
            },
            Box::new(NoShareGuard),
        ))
    }

    fn sync_in(&self, channel: &mut dyn GuestChannel, mounts: &[GuestMount]) -> Result<(), String> {
        for (i, m) in mounts.iter().enumerate() {
            let tar = host_tar(&m.host)?;
            let tmp = format!("/tmp/solomni-copy-in-{}.tar", i);
            channel.put_file(&tmp, &tar)?;
            let out = channel.exec(&GuestRun {
                command: format!("tar -xf '{}' -C '{}' && rm -f '{}'", tmp, m.guest, tmp),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
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
                command: format!("tar -cf '{}' -C '{}' .", tmp, m.guest),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
            });
            if !out.ok {
                return Err(format!("打包 guest 的 {} 失败：{}", m.guest, out.output));
            }
            let tar = channel.get_file(&tmp)?;
            host_untar(&m.host, &tar)?;
            let _ = channel.exec(&GuestRun {
                command: format!("rm -f '{}'", tmp),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: String::new(),
            });
        }
        Ok(())
    }
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
