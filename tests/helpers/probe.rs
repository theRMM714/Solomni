//! 探针工具（被 tests/cross-platform 与 tests/<平台> 通过 #[path] 共用同一份源码）。
//! 刻意不依赖 crate 内部类型：直接按命令行协议驱动编出来的可执行文件，走与运行期完全同一条路径。
//! 每个测试目标只用到其中一部分，所以这里对未用到的部分放行（同一份源码服务多个目标）。
#![allow(dead_code)]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// 围栏装不上时的退出码（与 src/adapters/confine/mod.rs 的 FENCE_FAILED 一致）。
pub const FENCE_FAILED: i32 = 111;
/// "本环境不允许容器围栏"的标记（与 confine::windows::ENV_BLOCKED_MARK 一致）。
pub const ENV_BLOCKED_MARK: &str = "容器围栏不可用（本环境不允许";

/// 被测二进制（cargo 保证它在跑集成测试前已构建）。
pub fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_solomni"))
}

/// 每次用的干净落点（target/ 下，不入库）。
pub fn scratch(name: &str) -> PathBuf {
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("test-scratch")
        .join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("建探针目录");
    d
}

/// 手写守门进程的入参 JSON（字段与 core/fence.rs 的 FenceSpec 同层，外加一个 prepared）：
/// `prepared` = 外层是否已经把本机授权做完（只有 Windows 的容器围栏用得上）。
/// 验容器机制传 true（ACL 授权是产品在真实会话里做的，探针不写本机权限项）；验协议与降级路径传 false
/// ——那正是未授权机器上的真实路径。
pub fn job_json(rw: &[PathBuf], cwd: &PathBuf, prepared: bool) -> String {
    let esc = |p: &PathBuf| p.to_string_lossy().replace('\\', "/");
    let roots: Vec<String> = rw.iter().map(|p| format!("\"{}\"", esc(p))).collect();
    format!(
        "{{\"agent\":\"probe\",\"rw\":[{}],\"ro\":[],\"cwd\":\"{}\",\"net\":false,\"prepared\":{},\"home\":null}}",
        roots.join(","),
        esc(cwd),
        prepared
    )
}

/// 带**只读根**的守门进程入参（只读档的探针用）：`ro` 与 `rw` 同层，`prepared` 说明外层有没有做完本机授权。
/// 只读根的授权只在 Windows 上需要写 ACE；unix 的机制在守门进程内自足，所以 prepared 传什么都行。
pub fn job_json_ro(rw: &[PathBuf], ro: &[PathBuf], cwd: &PathBuf, prepared: bool) -> String {
    let esc = |p: &PathBuf| p.to_string_lossy().replace('\\', "/");
    let list = |ps: &[PathBuf]| -> String {
        ps.iter()
            .map(|p| format!("\"{}\"", esc(p)))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"agent\":\"probe\",\"rw\":[{}],\"ro\":[{}],\"cwd\":\"{}\",\"net\":false,\"prepared\":{},\"home\":null}}",
        list(rw),
        list(ro),
        esc(cwd),
        prepared
    )
}

