//! 目的：Web 通道的启动：把围栏事实翻成呈现层要的结构，然后交给 `presentation::web::serve`。
//! 管：端口、围栏事实与异常退出码。
//! 不管：终端通道（`presentation::cli`）、核心装配（`entry/assembly`）。
//! 联动：`src/main.rs` 与 `entry/startup.rs`（`FenceReport`）、`presentation/web`。

pub(crate) fn serve_web(
    ops: crate::capabilities::conductor::api::Ops,
    port: u16,
    report: &crate::entry::startup::FenceReport,
    home: &std::path::Path,
) {
    // 与启动报告同一份事实：能力取本机，实际取授权后的路径级围栏。
    let fence = crate::presentation::web::FenceInfo {
        fs: report.cap.fs,
        net: report.cap.net,
        tree: report.cap.tree,
        note: report.cap.note.clone(),
        effective_fs: report.fs,
        effective_net: report.net,
        write_allowed: report.write_allowed,
        read_only_roots: ops
            .registry
            .settings()
            .map(|s| s.sandbox.policy.host_read_roots.len())
            .unwrap_or(0),
        sandbox_missing: report
            .sandbox_mode
            .missing()
            .into_iter()
            .map(|s| s.to_string())
            .collect(),
        sandbox_degraded: report.sandbox_mode.degraded(),
    };
    if let Err(e) = crate::presentation::web::serve(ops, port, fence, home) {
        eprintln!("[Web 服务异常] {}", e);
        std::process::exit(1);
    }
}
