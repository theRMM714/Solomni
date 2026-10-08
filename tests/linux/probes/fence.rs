//! Landlock 探针：真机验收本平台的文件系统围栏——允许的根里写得进，根之外读不到。
//! 驱动方式与运行期完全一致：交给守门进程（本程序 --fence-run）去装围栏。

use crate::probe::{job_json, job_json_tree, run_launcher, scratch};

/// 与 src/capabilities/tools/detail/confine/linux.rs 的 RULES_REJECTED_MARK 一致（集成测试看不到 crate 内部）。
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
    let spec = job_json(std::slice::from_ref(&inside), &inside, false);

    // 允许的根里：写得进。
    let (code, out, err) = run_launcher(
        &spec,
        &format!("echo ok > {}", inside.join("x.txt").display()),
    );
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
    assert!(
        !out.contains("SECRET-DO-NOT-LEAK"),
        "越界读必须拿不到：{} / {}",
        out,
        err
    );
    assert_ne!(code, Some(0), "越界读应以非零退出：{} / {}", out, err);
    assert!(
        err.contains("范围外的访问被围栏拒绝"),
        "围栏内失败要带一条不分语言的边界说明：{}",
        err
    );
}

/// 未授权时段（`prepared = false`）的机制验证：把"本机不允许"与"我们写错了"分开。
/// 两者在守门进程里都表现为降级，只有 `--fence-verify` 能把它们区分开——
/// 分不开就会出现"用户以为有围栏、实际没有"（见 PRODUCT.md「隔离级别如实」）。
/// **三态都要认**：本机围栏装得上（真机 CI 就是这一态，断言要照它过）也走这一条；
/// 只有既不是装上了、也不是本机不允许的结论才该响亮失败（那才是"我们写错了"）。
#[test]
fn verify_separates_env_unavailable_from_broken_rules() {
    use crate::probe::{job_json, verdict_is_broken, verdict_is_env_unavailable, verify_fence};
    let dir = scratch("fence-verify");
    // 单元素切片用 from_ref：`&[dir.clone()]` 是多余的 clone（clippy 会点名）。
    let spec = job_json(std::slice::from_ref(&dir), &dir, false);
    let verdict = verify_fence(&spec, "true");
    if verdict_is_env_unavailable(&verdict) {
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过）：{}",
            verdict
        );
        return;
    }
    assert!(
        !verdict_is_broken(&verdict),
        "本机 Landlock 有效时规则就该装得上，装不上就是我们写错了：{}",
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
    if err.contains(RULES_REJECTED_MARK) {
        panic!(
            "本机 Landlock 机制有效，但规则装不上（规则写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过）：{}",
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

/// **共享区产品契约**：agent 的围栏只放行它自己的沙箱，共享主副本在 rw 之外——
/// 模块工具进程因此绕不过 work_pull / work_commit 直接读写共享区（工具层那一半由 workspace 的用例钉住）。
#[test]
fn shared_area_is_outside_the_agent_fence() {
    let sandbox = scratch("shared-area-sandbox");
    let shared = scratch("shared-area-work");
    let planted = shared.join("planted.txt");
    std::fs::write(&planted, "SHARED-CONTENT").unwrap();
    // 运行期 agent 的真实形态：rw = [沙箱]，共享区不在其中。
    let spec = job_json(std::slice::from_ref(&sandbox), &sandbox, false);

    // 沙箱里写得进（对照：不是整条围栏坏了）。
    let ok = sandbox.join("out.txt");
    let (code, _out, err) = run_launcher(&spec, &format!("echo ok > {}", ok.display()));
    if err.contains(RULES_REJECTED_MARK) {
        panic!(
            "本机 Landlock 机制有效，但规则装不上（规则写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过，不作为通过）：{}",
            err.trim()
        );
        return;
    }
    assert_eq!(code, Some(0), "沙箱要写得进：{}", err);
    assert!(ok.exists(), "沙箱要写得进：{}", err);

    // 共享区写不进、读不到：共享区不在 rw，围栏这一层就挡住了。
    let target = shared.join("pwn.txt");
    let (_wc, _wo, _we) = run_launcher(&spec, &format!("echo x > {}", target.display()));
    assert!(!target.exists(), "共享区不该被工具进程直接写");
    let (_rc, ro, _re) = run_launcher(&spec, &format!("cat {}", planted.display()));
    assert!(
        !ro.contains("SHARED-CONTENT"),
        "共享区不该被工具进程直接读：{}",
        ro
    );
}

/// **模块目录默认只读**（策略层把模块根放进 `ro_tree`、只把 `userdata/` 放进 `rw`）：
/// 模块代码写不进、目录读得到、`userdata/` 写得进。
#[test]
fn module_dir_is_read_only_but_userdata_is_writable() {
    let module = scratch("fence-module");
    let userdata = module.join("userdata");
    std::fs::create_dir_all(&userdata).unwrap();
    let script = module.join("script.py");
    std::fs::write(&script, "print(1)\n").unwrap();
    // 运行期形态：rw = [userdata]，ro_tree = [模块根]，cwd = 模块根。
    let spec = job_json_tree(
        std::slice::from_ref(&userdata),
        std::slice::from_ref(&module),
        &module,
        false,
    );
    let (_, _, err) = run_launcher(&spec, "true");
    if err.contains(RULES_REJECTED_MARK) {
        panic!(
            "本机 Landlock 机制有效，但规则装不上（规则写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过，不作为通过）：{}",
            err.trim()
        );
        return;
    }
    // 模块代码写不进。
    let (code, _out, werr) = run_launcher(&spec, &format!("echo x > {}", script.display()));
    assert_ne!(code, Some(0), "模块目录默认只读：{}", werr);
    // 模块目录读得到（ro_tree 递归可读的对照）。
    let (rc, rout, rerr) = run_launcher(&spec, &format!("cat {}", script.display()));
    assert!(
        rout.contains("print(1)"),
        "模块目录要读得到：code={:?} out={} err={}",
        rc,
        rout,
        rerr
    );
    // userdata 写得进（对照：不是整条围栏坏了）。
    let ok = userdata.join("state.json");
    let (ocode, _oout, oerr) = run_launcher(&spec, &format!("echo ok > {}", ok.display()));
    assert!(ok.exists(), "userdata 要写得进：{}", oerr);
    assert_eq!(ocode, Some(0), "{}", oerr);
}

/// **JS 工具能起**：Node 启动初始化 OpenSSL 会读系统 OpenSSL 配置（python / C++ 不读）——
/// 只读运行基线必须放行它，否则模块工具在脚本执行前就失败。无 node 时如实跳过。
#[test]
fn fence_allows_node_to_start_by_reading_the_openssl_config() {
    let has_node = std::process::Command::new("node")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if !has_node {
        eprintln!("[探针] 本机没有 node：跳过 JS 工具启动探针（不静默当作通过）");
        return;
    }
    let module = scratch("fence-node");
    std::fs::write(module.join("listdir.js"), "console.log('NODE-OK')\n").expect("写 node 脚本");
    // Node 会从入口向上找最近的 package.json：模块要自带这份作用域，别靠祖先目录恰好没有。
    std::fs::write(module.join("package.json"), r#"{ "type": "commonjs" }"#)
        .expect("写模块根 package.json");
    // 运行期形态：rw = [模块根]，ro_tree = [模块根]，cwd = 模块根（脚本就在里面）。
    let spec = job_json_tree(
        std::slice::from_ref(&module),
        std::slice::from_ref(&module),
        &module,
        false,
    );
    let (code, out, err) = run_launcher(&spec, "node listdir.js");
    if err.contains(RULES_REJECTED_MARK) {
        panic!(
            "本机 Landlock 机制有效，但规则装不上（规则写错，不是环境不允许）：{}",
            err.trim()
        );
    }
    if err.contains("文件系统围栏未生效") {
        eprintln!(
            "[探针] 本机 Landlock 不产生实际约束（环境结论，如实跳过，不作为通过）：{}",
            err.trim()
        );
        return;
    }
    assert_eq!(code, Some(0), "围栏里 node 要起得来：{} / {}", out, err);
    assert!(
        out.contains("NODE-OK"),
        "node 脚本的 stdout 要透传：{} / {}",
        out,
        err
    );
}
