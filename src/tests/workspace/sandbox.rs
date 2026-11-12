//! 目的：沙箱寻址与主副本只读——路径必须落在三个允许根内。
//! 管：resolve 的根校验、主副本对 agent 只读的两处收口、per-work 写闸、guest 写法译回宿主。
//! 不管：共享区版本化工作流（`work.rs`）。
//! 联动：`src/capabilities/workspace/domain/workspace.rs`。

use super::super::prelude::*;

#[test]
pub(crate) fn sandbox_resolve_accepts_only_absolute_paths_inside_roots() {
    use crate::capabilities::workspace::domain::workspace::Place;
    let sb = test_sandbox("a1", &["data"]);
    // 绝对且在根内 → 通过（返回归一化后的绝对路径）
    let (place, path) = sb
        .resolve(&p(&["demo", "work", "notes", "a.txt"]))
        .expect("共享区可达");
    assert_eq!(place, Place::Shared);
    assert_eq!(path, abs(&["demo", "work", "notes", "a.txt"]));
    let (place, path) = sb
        .resolve(&p(&["demo", "a1", "b.txt"]))
        .expect("私有沙箱可达");
    assert_eq!(place, Place::Private);
    assert_eq!(path, abs(&["demo", "a1", "b.txt"]));
    let (place, path) = sb
        .resolve(&p(&["mods", "data", "c.txt"]))
        .expect("成员模块目录可达");
    assert_eq!(place, Place::Module("data".to_string()));
    assert_eq!(path, abs(&["mods", "data", "c.txt"]));
    // 允许的根就是这三处：错误文案要把它们列全
    let roots_line = s(&["demo", "work"]);
    // 拒绝：越界 / 非绝对路径（相对、裸文件名、带冒号前缀的伪路径）/ .. 与 . 段 / 空段 / 空
    let bad: Vec<String> = vec![
        p(&["outside", "x.txt"]),
        "a.txt".to_string(),
        "demo/work/a.txt".to_string(),
        "work:/a.txt".to_string(),
        "sandbox:/b.txt".to_string(),
        "module:data:/c.txt".to_string(),
        format!(
            "{}{}..{}x.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!(
            "{}{}a{}..{}b.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!("{}//a.txt", p(&["demo", "work"])),
        "   ".to_string(),
    ];
    for b in &bad {
        let err = sb.resolve(b).unwrap_err();
        assert!(
            err.contains(&roots_line),
            "错误要把允许的真实根列全（{}）：{}",
            b,
            err
        );
    }
}

/// 目的：版本库写闸按**工作**分：不同工作互不阻塞，同一个工作严格串行。
#[test]
pub(crate) fn version_gate_is_per_work() {
    let ws = crate::capabilities::workspace::service::WorkspaceService::new(
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(InMemoryWorkStore::new()),
    );
    let a = ws.gate("a");
    let b = ws.gate("b");
    let _hold_a = a.lock().unwrap_or_else(|e| e.into_inner());
    assert!(b.try_lock().is_ok(), "不同工作互不阻塞");
    let a2 = ws.gate("a");
    assert!(a2.try_lock().is_err(), "同一个工作要串行");
}

/// 目的：work_status 也要过 per-work 写闸，不能与提交并发读到撕裂快照。
#[test]
pub(crate) fn work_status_waits_for_the_per_work_gate() {
    use crate::capabilities::workspace::api::Workspace;
    use std::sync::{Arc, mpsc};
    use std::time::Duration;
    let ws = Arc::new(
        crate::capabilities::workspace::service::WorkspaceService::new(
            Arc::new(InMemoryWorkspace::new()),
            Arc::new(InMemoryWorkStore::new()),
        ),
    );
    let gate = ws.gate("w");
    let guard = gate.lock().unwrap_or_else(|e| e.into_inner());
    let (started_tx, started_rx) = mpsc::channel();
    let (done_tx, done_rx) = mpsc::channel();
    let worker = {
        let ws = Arc::clone(&ws);
        std::thread::spawn(move || {
            started_tx.send(()).unwrap();
            let ok = ws.work_status("w", "a", &[]).is_ok();
            done_tx.send(ok).unwrap();
        })
    };
    started_rx.recv().unwrap();
    assert!(
        done_rx.recv_timeout(Duration::from_millis(500)).is_err(),
        "status 不能绕过写闸"
    );
    drop(guard);
    assert!(
        done_rx.recv_timeout(Duration::from_secs(5)).unwrap(),
        "释放后 status 要完成"
    );
    worker.join().unwrap();
}

/// 目的：虚拟机档下模型给的 guest 写法要能寻址——译成宿主路径**再过同一道根校验**；越界仍拒。
#[test]
pub(crate) fn sandbox_resolves_guest_written_paths() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::domain::workspace::Place;
    // 跨方言才有翻译（同方言时桥是恒等，走的是宿主写法那条路）。
    let guest = if cfg!(windows) {
        PathDialect::Posix
    } else {
        PathDialect::Windows
    };
    let bridge = PathBridge::new(std::env::consts::OS, guest);
    let mut sb = test_sandbox("a1", &["data"]);
    sb.path_codec = Some(Arc::new(bridge));
    // 私有沙箱在 guest 里的写法要认，并落回宿主私有沙箱。
    let guest_note = format!("{}/note.txt", bridge.to_guest(&sb.private));
    let (place, path) = sb.resolve(&guest_note).expect("guest 写法该认");
    assert_eq!(place, Place::Private);
    assert_eq!(path, sb.private.join("note.txt"));
    // 虚拟机档只认 guest 写法：宿主写法要明确拒绝，并说清是写法不对。
    let host_note = sb.private.join("host.txt").to_string_lossy().into_owned();
    let why = sb.resolve(&host_note).expect_err("宿主写法不再直通");
    assert!(why.contains("不是本环境的写法"), "{}", why);
    // 镜像根下、但不在任何允许根里的路径：翻译得出来也要被根校验拒。
    let outside = bridge.to_guest(&abs(&["elsewhere", "x.txt"]));
    let why = sb.resolve(&outside).expect_err("越界要拒");
    assert!(why.contains("系统工具允许的根"), "{}", why);
}
