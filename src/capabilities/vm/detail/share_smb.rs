//! 目的：ShareBackend 的 SMB 实现——宿主侧起用户态 SMB 服务（smbd helper），guest 经 QEMU guestfwd 隧道挂 cifs。
//! 管：helper 的探测与起停、guestfwd 隧道参数、guest 端 mount -t cifs 脚本与 helper 存活凭据。
//! 不管：QEMU 进程本身与命令通道（detail/qemu.rs）、镜像判定与挂载清单（domain/guest.rs）。
//! 联动：实现 ports::ShareBackend；由组合根构造、service.rs 持有；helper 是独立仓的预编译产物（见 dependencies.yaml 的 vm.smbd.windows）。

use crate::capabilities::vm::domain::exec::SMBD_BIN;
use crate::capabilities::vm::domain::guest::{GuestImage, GuestMount};
use crate::capabilities::vm::ports::{ShareBackend, ShareGuard, SharePlan};
use crate::kernel::detail::procs::{self, Entry, ProcessBook};
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// 目的：guest 看到的 SMB 服务地址——slirp 虚拟子网里的固定点；guestfwd 把它隧道到宿主 loopback 上的 helper 端口。
const GUEST_SMB_IP: &str = "10.0.2.100";
/// 目的：guest 侧 SMB 端口（mount -t cifs 与 guestfwd 规则都按它写）。
const GUEST_SMB_PORT: u16 = 445;

/// 目的：SMB 共享后端（QEMU 的 guestfwd + 用户态 smbd helper；Windows 宿主没有 9p / virtiofs 时的实时共享档）。
pub struct SmbShare {
    /// 长驻子进程的账（组合根注入；None = 不记账）。
    book: Option<std::sync::Arc<ProcessBook>>,
}

impl SmbShare {
    /// 目的：组合根专用。
    pub fn new() -> SmbShare {
        SmbShare { book: None }
    }

    /// 目的：接上长驻子进程的账：起一个 helper 登记一条，释放时注销。
    pub fn with_book(mut self, book: std::sync::Arc<ProcessBook>) -> SmbShare {
        self.book = Some(book);
        self
    }
}

/// 目的：探测 smbd helper——设置里登记的路径优先，其次看 PATH。
/// 返回：可执行文件路径；找不到给 None。
pub fn find_smbd(registered: Option<&str>) -> Option<PathBuf> {
    match registered.map(str::trim) {
        Some(p) if !p.is_empty() => {
            let path = PathBuf::from(p);
            if path.is_file() {
                Some(path)
            } else {
                None
            }
        }
        _ => crate::kernel::detail::host_probe::find_exe(SMBD_BIN),
    }
}

/// 目的：把挂载清单翻成共享计划——一条 guestfwd 隧道（helper 端口）+ 每个挂载点一条 mount -t cifs。
fn smb_plan(mounts: &[GuestMount], port: u16) -> SharePlan {
    let net_options = vec![format!(
        "guestfwd=tcp:{}:{}-tcp:127.0.0.1:{}",
        GUEST_SMB_IP, GUEST_SMB_PORT, port
    )];
    let mut guest_scripts = Vec::new();
    for (i, m) in mounts.iter().enumerate() {
        let ro = if m.read_only { ",ro" } else { "" };
        // 有界重试：SMB 握手在慢启动 / 嵌套虚拟化下会瞬时报 "server abruptly closed"；一次就判整台起不来
        // 会把瞬时失败放大成"虚拟机档不可用"。三次都失败必须**非零退出**（循环末尾的子命令不许吞掉失败）。
        guest_scripts.push(format!(
            "mkdir -p '{g}' && n=0 && ok=0 && while [ $n -lt 3 ]; do n=$((n+1)); if mount -t cifs //{ip}/s{i} '{g}' -o port={p},guest,soft,nounix,noserverino,uid=0,gid=0,file_mode=0666,dir_mode=0777{ro}; then ok=1; break; fi; sleep 2; done; if [ $ok -ne 1 ]; then printf '共享 %s 挂载失败（已尝试 %s 次）' '{g}' $n; exit 1; fi",
            g = m.guest,
            ip = GUEST_SMB_IP,
            i = i,
            p = GUEST_SMB_PORT,
            ro = ro
        ));
    }
    SharePlan {
        kind: "smb",
        engine_args: Vec::new(),
        net_options,
        guest_scripts,
        needs_net: true,
        needs_shared_mem: false,
    }
}

