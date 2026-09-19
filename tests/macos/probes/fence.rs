//! seatbelt 探针：真机验收本平台的文件系统围栏——允许的根里写得进，根之外读不到。
//! 驱动方式与运行期完全一致：交给守门进程（本程序 --fence-run）去装围栏。
//! 排障时打开 SOLOMNI_FENCE_PROFILE=1，让守门进程把完整 profile 打进 stderr。

use crate::probe::{job_json, run_launcher_env, scratch};

/// 与 src/adapters/confine/macos.rs 的 PROFILE_REJECTED_MARK 一致（集成测试看不到 crate 内部）。
/// 自检已确认本机 seatbelt 有效，却仍装不上 = 我们生成的 profile 写错了；
/// 那种情况**必须响亮失败**，否则「绿」等于围栏根本没验收。
const PROFILE_REJECTED_MARK: &str = "seatbelt profile 被拒绝";

#[test]
fn fence_denies_outside_paths_and_allows_the_given_roots() {
    // 排障开关不在运行期白名单里，由探针自己带给守门进程（探针的环境与运行期一致，见 probe.rs）。
    let profile: &[(&str, &str)] = &[("SOLOMNI_FENCE_PROFILE", "1")];
    let inside = scratch("fence-inside");
    let outside = scratch("fence-outside");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "SECRET-DO-NOT-LEAK").unwrap();
    // prepared = false：macOS 的 seatbelt 在守门进程里自足，没有"外层先授权"这一步。
    let spec = job_json(&[inside.clone()], &inside, false);

    // 允许的根里：写得进。
    let (code, out, err) = run_launcher_env(&spec, &format!("echo ok > {}", inside.join("x.txt").display()), profile);
    if err.contains(PROFILE_REJECTED_MARK) {
        // 非法操作名、语法错、转义错都会走到这里：是 profile 写错，不是环境不允许。
        panic!(
            "本机 seatbelt 机制有效，但生成的 profile 被 sandbox_init 拒绝（profile 写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        // 剩下的只能是「本机该私有 ABI 失效」这类环境结论：如实跳过并留痕（不算通过）。
        eprintln!(
            "[探针] 本机 seatbelt 不产生实际约束（环境结论，如实跳过，不作为通过）：{}",
            err.trim()
        );
        return;
    }
    if !inside.join("x.txt").exists() {
        // 只报「没写进去」太薄：再跑一条不碰文件系统的命令（shell 内建 true）做对照，
        // 用来分辨「连 shell 都起不来」（进程/加载器层）与「只有写被挡」（路径规则层）。
        let (tcode, tout, terr) = run_launcher_env(&spec, "true", profile);
        panic!(
            "允许的根里应当写得进：{} / {} / [对照] true → code={:?} out={} err={}",
            out, err, tcode, tout, terr
        );
    }
    assert_eq!(code, Some(0), "{} / {}", out, err);

    // 允许的根之外：同一个用户、同一台机器，只有围栏能挡住这一读。
    let (code, out, err) = run_launcher_env(&spec, &format!("cat {}", secret.display()), profile);
    assert!(
        !out.contains("SECRET-DO-NOT-LEAK"),
        "越界读必须拿不到：{} / {}",
        out,
        err
    );
    assert_ne!(code, Some(0), "越界读应以非零退出：{} / {}", out, err);
}
