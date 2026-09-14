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
    let d = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join("test-scratch").join(name);
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("建探针目录");
    d
}

/// 手写 FenceSpec 的 JSON（字段与 core/fence.rs 一致：agent / rw / cwd / net）。
pub fn spec_json(rw: &[PathBuf], cwd: &PathBuf) -> String {
    let esc = |p: &PathBuf| p.to_string_lossy().replace('\\', "/");
    let roots: Vec<String> = rw.iter().map(|p| format!("\"{}\"", esc(p))).collect();
    format!("{{\"agent\":\"probe\",\"rw\":[{}],\"cwd\":\"{}\",\"net\":false}}", roots.join(","), esc(cwd))
}

/// 按运行期同一套逻辑起守门进程（Unix 独立进程组；Windows 靠它自己的 Job Object）。
pub fn spawn_fenced(spec: &str, command: &str) -> Child {
    let mut cmd = Command::new(bin());
    cmd.arg("--fence-run").arg(spec).arg("--").arg(command);
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

/// 跑一次守门进程，拿回（退出码, stdout, stderr）。
pub fn run_launcher(spec: &str, command: &str) -> (Option<i32>, String, String) {
    let out = Command::new(bin())
        .arg("--fence-run")
        .arg(spec)
        .arg("--")
        .arg(command)
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
