//! 守门进程协议与环境白名单（跨平台）：退出码如实回传、围栏装不上如实报错、超时连根杀掉整棵树。

use crate::probe::{bin, job_json, kill_like_runtime, python_command, run_launcher, scratch, spawn_fenced, wait, FENCE_FAILED};
use std::process::Command;
use std::time::{Duration, Instant};

#[test]
fn fence_failure_is_reported_honestly() {
    // 围栏参数非法 = 守门进程如实报错并退出 FENCE_FAILED，绝不无围栏地跑命令。
    let dir = scratch("bad-spec");
    let out = Command::new(bin())
        .arg("--fence-run")
        .arg("这不是 JSON")
        .arg("--")
        .arg("echo 不该被执行")
        .output()
        .expect("跑守门进程");
    assert_eq!(out.status.code(), Some(FENCE_FAILED), "围栏装不上必须如实失败");
    assert!(String::from_utf8_lossy(&out.stderr).contains("[围栏]"), "要在 stderr 说明原因");
    assert!(!String::from_utf8_lossy(&out.stdout).contains("不该被执行"), "命令不得执行");
    let _ = dir;
}

#[test]
fn launcher_runs_the_command_and_passes_its_exit_code() {
    // prepared = false = 未授权机器上的真实路径：守门进程不装容器，按平台围栏如实降级跑命令。
    let dir = scratch("ok");
    let (code, out, err) = run_launcher(&job_json(&[dir.clone()], &dir, false), "echo hello-from-tool");
    assert_eq!(code, Some(0), "退出码要如实回传：{} / {}", out, err);
    assert!(out.contains("hello-from-tool"), "{}", out);
}

#[test]
fn killing_the_guarded_process_takes_the_whole_tree_with_it() {
    // 工具里 fork 出来的孙进程必须跟着死（不留孤儿）：让孙进程 5 秒后写标记，1.5 秒就杀，
    // 之后等够了再断言标记不存在——存在即说明有孤儿活下来了。
    let dir = scratch("tree");
    let late = dir.join("late.txt");
    let path = late.to_string_lossy().replace('\\', "/");
    // 孙进程：5 秒后写标记，然后父进程长睡——被围栏连根杀掉时它不该活下来。
    #[cfg(windows)]
    let grandchild = format!("cmd /C ping -n 6 127.0.0.1 > nul & echo late> {}", path);
    #[cfg(not(windows))]
    let grandchild = format!("sleep 5; echo late > {}", path);
    #[cfg(windows)]
    let spawn = format!("import subprocess,time;subprocess.Popen(['cmd','/C','{}']);time.sleep(30)", grandchild);
    #[cfg(not(windows))]
    let spawn = format!("import subprocess,time;subprocess.Popen(['sh','-c','{}']);time.sleep(30)", grandchild);
    let script = spawn;
    let command = match python_command(&script) {
        Some(c) => c,
        None => {
            eprintln!("[探针] 本机没有 python，跳过进程树围栏探针（不静默当作通过）");
            return;
        }
    };
    let mut child = spawn_fenced(&job_json(&[dir.clone()], &dir, false), &command);
    wait(Duration::from_millis(1500));
    kill_like_runtime(&mut child);
    let deadline = Instant::now() + Duration::from_secs(7);
    while Instant::now() < deadline {
        if late.exists() {
            panic!("超时后仍有孤儿进程活着（孙进程写下了 {}）", late.display());
        }
        wait(Duration::from_millis(250));
    }
}