/// 运行期交给工具进程的环境白名单：**问产品自己拿**（`--print-fence-env`），不在这里另抄一份。
/// 探针必须在**同一个环境**里驱动守门进程：环境不同，围栏的真实行为就不同（真机上已抓到过这种盲区）。
pub fn runtime_env(spec: &str) -> Vec<(String, String)> {
    let out = Command::new(bin())
        .arg("--print-fence-env")
        .arg(spec)
        .output()
        .expect("问产品要环境白名单");
    assert!(
        out.status.success(),
        "环境白名单要能问出来：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| {
            line.split_once('=')
                .map(|(k, v)| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// 起守门进程：先套运行期的环境（清空 + 白名单），再补调用方要的排障开关。
fn fenced_command(spec: &str, command: &str, extra: &[(&str, &str)]) -> Command {
    let mut cmd = Command::new(bin());
    cmd.arg("--fence-run").arg(spec).arg("--").arg(command);
    cmd.env_clear();
    for (k, v) in runtime_env(spec) {
        cmd.env(k, v);
    }
    for (k, v) in extra {
        cmd.env(k, v);
    }
    cmd
}

/// 按运行期同一套逻辑起守门进程（同样的环境；Unix 独立进程组；Windows 靠它自己的 Job Object）。
pub fn spawn_fenced(spec: &str, command: &str) -> Child {
    let mut cmd = fenced_command(spec, command, &[]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().expect("拉起守门进程")
}

/// 与运行期 kill_tree 同一套语义：Unix 杀整个进程组，Windows 杀守门进程（Job Object 连根收）。
pub fn kill_like_runtime(child: &mut Child) {
    #[cfg(unix)]
    unsafe {
        libc::kill(-(child.id() as i32), libc::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = child.kill();
    }
    let _ = child.wait();
}

/// 跑一次守门进程，拿回（退出码, stdout, stderr）——环境与运行期一致。
pub fn run_launcher(spec: &str, command: &str) -> (Option<i32>, String, String) {
    run_launcher_env(spec, command, &[])
}

/// 同上，并给**守门进程**补上调用方要的环境（例如 macOS 的 profile 排障开关：它不在白名单里，探针自己带）。
pub fn run_launcher_env(
    spec: &str,
    command: &str,
    extra: &[(&str, &str)],
) -> (Option<i32>, String, String) {
    let out = fenced_command(spec, command, extra)
        .output()
        .expect("跑守门进程");
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// 本环境是否不允许容器围栏（探针据此如实跳过——依据是环境结论，不是被测代码的成功与否）。
pub fn env_blocks_container(err: &str) -> bool {
    err.contains(ENV_BLOCKED_MARK)
}

/// 测试专用的自检注入开关（与 src/adapters/confine/mod.rs 的 SELFCHECK_FAIL_FLAG 一致）。
/// 打开它 = 让机制验证确定性地报「本机不允许」，用来覆盖那条正常 runner 上碰不到的分支。
pub const SELFCHECK_FAIL_FLAG: &str = "SOLOMNI_FENCE_SELFCHECK_FAIL";

/// 机制验证（机器可读）：**不装围栏、不写任何权限项**，只问"这次能不能强制住"。
/// 三态原话交回调用方（enforced / env-unavailable / broken）：探针据此决定 env-skip 还是失败。
pub fn verify_fence(spec: &str, command: &str) -> String {
    verify_fence_with(spec, command, &[])
}

/// 同上，并给守门进程补上调用方要的环境（例如测试专用的自检注入开关）。
pub fn verify_fence_with(spec: &str, command: &str, extra: &[(&str, &str)]) -> String {
    let mut cmd = Command::new(bin());
    cmd.arg("--fence-verify")
        .arg(spec)
        .arg("--")
        .arg(command)
        .env_clear()
        .envs(runtime_env(spec));
    for (k, v) in extra {
        cmd.env(k, v);
    }
    let out = cmd.output().expect("问产品要机制验证结论");
    assert!(
        out.status.success(),
        "机制验证要能跑完：{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

/// 守门进程（`--fence-run`）按「本机不允许」执行时的降级标记。
/// 与 src/adapters/confine/linux.rs / macos.rs / windows.rs 的原文一致——探针据此断言"确实降级了"。
pub fn degraded_by_env(err: &str) -> bool {
    err.contains("文件系统围栏未生效") || err.contains("容器围栏不可用")
}

/// 自检已确认机制有效却仍装不上 = 我们写错了（不是环境不允许）。
/// 与 src/adapters/confine/linux.rs 的 RULES_REJECTED_MARK、macos.rs 的 PROFILE_REJECTED_MARK 同义。
pub fn verdict_is_broken(verdict: &str) -> bool {
    verdict.starts_with("broken")
}

/// 本环境不允许装围栏（环境结论：如实跳过，而不是失败）。
pub fn verdict_is_env_unavailable(verdict: &str) -> bool {
    verdict.starts_with("env-unavailable")
}

/// 本机有没有能用的 python（没有就如实跳过需要它的探针）。
/// 平台差异如实处理：Linux / macOS 常见的是 python3，Windows 常见的是 python。
pub fn python_command(script: &str) -> Option<String> {
    for name in ["python", "python3"] {
        if let Ok(o) = Command::new(name).arg("-c").arg("print(1)").output() {
            if o.status.success() {
                return Some(format!("{} -c \"{}\"", name, script));
            }
        }
    }
    None
}

/// 等一会儿（探针里等子进程写文件用）。
pub fn wait(d: Duration) {
    std::thread::sleep(d);
}