/// 目的：helper 的存活凭据——guest 租约期间持进程，释放即杀（幂等）。
struct SmbGuard {
    child: Option<Child>,
    /// 长驻子进程的账（有就在收尾时注销）。
    book: Option<std::sync::Arc<ProcessBook>>,
}

impl SmbGuard {
    fn cleanup(&mut self) {
        if let Some(mut c) = self.child.take() {
            if let Some(book) = &self.book {
                book.forget(c.id());
            }
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl ShareGuard for SmbGuard {
    fn release(&mut self) {
        self.cleanup();
    }
}

impl Drop for SmbGuard {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl ShareBackend for SmbShare {
    fn id(&self) -> &'static str {
        "smb"
    }

    /// 约束：只有 QEMU 的 user 网卡有 guestfwd；guest 侧要声明装了 cifs-utils（clients.cifs）。
    fn supports(&self, engine: &str, guest: &GuestImage) -> bool {
        crate::capabilities::vm::domain::exec::share_carries("smb", engine, guest)
    }

    /// 目的：起 helper（匿名共享，每个挂载点一个共享名 s0/s1…），产出 guestfwd 隧道参数与 guest 端 cifs 挂载脚本。
    /// 错误：helper 找不到 / 起不来 / 10 秒内没打出 LISTENING，都如实返回（并收掉半起的进程）。
    fn prepare(
        &self,
        _engine: &str,
        mounts: &[GuestMount],
        share_tool: Option<&str>,
    ) -> Result<(SharePlan, Box<dyn ShareGuard>), String> {
        if mounts.is_empty() {
            return Err("SMB 共享没有要挂的目录".to_string());
        }
        let exe = find_smbd(share_tool).ok_or_else(|| {
            format!(
                "没找到 {}（把它放进 PATH，或在设置里登记它的完整路径）",
                SMBD_BIN
            )
        })?;
        let mut cmd = Command::new(&exe);
        cmd.arg("--listen").arg("127.0.0.1:0");
        for (i, m) in mounts.iter().enumerate() {
            let ro = if m.read_only { ":ro" } else { "" };
            cmd.arg("--share")
                .arg(format!("s{i}={}{}", m.host.display(), ro));
        }
        // 父死子亡（Unix）：核心被强杀时 helper 不该继续占着共享目录。
        procs::guard(&mut cmd);
        // stderr 不丢：握手失败时现场（helper 自己说的话）是唯一能区分"谁关的连接"的证据。
        cmd.stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = procs::spawn(cmd)
            .map_err(|e| format!("起不了 SMB helper（{}）：{}", exe.display(), e))?;
        let launched = procs::attach(&child, "share:smb");
        if let Some(book) = &self.book {
            let first_mount = mounts
                .first()
                .map(|m| m.host.display().to_string())
                .unwrap_or_default();
            book.record(Entry {
                pid: child.id(),
                since: procs::now_ts(),
                purpose: "share:smb".to_string(),
                owner: first_mount.clone(),
                marker: format!("--share s0={}", first_mount),
                created: launched.created,
                guard_note: launched.guard_note,
            });
        }
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "SMB helper 没有 stdout".to_string())?;
        // helper 的诊断输出逐行转出来（带前缀，便于和 guest 侧 mount 报错对上时间）。
        if let Some(stderr) = child.stderr.take() {
            std::thread::spawn(move || {
                let reader = BufReader::new(stderr);
                for line in reader.lines().map_while(Result::ok) {
                    eprintln!("[虚拟机] smbd：{}", line);
                }
            });
        }
        // 有界等待 helper 打出第一行 LISTENING <addr>：它卡住时不能让我们干等。
        let (tx, rx) = mpsc::channel::<String>();
        std::thread::spawn(move || {
            let reader = BufReader::new(stdout);
            for line in reader.lines().map_while(Result::ok) {
                // 首行是 LISTENING <addr>；其余是 helper 自己的诊断，转出来，不静默丢。
                if line.starts_with("LISTENING ") {
                    if tx.send(line).is_err() {
                        break;
                    }
                } else {
                    eprintln!("[虚拟机] smbd：{}", line);
                }
            }
        });
        let first = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| {
            if let Some(book) = &self.book {
                book.forget(child.id());
            }
            let _ = child.kill();
            "等 SMB helper 的 LISTENING 超时（helper 自己的输出见上方 [虚拟机] smbd 行）"
                .to_string()
        })?;
        let addr = first
            .strip_prefix("LISTENING ")
            .map(str::trim)
            .filter(|a| !a.is_empty())
            .ok_or_else(|| format!("SMB helper 首行不是 LISTENING：{}", first))?;
        let port = addr
            .rsplit(':')
            .next()
            .and_then(|p| p.parse::<u16>().ok())
            .ok_or_else(|| format!("SMB helper 给的地址不合法：{}", addr))?;

        Ok((
            smb_plan(mounts, port),
            Box::new(SmbGuard {
                child: Some(child),
                book: self.book.clone(),
            }),
        ))
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    fn mount(guest: &str, host: &str, read_only: bool) -> GuestMount {
        GuestMount {
            host: std::path::PathBuf::from(host),
            guest: guest.to_string(),
            read_only,
        }
    }

    /// 目的：一条 guestfwd 隧道 + 每个挂载点一条 cifs 挂载；只读点带 ro。
    /// 约束：挂载必须有界重试，且三次失败要**非零退出**（循环末尾的子命令不许把失败吞成成功）。
    #[test]
    fn plan_builds_guestfwd_and_cifs_mounts() {
        let plan = smb_plan(
            &[mount("/a", "C:/host", false), mount("/b", "C:/ro", true)],
            45678,
        );
        assert_eq!(plan.kind, "smb");
        assert!(plan.needs_net && !plan.needs_shared_mem);
        assert_eq!(
            plan.net_options,
            vec!["guestfwd=tcp:10.0.2.100:445-tcp:127.0.0.1:45678".to_string()]
        );
        let script = &plan.guest_scripts[0];
        assert!(
            script.contains("//10.0.2.100/s0 '/a'") && script.contains("port=445"),
            "{:?}",
            script
        );
        assert!(
            !script.contains("dir_mode=0777,ro"),
            "可写挂载不该带 ro：{:?}",
            script
        );
        assert!(
            script.contains("while [ $n -lt 3 ]") && script.contains("sleep 2"),
            "挂载要有界重试：{:?}",
            script
        );
        assert!(
            script.contains("if [ $ok -ne 1 ]") && script.contains("exit 1"),
            "三次都失败要非零退出，不能被循环吞掉：{:?}",
            script
        );
        let ro_script = &plan.guest_scripts[1];
        assert!(
            ro_script.contains("//10.0.2.100/s1 '/b'") && ro_script.contains("dir_mode=0777,ro;"),
            "只读点要把 ro 写进 mount 选项：{:?}",
            ro_script
        );
    }

    /// 目的：只落在 QEMU 且 guest 声明了 cifs 客户端；换引擎或缺客户端都不落。
    #[test]
    fn supports_only_qemu_with_cifs_client() {
        let s = SmbShare::new();
        let mut guest = GuestImage::linux("t", std::path::PathBuf::from("/x"), None, None);
        guest.clients.cifs = true;
        assert!(s.supports("qemu", &guest));
        assert!(!s.supports("vz", &guest), "smb 只落在 qemu 上");
        guest.clients.cifs = false;
        assert!(!s.supports("qemu", &guest), "没声明 cifs 客户端就不落");
    }

    /// 目的：helper 不在场时如实拒绝（登记路径优先，登记了就必须是那个文件）。
    fn expect_err(r: Result<(SharePlan, Box<dyn ShareGuard>), String>) -> String {
        match r {
            Ok(_) => panic!("要拒"),
            Err(e) => e,
        }
    }

    #[test]
    fn prepare_refuses_when_helper_is_missing() {
        let s = SmbShare::new();
        let err = expect_err(s.prepare(
            "qemu",
            &[mount("/a", "C:/host", false)],
            Some("/nowhere/smbd"),
        ));
        assert!(err.contains("没找到"), "{}", err);
        let err = expect_err(s.prepare("qemu", &[], Some("/nowhere/smbd")));
        assert!(err.contains("没有要挂的目录"), "{}", err);
    }
}
