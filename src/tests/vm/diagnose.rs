//! 目的：诊断报告的形状与失败原因。
//! 管：解析形状、七类失败原因、只读系统允许集。
//! 不管：通道实现（`fence.rs`）。
//! 联动：`docs/vm/README.md`。

use crate::capabilities::vm::api::Vm;
use crate::capabilities::vm::domain::guest::GuestSpec;

/// 目的：helper 诊断模式输出的**真形状**（键与 guest_helper 的 collect_diagnosis 一致）——解析与 T2 共用。
fn sample_diagnosis_json() -> String {
    serde_json::json!({
        "uptime_secs": 500.0,
        "loadavg": [0.5, 0.4, 0.3],
        "process_total": 2,
        "truncated": true,
        "processes": [
            {"pid": 7, "ppid": 1, "state": "R", "cpu_percent": 96.5, "rss_kb": 4096, "etime_secs": 120, "comm": "busy", "args": "sh -c while :; do :; done"},
            {"pid": 1, "ppid": 0, "state": "S", "cpu_percent": 0.1, "rss_kb": 2048, "etime_secs": 499, "comm": "init", "args": "init --user"}
        ],
        "memory": {"total_kb": 2048000, "available_kb": 1000000, "free_kb": 100000, "buffers_kb": 1000, "cached_kb": 200000, "swap_total_kb": 0, "swap_free_kb": 0, "used_percent": 51.2},
        "cpu": {"user": 100, "nice": 0, "system": 200, "idle": 3000, "iowait": 10, "irq": 0, "softirq": 0, "steal": 0, "idle_percent": 90.6},
        "mounts": [{"source": "/dev/vda", "target": "/", "fstype": "ext4", "total_kb": 2048000, "free_kb": 1024000}],
        "guest": {"kernel": "6.1.0-solomni-test", "nproc": 2}
    })
    .to_string()
}

/// 目的：诊断报告的形状与失败原因——合法 JSON 解析、被截断 / 缺字段 / 垃圾 JSON 一律 Err，七类原因各不相同。
#[test]
pub(crate) fn guest_diagnosis_parses_report_and_reasons() {
    use crate::capabilities::vm::domain::diagnose;
    let text = sample_diagnosis_json();
    let report = diagnose::parse_report(&text, "w-diag").expect("合法 JSON 要能解析");
    assert_eq!(report.lease, "w-diag", "租约由宿主补，不从 guest 取");
    assert_eq!(report.process_total, 2);
    assert!(report.truncated, "helper 说裁剪过就要如实带出来");
    assert_eq!(report.processes.len(), 2);
    assert_eq!(report.processes[0].pid, 7);
    assert!(
        report.processes[0].cpu_percent > 90.0,
        "忙进程的 CPU 占用要保留"
    );
    assert_eq!(report.processes[1].comm, "init");
    assert_eq!(report.memory.total_kb, 2048000);
    assert!(report.memory.used_percent > 0.0);
    assert!(report.cpu.idle_percent > 90.0);
    assert_eq!(report.mounts[0].target, "/");
    assert_eq!(report.guest.nproc, 2);
    assert!(report.loadavg[0] > 0.0 && report.uptime_secs > 0.0);

    // 被截断 / 垃圾 / 缺字段：都是"输出不可解析"，绝不是一份空诊断。
    let cut: String = text.chars().take(text.chars().count() / 2).collect();
    for bad in [cut.as_str(), "not json", r#"{"uptime_secs": 1.0}"#] {
        let err = diagnose::parse_report(bad, "w-diag").expect_err("非法输出必须报错");
        assert!(err.contains("输出不可解析"), "{}", err);
    }

    // 七类失败文案：各自能看出是什么原因，且互不重复（调用方据此分别报）。
    let texts = [
        diagnose::no_live_guest("w"),
        diagnose::not_linux("windows"),
        diagnose::channel_down("读超时"),
        diagnose::helper_unavailable("/usr/local/sbin 不在"),
        diagnose::python_missing("command -v 失败"),
        diagnose::timed_out(),
        diagnose::unparseable("EOF"),
    ];
    let hints = [
        "没有活着的 guest",
        "只有 Linux",
        "QGA 通道不通",
        "助手不在",
        "缺 python3",
        "超时",
        "不可解析",
    ];
    let mut seen = std::collections::BTreeSet::new();
    for (text, hint) in texts.iter().zip(hints.iter()) {
        assert!(text.contains(hint), "{} 要能看出是 {}", text, hint);
        assert!(
            seen.insert(text.clone()),
            "失败文案不能混成同一句：{}",
            text
        );
    }
}

