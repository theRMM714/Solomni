//! Landlock 探针：真机验收本平台的文件系统围栏——允许的根里写得进，根之外读不到。
//! 驱动方式与运行期完全一致：交给守门进程（本程序 --fence-run）去装围栏。

use crate::probe::{job_json, run_launcher, scratch};

/// 与 src/adapters/confine/linux.rs 的 RULES_REJECTED_MARK 一致（集成测试看不到 crate 内部）。
/// 自检已确认本机 Landlock 有效，却仍装不上 = 我们的规则写错了；
/// 那种情况**必须响亮失败**，否则「绿」等于围栏根本没验收。
const RULES_REJECTED_MARK: &str = "landlock 规则被拒绝";

#[test]
fn fence_denies_outside_paths_and_allows_the_given_roots() {
    let inside = scratch("fence-inside");
    let outside = scratch("fence-outside");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "SECRET-DO-NOT-LEAK").unwrap();
    // prepared = false：Linux 的 Landlock 在守门进程里自足，没有"外层先授权"这一步。
    let spec = job_json(&[inside.clone()], &inside, false);

    // 允许的根里：写得进。
    let (code, out, err) = run_launcher(&spec, &format!("echo ok > {}", inside.join("x.txt").display()));
    if err.contains(RULES_REJECTED_MARK) {
        // 掩码/路径写错都会走到这里：是规则写错，不是环境不允许。
        panic!(
            "本机 Landlock 机制有效，但规则装不上（规则写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        // 剩下的只能是「本机内核不能围栏」这类环境结论：如实跳过并留痕（不算通过）。
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过，不作为通过）：{}",
            err.trim()
        );
        return;
    }
    if !inside.join("x.txt").exists() {
        // 只报「没写进去」太薄：再跑一条不碰文件系统的命令（shell 内建 true）做对照，
        // 用来分辨「连 shell 都起不来」（规则太紧）与「只有写被挡」（路径规则层）。
        let (tcode, tout, terr) = run_launcher(&spec, "true");
        panic!(
            "允许的根里应当写得进：{} / {} / [对照] true → code={:?} out={} err={}",
            out, err, tcode, tout, terr
        );
    }
    assert_eq!(code, Some(0), "{} / {}", out, err);

    // 允许的根之外：同一个用户、同一台机器，只有围栏能挡住这一读。
    let (code, out, err) = run_launcher(&spec, &format!("cat {}", secret.display()));
    assert!(!out.contains("SECRET-DO-NOT-LEAK"), "越界读必须拿不到：{} / {}", out, err);
    assert_ne!(code, Some(0), "越界读应以非零退出：{} / {}", out, err);
}

/// 未授权时段（`prepared = false`）的机制验证：把"环境不允许"与"我们写错了"分开。
/// 两者在守门进程里都表现为降级，只有 `--fence-verify` 能把它们区分开——
/// 分不开就会出现"用户以为有围栏、实际没有"（见 PRODUCT.md「隔离级别如实」）。
#[test]
fn verify_separates_env_unavailable_from_broken_rules() {
    use crate::probe::{job_json, verdict_is_broken, verdict_is_env_unavailable, verify_fence};
    let dir = scratch("fence-verify");
    let spec = job_json(&[dir.clone()], &dir, false);
    let verdict = verify_fence(&spec, "true");
    if verdict_is_env_unavailable(&verdict) {
        eprintln!("[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过）：{}", verdict);
        return;
    }
    assert!(verdict_is_broken(&verdict), "本机 Landlock 有效时规则就该装得上：{}", verdict);
}
