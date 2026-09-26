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
    let spec = job_json(std::slice::from_ref(&inside), &inside, false);

    // 允许的根里：写得进。
    let (code, out, err) = run_launcher_env(
        &spec,
        &format!("echo ok > {}", inside.join("x.txt").display()),
        profile,
    );
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

/// 未授权时段（`prepared = false`）的机制验证：把"本机 ABI 失效"与"我们的 profile 写错"分开。
/// 与 `fence_denies_outside_paths_and_allows_the_given_roots` 同一把尺子，只是这里要的是**结论**，不是跑命令。
/// **三态都要认**：本机 seatbelt 装得上（真机 CI 就是这一态，断言要照它过）也走这一条；
/// 只有既不是装上了、也不是本机不允许的结论才该响亮失败（那才是"profile 写错"）。
#[test]
fn verify_separates_env_unavailable_from_a_rejected_profile() {
    use crate::probe::{job_json, verdict_is_broken, verdict_is_env_unavailable, verify_fence};
    let dir = scratch("fence-verify");
    // 单元素切片用 from_ref：`&[dir.clone()]` 是多余的 clone（clippy 会点名）。
    let spec = job_json(std::slice::from_ref(&dir), &dir, false);
    let verdict = verify_fence(&spec, "true");
    if verdict_is_env_unavailable(&verdict) {
        eprintln!(
            "[探针] 本机 seatbelt 不产生实际约束（环境结论，如实跳过）：{}",
            verdict
        );
        return;
    }
    assert!(
        !verdict_is_broken(&verdict),
        "本机 seatbelt 有效时 profile 就该装得上，装不上就是 profile 写错：{}",
        verdict
    );
}
/// 未授权时段 + **本机不允许**这一路（测试专用注入）：结论必须是 env-unavailable（不是 broken），
/// 而且守门进程要**照常执行命令**（如实降级，不是拒绝执行）——这条分支正常 runner 上碰不到。
#[test]
fn env_unavailable_degrades_and_still_runs_the_command() {
    use crate::probe::{
        degraded_by_env, job_json, run_launcher_env, verdict_is_broken, verdict_is_env_unavailable,
        verify_fence_with, SELFCHECK_FAIL_FLAG,
    };
    let dir = scratch("fence-env-unavailable");
    let spec = job_json(std::slice::from_ref(&dir), &dir, false);
    let injected: &[(&str, &str)] = &[(SELFCHECK_FAIL_FLAG, "1")];

    let verdict = verify_fence_with(&spec, "true", injected);
    assert!(
        verdict_is_env_unavailable(&verdict),
        "注入后必须报本机不允许：{}",
        verdict
    );
    assert!(
        !verdict_is_broken(&verdict),
        "本机不允许不是我们写错了：{}",
        verdict
    );

    // 降级照跑：命令真的执行了（写出文件），stderr 如实说明围栏未生效。
    let mark = dir.join("ran.txt");
    let (code, _out, err) =
        run_launcher_env(&spec, &format!("echo ok > {}", mark.display()), injected);
    assert_eq!(code, Some(0), "本机不允许时应降级照跑：{}", err);
    assert!(mark.exists(), "命令要真的执行了：{}", err);
    assert!(
        degraded_by_env(&err),
        "stderr 要如实说明围栏未生效：{}",
        err
    );
}
/// 只读档（用户显式授权的 `fence_read`）：授权的根**读得到、写不进**，而读写根照旧写得进。
/// 三件事分开断言，才能区分"只读位生效"与"整条围栏坏了"。
#[test]
fn read_only_roots_are_readable_but_not_writable() {
    use crate::probe::{job_json_ro, run_launcher, scratch};
    let inside = scratch("fence-ro-inside");
    let shared = scratch("fence-ro-shared");
    let secret = shared.join("data.txt");
    std::fs::write(&secret, "READ-ONLY-VISIBLE").unwrap();
    let spec = job_json_ro(
        std::slice::from_ref(&inside),
        std::slice::from_ref(&shared),
        &inside,
        false,
    );

    // ① 只读根读得到。
    let (code, out, err) = run_launcher(&spec, &format!("cat {}", secret.display()));
    if err.contains(PROFILE_REJECTED_MARK) {
        panic!(
            "本机 seatbelt 机制有效，但 profile 被拒（profile 写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        eprintln!(
            "[探针] 本机 seatbelt 不产生实际约束（环境结论，如实跳过）：{}",
            err.trim()
        );
        return;
    }
    assert!(
        out.contains("READ-ONLY-VISIBLE"),
        "只读根要读得到：{} / {}",
        out,
        err
    );
    assert_eq!(code, Some(0), "{} / {}", out, err);

    // ② 只读根写不进。
    let target = shared.join("should-not-exist.txt");
    let (wcode, _wout, _werr) = run_launcher(&spec, &format!("echo x > {}", target.display()));
    assert!(!target.exists(), "只读根不该写得进");
    assert_ne!(wcode, Some(0), "写只读根应以非零退出");

    // ③ 读写根照旧写得进（对照：证明不是整条围栏坏了）。
    let ok = inside.join("ok.txt");
    let (ocode, _oout, oerr) = run_launcher(&spec, &format!("echo ok > {}", ok.display()));
    assert!(ok.exists(), "读写根要写得进：{}", oerr);
    assert_eq!(ocode, Some(0), "{}", oerr);
}