/// 目的：诊断经 VmService 走**只读系统允许集**（不是 per-agent 围栏）；失败原因按事实区分。
#[test]
pub(crate) fn guest_diagnose_uses_read_only_system_access_and_reports_reasons() {
    use crate::capabilities::vm::api::GuestImage;
    use crate::capabilities::vm::domain::guest::GuestOs;
    use crate::capabilities::vm::service::VmService;
    use crate::tests::doubles::{FakeEngine, NamedShare};
    use std::sync::Arc;

    let engine = Arc::new(FakeEngine {
        fenced_output: Some(sample_diagnosis_json()),
        ..Default::default()
    });
    let vm = VmService::new(engine.clone(), vec![Arc::new(NamedShare::new("9p", true))]);
    let image = GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None);
    let spec = GuestSpec {
        lease: "w-diag".to_string(),
        image,
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    vm.guest_start(&spec).expect("替身 guest 要能起来");
    let report = vm.guest_diagnose("w-diag").expect("预设 JSON 要能解析");
    assert_eq!(report.lease, "w-diag");
    assert_eq!(report.processes[0].pid, 7);

    // 走的是围栏通道的**只读系统允许集**：rw / cwd / private 全空，ro_tree 是诊断那四个只看根，diagnose=true。
    let calls = engine.calls.lock().expect("锁").clone();
    let fenced = calls
        .iter()
        .find(|c| c.starts_with("fenced:w-diag:guest-diagnose:"))
        .expect("诊断必须走围栏通道（helper 诊断模式）");
    assert_eq!(
        fenced, "fenced:w-diag:guest-diagnose:::/proc,/sys,/dev,/etc:true",
        "只读系统允许集不能带 rw / cwd / private，且必须标记 diagnose"
    );

    // 没有活 guest：如实说"没有活着的 guest"（与"没有进程"分开）。
    let gone = vm.guest_diagnose("nope").expect_err("没有活 guest 要拒绝");
    assert!(gone.contains("没有活着的 guest（租约 nope）"), "{}", gone);

    // 非 Linux guest：本期没有 helper 诊断模式，如实拒绝且不去打通道。
    let mut win = GuestImage::linux(
        "win",
        std::path::PathBuf::from("/img/win.qcow2"),
        None,
        None,
    );
    win.os = GuestOs::Windows;
    let win_spec = GuestSpec {
        lease: "w-win".to_string(),
        image: win,
        ..spec.clone()
    };
    vm.guest_start(&win_spec).expect("替身不挑 guest 的 OS");
    let win_err = vm
        .guest_diagnose("w-win")
        .expect_err("非 Linux guest 要拒绝");
    assert!(win_err.contains("只有 Linux guest 支持诊断"), "{}", win_err);
    assert!(
        !engine
            .calls
            .lock()
            .expect("锁")
            .iter()
            .any(|c| c.starts_with("fenced:w-win:")),
        "非 Linux guest 不该去调通道"
    );
}

/// 目的：诊断失败按事实分流：超时 / helper 自报错 / 助手不在 / 缺 python3 / 通道不通，各有各的说法。
#[test]
pub(crate) fn guest_diagnose_separates_failure_causes() {
    use crate::capabilities::vm::api::GuestImage;
    use crate::capabilities::vm::domain::diagnose as diag;
    use crate::capabilities::vm::service::VmService;
    use crate::kernel::api::ToolOutcome;
    use crate::tests::doubles::{FakeEngine, NamedShare};
    use std::sync::Arc;

    let spec = GuestSpec {
        lease: "w-fail".to_string(),
        image: GuestImage::linux("t", std::path::PathBuf::from("/img/root.qcow2"), None, None),
        mounts: Vec::new(),
        net: false,
        engine_path: None,
        share_path: None,
        shares: Vec::new(),
        seed: None,
        isolated: true,
        cache_disk: None,
    };
    let started = |fenced: Option<ToolOutcome>, probe: Option<ToolOutcome>| {
        let engine = Arc::new(FakeEngine {
            fenced_result: fenced,
            exec_result: probe,
            ..Default::default()
        });
        let vm = VmService::new(engine, vec![Arc::new(NamedShare::new("9p", true))]);
        vm.guest_start(&spec).expect("替身 guest 要能起来");
        vm
    };
    let fail = |output: &str| ToolOutcome {
        ok: false,
        output: output.to_string(),
    };
    let say = |output: &str| ToolOutcome {
        ok: true,
        output: output.to_string(),
    };

    let err = started(
        Some(fail(&format!(
            "helper 挂了 {}",
            diag::HELPER_TIMEOUT_MARKER
        ))),
        None,
    )
    .guest_diagnose("w-fail")
    .expect_err("超时要拒绝");
    assert!(err.contains("超时"), "{}", err);

    let err = started(
        Some(fail(&format!("{}自检失败", diag::HELPER_DIAGNOSE_PREFIX))),
        None,
    )
    .guest_diagnose("w-fail")
    .expect_err("helper 自报错要拒绝");
    assert!(err.contains("诊断执行失败"), "{}", err);

    let err = started(Some(fail("没有输出")), Some(say(diag::PROBE_NO_HELPER)))
        .guest_diagnose("w-fail")
        .expect_err("助手不在要拒绝");
    assert!(err.contains("诊断助手不在"), "{}", err);

    let err = started(Some(fail("没有输出")), Some(say(diag::PROBE_NO_PYTHON)))
        .guest_diagnose("w-fail")
        .expect_err("缺 python3 要拒绝");
    assert!(err.contains("缺 python3"), "{}", err);

    let err = started(Some(fail("预检都说在")), Some(say(diag::PROBE_OK)))
        .guest_diagnose("w-fail")
        .expect_err("预检正常但诊断失败要拒绝");
    assert!(err.contains("诊断执行失败"), "{}", err);

    let err = started(Some(fail("没有输出")), Some(fail("")))
        .guest_diagnose("w-fail")
        .expect_err("预检没有回应要拒绝");
    assert!(err.contains("通道不通"), "{}", err);
    assert!(err.contains("没有回应诊断预检"), "{}", err);

    let err = started(Some(say("不是 JSON")), None)
        .guest_diagnose("w-fail")
        .expect_err("输出不含报告要拒绝");
    assert!(err.contains("诊断助手不在"), "{}", err);

    let command = diag::probe_command();
    assert!(command.contains(diag::HELPER_PATH), "{}", command);
    assert!(command.contains("timeout 10"), "{}", command);
    assert!(command.contains(diag::PROBE_NO_HELPER), "{}", command);
    assert!(command.contains(diag::PROBE_NO_PYTHON), "{}", command);
    assert!(command.contains(diag::PROBE_OK), "{}", command);
}
