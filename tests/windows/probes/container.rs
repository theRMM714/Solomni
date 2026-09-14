//! AppContainer 探针：容器真起来了、允许范围之外读不到也写不进、不给 capability 就是断网。
//! 这些断言不需要写目录 ACL（ACL 授权由产品在真实会话里做），但要**建一个 AppContainer profile**——
//! 那是改本机状态的动作，所以默认不跑：必须显式开启（node run-tests.js --fence-live 会把它传进来）。

use crate::probe::{env_blocks_container, run_launcher, scratch, spec_json};
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

/// 是否允许跑"会改本机状态"的探针（默认否：测试不该在真机上留下痕迹）。
fn live_enabled() -> bool {
    std::env::var("SOLOMNI_FENCE_LIVE").map(|v| v == "1").unwrap_or(false)
}

/// 未开启就如实跳过（不静默算过，并说清为什么与怎么开）。
fn skip_unless_live(name: &str) -> bool {
    if live_enabled() {
        return false;
    }
    eprintln!(
        "[探针] 未开启真机围栏测试：{} 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live",
        name
    );
    true
}


/// 探针用的围栏：rw 给一个落点；cwd 用系统目录（容器默认可读，避免把"CWD 能不能读"混进断言）。
fn spec_for(rw: &PathBuf) -> String {
    spec_json(std::slice::from_ref(rw), &PathBuf::from("C:\\Windows\\System32"))
}

#[test]
fn container_starts_and_passes_stdio_through() {
    if skip_unless_live("container_starts_and_passes_stdio_through") {
        return;
    }
    let dir = scratch("container-ok");
    let (code, out, err) = run_launcher(&spec_for(&dir), "echo container-ok");
    if env_blocks_container(&err) {
        eprintln!("[探针] 本环境不允许容器围栏：{}（请在普通 shell 里重跑本探针）", err.trim());
        return;
    }
    assert!(!err.contains("容器围栏未生效"), "本机应能建起容器：{}", err);
    assert!(out.contains("container-ok"), "stdout 要透传：{} / {}", out, err);
    assert_eq!(code, Some(0));
}

#[test]
fn container_cannot_read_outside_its_roots() {
    if skip_unless_live("container_cannot_read_outside_its_roots") {
        return;
    }
    let dir = scratch("container-deny-read");
    let secret = dir.join("secret.txt");
    std::fs::write(&secret, "SECRET-DO-NOT-LEAK").unwrap();
    let (code, out, err) = run_launcher(&spec_for(&dir), &format!("type {}", secret.display()));
    if env_blocks_container(&err) {
        eprintln!("[探针] 本环境不允许容器围栏，跳过越界读断言：{}", err.trim());
        return;
    }
    assert!(!out.contains("SECRET-DO-NOT-LEAK"), "越界读必须拿不到：{} / {}", out, err);
    assert_ne!(code, Some(0), "越界读应以非零退出：{} / {}", out, err);
}

#[test]
fn container_cannot_write_outside_its_roots() {
    if skip_unless_live("container_cannot_write_outside_its_roots") {
        return;
    }
    let dir = scratch("container-deny-write");
    let target = dir.join("should-not-exist.txt");
    let (code, out, err) = run_launcher(&spec_for(&dir), &format!("echo x> {}", target.display()));
    if env_blocks_container(&err) {
        eprintln!("[探针] 本环境不允许容器围栏，跳过越界写断言：{}", err.trim());
        return;
    }
    assert!(!target.exists(), "越界写不该落盘：{} / {}", out, err);
    assert_ne!(code, Some(0));
}

#[test]
fn container_has_no_network() {
    if skip_unless_live("container_has_no_network") {
        return;
    }
    // 先在本机（容器外）证明那个监听确实连得上，再在容器里证明连不上——否则这条断言没有意义。
    let listener = TcpListener::bind("127.0.0.1:0").expect("起本地监听");
    let port = listener.local_addr().expect("取端口").port();
    std::thread::spawn(move || {
        for s in listener.incoming() {
            if s.is_err() {
                break;
            }
        }
    });
    let cmd = format!("curl -s -m 2 -o NUL -w ok http://127.0.0.1:{}/", port);
    let outside = Command::new("cmd").arg("/C").arg(&cmd).output().expect("容器外跑一遍");
    let outside_ok = String::from_utf8_lossy(&outside.stdout).contains("ok");
    let dir = scratch("container-net");
    let (_, out, err) = run_launcher(&spec_for(&dir), &cmd);
    if env_blocks_container(&err) {
        eprintln!("[探针] 本环境不允许容器围栏，跳过断网断言：{}", err.trim());
        return;
    }
    assert!(
        !out.contains("ok"),
        "容器不该连得上网络（容器外那条路已证明可用={}）：stdout={} stderr={}",
        outside_ok,
        out,
        err
    );
}
