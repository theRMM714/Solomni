//! 目的：进程适配器 的测试：适配器的真实契约（真起进程、真收输出、真判退出码）。
//! 管：只覆盖本模块的公开契约与平台分支；夹具在本文件内。
//! 不管：上层调用方的装配（在 `src/tests/`）、其它适配器。
//! 联动：与 `src/kernel/detail/process.rs` 同模块，`#[cfg(test)] mod tests;` 指到这里。

use super::*;
use crate::kernel::api::FenceSpec;
// 测试夹具按 &Path 收参（clippy 的 ptr_arg）：Path 显式写在测试模块里，
// 顶层只按需导入 PathBuf——否则顶层会多出一次"只被 glob 用到"的导入。
use std::path::{Path, PathBuf};

/// 环境白名单：父进程的无关变量（密钥之类）不进子进程；HOME / TEMP 落在该 agent 的私有沙箱里。
#[test]
fn env_whitelist_drops_foreign_vars_and_moves_home_into_the_sandbox() {
    // SAFETY: 只改本用例独占的探针变量，用例内即改即清。
    unsafe {
        std::env::set_var("SOLOMNI_PROBE_SECRET", "leak-me");
    }
    let private = PathBuf::from("demo").join("agent-a");
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![PathBuf::from("demo").join("work"), private.clone()],
        cwd: PathBuf::from("mods").join("m0"),
        ro: Vec::new(),
        net: false,
    };
    let env: Vec<(String, String)> =
        crate::kernel::detail::confine::fence_env(&spec, "python tools/x.py")
            .into_iter()
            .map(|(k, v)| {
                (
                    k.to_string_lossy().into_owned(),
                    v.to_string_lossy().into_owned(),
                )
            })
            .collect();
    let get = |key: &str| env.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone());
    assert!(
        !env.iter().any(|(_, v)| v.contains("leak-me")),
        "无关变量不得进子进程"
    );
    assert!(get("SOLOMNI_PROBE_SECRET").is_none());
    assert_eq!(
        get("HOME").as_deref(),
        Some(private.to_string_lossy().as_ref()),
        "HOME 落在私有沙箱"
    );
    assert_eq!(
        get("TEMP").as_deref(),
        Some(private.to_string_lossy().as_ref())
    );
    // Windows 建 AppContainer 进程要读 LOCALAPPDATA：白名单里没有它，CreateProcessW 直接失败（os error 203），
    // 容器整条路会静默降级成无围栏——落户同样指进私有沙箱。
    assert_eq!(
        get("LOCALAPPDATA").as_deref(),
        Some(private.to_string_lossy().as_ref()),
        "LOCALAPPDATA 也落在私有沙箱"
    );
    assert_eq!(
        get("PYTHONIOENCODING").as_deref(),
        Some("utf-8"),
        "编码统一 UTF-8"
    );
    // SAFETY: 同上。
    unsafe {
        std::env::remove_var("SOLOMNI_PROBE_SECRET");
    }
}

/// 回执里的标记文案由进程机制自己的代码返回（它们随 [工具结果] 进模型上下文）。
#[test]
fn receipt_markers_are_code_owned() {
    let texts = CodeProcessTexts;
    let tools = ProcTools::new(
        PathBuf::from("solomni"),
        std::sync::Arc::new(CodeProcessTexts),
        PathBuf::from("target").join("test-scratch"),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );

    let ok = tools.assemble("正常输出".to_string(), String::new(), false, Some(0));
    assert!(ok.ok);
    assert_eq!(ok.output, "正常输出", "没有异常就不加任何标记");

    let with_err = tools.assemble("正文".to_string(), "警告".to_string(), false, Some(0));
    assert!(with_err.output.contains(&texts.stderr_header()) && with_err.output.contains("警告"));

    let timed = tools.assemble(String::new(), String::new(), true, None);
    assert!(!timed.ok);
    assert!(timed.output.contains(&texts.timeout()), "{}", timed.output);

    let fenced = tools.assemble(
        String::new(),
        String::new(),
        false,
        Some(confine::FENCE_FAILED),
    );
    assert!(!fenced.ok);
    assert!(
        fenced.output.contains(&texts.fence_failed()),
        "{}",
        fenced.output
    );

    let long = "字".repeat(20_000);
    let cut = tools.assemble(long, String::new(), false, Some(0));
    assert!(
        cut.output.contains("20000"),
        "截断要如实报字符数：{}",
        &cut.output[cut.output.len() - 80..]
    );
}

/// 全局沙箱要求：机制不可用且未同意降级时拒绝；同意降级后放行；"允许联网"不需要网络围栏。
#[test]
fn sandbox_mode_refuses_missing_requirements_until_degraded() {
    use crate::kernel::api::{SandboxMode, SandboxRequirements};
    let spec = FenceSpec {
        agent: "a".to_string(),
        lease: String::new(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::new(),
        cwd: PathBuf::new(),
        net: false,
    };
    let mode = SandboxMode::new(
        SandboxRequirements {
            filesystem: true,
            network: true,
            process_tree: true,
            env_whitelist: true,
        },
        false,
        false,
        false,
        false,
        true,
    );
    assert!(mode.check(&spec, false).is_err(), "缺机制时默认拒绝");
    mode.set_degraded(true);
    assert!(mode.check(&spec, false).is_ok(), "本进程同意降级后放行");
    mode.set_degraded(false);
    let allow_net = FenceSpec {
        net: true,
        ro: Vec::new(),
        ..spec.clone()
    };
    // 文件系统仍然缺，所以还是拒绝；但“网络围栏”不应再单独要求。
    let err = mode.check(&allow_net, false).unwrap_err();
    assert!(!err.contains("网络围栏"), "{}", err);
}

/// 未授权时段的放行规矩：**只有本机装不上能降级**。
/// 自检已确认机制有效却仍装不上 = 我们写错了，那一路必须拒绝执行——
/// 按无围栏跑等于用户以为有围栏、实际什么都没有（回执文案由进程机制代码返回）。
#[cfg(windows)]
#[test]
fn broken_mechanism_refuses_execution_instead_of_degrading() {
    let texts = code_texts();
    let broken = refuse_when_broken(
        &*texts,
        confine::FenceVerdict::Broken("profile 写错".to_string()),
    )
    .expect("我们写错了必须拒绝执行");
    assert!(!broken.ok, "拒绝执行时 ok 必须为假");
    assert_eq!(
        broken.output,
        texts.fence_failed(),
        "回执用册子里的固定说法"
    );
    assert!(
        refuse_when_broken(&*texts, confine::FenceVerdict::Enforced).is_none(),
        "机制装上了就没有拒绝的理由"
    );
    assert!(
        refuse_when_broken(
            &*texts,
            confine::FenceVerdict::EnvUnavailable("内核不支持".to_string())
        )
        .is_none(),
        "本机不允许是环境结论：如实降级照跑，不拒绝"
    );
}

// ---------- 真实工具进程（T2 真实适配器边界；见 docs/testing/doubles.md 的 ProcessRunner 行） ----------
/// 已构建的产品可执行文件（守门进程就是它自己）：`cargo build` 之后才存在；没有就如实跳过。
fn built_exe() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let profile_dir = me.parent()?.parent()?; // target/<profile>/deps → target/<profile>
    let name = if cfg!(windows) {
        "solomni.exe"
    } else {
        "solomni"
    };
    let p = profile_dir.join(name);
    if p.is_file() { Some(p) } else { None }
}

/// 本机可用的 python（没有就如实跳过需要解释器的用例）。
fn python() -> Option<&'static str> {
    for name in ["python", "python3"] {
        if let Ok(o) = std::process::Command::new(name)
            .arg("-c")
            .arg("print(1)")
            .output()
            && o.status.success()
        {
            return Some(name);
        }
    }
    None
}

/// 本机可用的 node（没有就如实跳过需要它的用例）。解释器基线只在 Windows 的容器围栏上需要。
#[cfg(windows)]
fn node() -> Option<&'static str> {
    let ok = std::process::Command::new("node")
        .arg("-v")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    if ok { Some("node") } else { None }
}

fn code_texts() -> std::sync::Arc<dyn crate::kernel::ports::ProcessTexts> {
    std::sync::Arc::new(CodeProcessTexts)
}

/// 造一个按「cwd = 隔离根」跑真工具的 runner（命令用裸文件名，避免命令行里出现引号）。
fn real_runner(exe: PathBuf, home: &std::path::Path, timeout_secs: u64) -> ProcTools {
    let mut t = ProcTools::new(
        exe,
        code_texts(),
        home.to_path_buf(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    t.timeout = Duration::from_secs(timeout_secs);
    t
}

fn spec_for(dir: &Path) -> FenceSpec {
    FenceSpec {
        agent: "proc".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![dir.to_path_buf()],
        cwd: dir.to_path_buf(),
        ro: Vec::new(),
        net: false,
    }
}

/// 真实工具进程：stdin 的 JSON 原样送达，退出码决定 ok（不猜、不吞）。
#[test]
fn real_tool_process_receives_stdin_json_and_reports_success() {
    let Some(exe) = built_exe() else {
        eprintln!(
            "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过真实工具进程契约"
        );
        return;
    };
    let Some(py) = python() else {
        eprintln!("[探针] 本机没有可用的 python，跳过真实工具进程契约");
        return;
    };
    let dir = crate::tests::scratch("proc-tools-stdin");
    std::fs::write(
        dir.join("echo_stdin.py"),
        "import sys\nprint(sys.stdin.read().strip())\n",
    )
    .expect("写脚本");
    let tools = real_runner(exe, &dir, 60);
    let out = tools.run(
        &spec_for(&dir),
        &format!("{} echo_stdin.py", py),
        "{\"k\":\"v\"}",
        &[],
        None,
        None,
        None,
    );
    assert!(out.ok, "工具应当成功：{}", out.output);
    assert!(
        out.output.contains("{\"k\":\"v\"}"),
        "stdin 的 JSON 必须原样送达工具：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 真实工具进程：父进程的无关变量（密钥之类）不得进工具进程——环境白名单要在真进程上生效。
#[test]
fn real_tool_process_does_not_inherit_foreign_env() {
    let Some(exe) = built_exe() else {
        eprintln!(
            "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过工具进程环境白名单契约"
        );
        return;
    };
    let Some(py) = python() else {
        eprintln!("[探针] 本机没有可用的 python，跳过工具进程环境白名单契约");
        return;
    };
    let dir = crate::tests::scratch("proc-tools-env");
    std::fs::write(
        dir.join("echo_env.py"),
        "import os\nprint('LEAK=' + str(os.environ.get('SOLOMNI_PROBE_ENV_LEAK')))\n",
    )
    .expect("写脚本");
    // SAFETY: 只改本用例独占的探针变量，用例内即改即清。
    unsafe {
        std::env::set_var("SOLOMNI_PROBE_ENV_LEAK", "leak-me");
    }
    let tools = real_runner(exe, &dir, 60);
    let out = tools.run(
        &spec_for(&dir),
        &format!("{} echo_env.py", py),
        "{}",
        &[],
        None,
        None,
        None,
    );
    // SAFETY: 同上。
    unsafe {
        std::env::remove_var("SOLOMNI_PROBE_ENV_LEAK");
    }
    assert!(out.ok, "工具应当成功：{}", out.output);
    assert!(
        out.output.contains("LEAK=None"),
        "无关变量不得进工具进程：{}",
        out.output
    );
    assert!(
        !out.output.contains("leak-me"),
        "密钥不得进工具进程：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 解释器基线要在**真进程**上成立：`node <文件>` 的工具进程拿到 `NODE_OPTIONS`（容器里 node 靠它跳过
/// realpath，否则脚本执行前就 EPERM 死）；别的解释器看不到它——注入面只覆盖真正需要它的那一种命令。
#[cfg(windows)]
#[test]
fn real_node_tool_process_carries_the_realpath_skip() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过解释器基线契约");
        return;
    };
    let Some(node) = node() else {
        eprintln!("[探针] 本机没有可用的 node，跳过解释器基线契约");
        return;
    };
    let dir = crate::tests::scratch("proc-tools-node-env");
    std::fs::write(
        dir.join("echo_node_opts.js"),
        "process.stdout.write('OPTS=' + String(process.env.NODE_OPTIONS));",
    )
    .expect("写脚本");
    let tools = real_runner(exe, &dir, 60);
    let out = tools.run(
        &spec_for(&dir),
        &format!("{} echo_node_opts.js", node),
        "{}",
        &[],
        None,
        None,
        None,
    );
    assert!(out.ok, "node 工具应当成功：{}", out.output);
    for flag in ["--preserve-symlinks", "--preserve-symlinks-main"] {
        assert!(
            out.output.contains(flag),
            "工具进程要拿到解释器基线（缺 {}）：{}",
            flag,
            out.output
        );
    }
    if let Some(py) = python() {
        std::fs::write(
            dir.join("echo_node_opts.py"),
            "import os\nprint('OPTS=' + str(os.environ.get('NODE_OPTIONS')))\n",
        )
        .expect("写 python 脚本");
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} echo_node_opts.py", py),
            "{}",
            &[],
            None,
            None,
            None,
        );
        assert!(out.ok, "python 工具应当成功：{}", out.output);
        assert!(
            out.output.contains("OPTS=None"),
            "非 node 命令不该带解释器基线：{}",
            out.output
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

/// 命令里的**程序名**写成带正斜杠的相对路径也必须跑得起来：Windows 的 cmd 不认程序名里的 `/`
/// （`build/indexer build` 会被它读成命令 `build` + 开关 `/indexer`），守门进程要按平台把程序名里的 `/` 转成 `\`。
/// 参数里的正斜杠不受影响——`node tools/report.js` 这类命令靠的就是它。
#[test]
fn real_tool_process_runs_a_relative_program_path() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过相对程序名契约");
        return;
    };
    let dir = crate::tests::scratch("proc-tools-relative-program");
    let sub = dir.join("sub");
    std::fs::create_dir_all(&sub).expect("建子目录");
    let (name, body, command) = if cfg!(windows) {
        (
            "probe.cmd",
            "@echo off\r\necho PROBE-OK\r\n",
            "sub/probe.cmd",
        )
    } else {
        ("probe.sh", "echo PROBE-OK\n", "sub/probe.sh")
    };
    let script = sub.join(name);
    std::fs::write(&script, body).expect("写脚本");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perm = std::fs::metadata(&script).expect("读权限位").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script, perm).expect("加执行位");
    }
    let tools = real_runner(exe, &dir, 60);
    let out = tools.run(&spec_for(&dir), command, "{}", &[], None, None, None);
    assert!(out.ok, "带正斜杠的相对程序名必须能跑起来：{}", out.output);
    assert!(
        out.output.contains("PROBE-OK"),
        "工具输出要如实回来：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 超时：连根杀掉整棵树并如实回执失败（不静默、不无限等它自然结束）。
#[test]
fn real_tool_process_is_killed_on_timeout_and_reported() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的 solomni 可执行文件，跳过超时杀树契约");
        return;
    };
    let Some(py) = python() else {
        eprintln!("[探针] 本机没有可用的 python，跳过超时杀树契约");
        return;
    };
    if !crate::kernel::detail::confine::capability().tree {
        eprintln!("[探针] 本机进程树围栏不可用，跳过超时杀树契约");
        return;
    }
    let dir = crate::tests::scratch("proc-tools-timeout");
    std::fs::write(dir.join("sleep60.py"), "import time\ntime.sleep(60)\n").expect("写脚本");
    let tools = real_runner(exe, &dir, 2);
    let started = Instant::now();
    let out = tools.run(
        &spec_for(&dir),
        &format!("{} sleep60.py", py),
        "{}",
        &[],
        None,
        None,
        None,
    );
    assert!(!out.ok, "超时必须如实回执失败：{}", out.output);
    assert!(
        out.output.contains(&tools.texts.timeout()),
        "回执要带上超时标记：{}",
        out.output
    );
    assert!(
        started.elapsed() < Duration::from_secs(30),
        "超时后不得继续等它自然结束（实际 {:?}）",
        started.elapsed()
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// 开了授权的 runner：只有开了才会走到"必要落点授不上"这条路（关着是用户自己选的档位）。
/// 授权是把本机目录 ACL 写给容器身份（真机探针才用，所以它在 `--fence-live` 之外没人调）。
#[cfg(windows)]
fn real_runner_with_write(exe: PathBuf, home: &std::path::Path) -> ProcTools {
    let mut t = ProcTools::new(
        exe,
        code_texts(),
        home.to_path_buf(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true)),
    );
    t.timeout = Duration::from_secs(60);
    t
}

/// 是否允许跑"会改本机状态"的真机探针（默认否：测试不该在真机上留下痕迹）。
#[cfg(windows)]
fn fence_live() -> bool {
    std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// 探针用的提问端口替身：记下每次问到的选项 id，按脚本作答（`None` = 拒绝 / 没人答）。
#[cfg(windows)]
struct RecordingAsk {
    answer: Option<String>,
    asked: std::sync::Mutex<Vec<Vec<String>>>,
    halted: std::sync::Mutex<Vec<String>>,
}

#[cfg(windows)]
impl RecordingAsk {
    fn new(answer: Option<&str>) -> RecordingAsk {
        RecordingAsk {
            answer: answer.map(|s| s.to_string()),
            asked: std::sync::Mutex::new(Vec::new()),
            halted: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn asked_ids(&self) -> Vec<Vec<String>> {
        self.asked.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

#[cfg(windows)]
impl crate::kernel::ports::AskUser for RecordingAsk {
    fn ask(&self, ask: &crate::kernel::api::Ask) -> crate::kernel::api::AskOutcome {
        use crate::kernel::api::AskOutcome;
        self.asked
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(ask.options.iter().map(|(id, _)| id.clone()).collect());
        match &self.answer {
            Some(id) => AskOutcome::Chosen(id.clone()),
            // 替身按脚本作答；脚本没写 = 没人答（发起方按声明 / fail-closed 收场）。
            None => AskOutcome::NoAnswer,
        }
    }

    fn halt(&self, why: &str) {
        self.halted
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(why.to_string());
    }
}

/// **真机探针（T4）**：**必要落点授不上时绝不静默无围栏执行**。
/// 本机构造这一态：解释器装在属主不是当前用户的目录里（系统级安装，当前用户对那个目录没有写 DACL 的权限）——
/// 那里的 DACL 写不进（错误码 5），于是**任何**这种解释器的工具都曾静默变成无围栏跑。
/// 断言三条：选了"本轮无围栏跑一次"才跑（且回执如实标为无围栏）；选"放弃"不执行；没有可回答的前端也不执行。
/// 本机构造不出这一态（解释器目录授得进）就如实 env-skip——不静默当作通过。
#[cfg(windows)]
#[test]
fn unwritable_interpreter_dir_never_silently_runs_unfenced() {
    use crate::capabilities::tools::domain::fence::{OPT_FENCE_ABORT, OPT_FENCE_UNFENCED};
    use crate::capabilities::tools::service::fence::ToolFenceGate;
    use crate::kernel::api::FencePart;
    if !fence_live() {
        eprintln!("[探针] 未开启真机围栏测试：本探针要写本机权限项，已跳过；要真跑加 --fence-live");
        return;
    }
    let Some(node) = node() else {
        eprintln!("[探针] 本机没有可用的 node：这条探针构造不出解释器目录授不上，跳过");
        return;
    };
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过真机探针");
        return;
    };
    let dir = crate::tests::scratch("proc-tools-fence-blocked");
    let home = dir.join(".home");
    std::fs::write(dir.join("probe.js"), "process.stdout.write('RAN');").expect("写脚本");
    let spec = spec_for(&dir);
    let command = format!("{} probe.js", node);
    // 先问机制：这一态在本机构造得出来吗（真授不上才继续，能授权就如实跳过）。
    let prep = confine::prepare_fence(&spec, &command, &home);
    let Some(blocked) = prep.blocked.as_ref() else {
        eprintln!(
            "[探针] 本机解释器目录授得进（构造不出\"授不上\"这一态）：真机探针跳过（不静默当作通过）"
        );
        let _ = confine::clean(&home);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    };
    if blocked.part != FencePart::Interpreter {
        eprintln!(
            "[诊断] 本机授不上的不是解释器目录，而是 {}：这条探针仍按同一条规矩断言",
            blocked.line()
        );
    }
    let tools = real_runner_with_write(exe, &home);
    let gate = ToolFenceGate;
    // ① 用户选"本轮无围栏跑一次"：命令真跑，回执**如实标为无围栏**。
    let ask = RecordingAsk::new(Some(OPT_FENCE_UNFENCED));
    let out = tools.run(&spec, &command, "{}", &[], None, Some(&ask), Some(&gate));
    assert_eq!(
        ask.asked_ids(),
        vec![vec![
            OPT_FENCE_UNFENCED.to_string(),
            OPT_FENCE_ABORT.to_string()
        ]],
        "必要落点授不上要**问**用户（选项 id 是契约）"
    );
    assert!(
        !blocked.why.contains("回滚失败"),
        "写入没成功就不许报“回滚失败”（否则用户以为本机权限被改了一半）：{}",
        blocked.why
    );
    assert!(
        ask.halted.lock().expect("锁").is_empty(),
        "有选项就不该停会话"
    );
    assert!(
        // 册子文案带占位符，渲染后才进回执：只断言"如实标为无围栏"这个稳定事实，不复制整段文案。
        out.output.contains("无围栏"),
        "回执要如实标为无围栏：{}",
        out.output
    );
    assert!(
        out.output.contains("RAN"),
        "选了跑一次就要真跑起来：{}",
        out.output
    );
    // ② 用户选"放弃这次调用"：不执行，回执写清哪一环、哪个目录、缺什么、怎么补。
    let ask = RecordingAsk::new(Some(OPT_FENCE_ABORT));
    let out = tools.run(&spec, &command, "{}", &[], None, Some(&ask), Some(&gate));
    assert!(!out.ok, "放弃 = 不执行");
    assert!(
        !out.output.contains("RAN"),
        "放弃之后命令绝不许跑：{}",
        out.output
    );
    for want in [blocked.part.label(), "没有执行", "怎么补"] {
        assert!(
            out.output.contains(want),
            "回执要写清 {}：{}",
            want,
            out.output
        );
    }
    // ③ 没有可回答的前端（纯终端 / e2e）：没有可点的选项 = 不执行（fail-closed）。
    let out = tools.run(&spec, &command, "{}", &[], None, None, Some(&gate));
    assert!(!out.ok, "没有可回答的前端 = 不执行");
    assert!(
        !out.output.contains("RAN"),
        "没有可回答的前端时命令绝不许跑：{}",
        out.output
    );
    assert!(out.output.contains("没有执行"), "{}", out.output);
    // 收尾：撤掉这次写下的授权、删掉台账——测试不在本机留痕。
    if let Err(e) = confine::release_fence(&spec, &home) {
        eprintln!("[诊断] 撤权未完成（{}）：要收尾请跑 --fence-clean", e);
    }
    confine::clean(&home).ok();
    let _ = std::fs::remove_dir_all(&dir);
}

// ---------- 一次性 shell（T3 真实进程） ----------
#[cfg(unix)]
fn shell_fence_ready() -> bool {
    let c = confine::capability();
    c.fs && c.net
}

#[cfg(unix)]
#[test]
fn real_shell_reports_chinese_and_exit_code() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 契约");
        return;
    };
    if !shell_fence_ready() {
        eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 契约");
        return;
    }
    let dir = crate::tests::scratch("proc-shell-basic");
    let tools = real_runner(exe, &dir, 30);
    let out = tools.run_shell(&crate::kernel::ports::ShellRun {
        fence: &spec_for(&dir),
        script: "printf '中文\\n'; exit 3",
        timeout: Duration::from_secs(10),
        cancel: None,
        path: None,
        ask: None,
        gate: None,
    });
    assert!(!out.ok, "退出码 3 应如实失败：{}", out.output);
    assert!(out.output.contains("中文"), "{}", out.output);
    assert!(
        out.output.contains("[shell] 退出码 3"),
        "回执要带退出码：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn real_shell_keeps_head_and_tail_when_output_is_long() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 截断契约");
        return;
    };
    if !shell_fence_ready() {
        eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 截断契约");
        return;
    }
    let dir = crate::tests::scratch("proc-shell-truncate");
    let tools = real_runner(exe, &dir, 30);
    let out = tools.run_shell(&crate::kernel::ports::ShellRun {
        fence: &spec_for(&dir),
        script: "i=0; while [ $i -lt 20000 ]; do printf a; i=$((i+1)); done; printf TAIL",
        timeout: Duration::from_secs(10),
        cancel: None,
        path: None,
        ask: None,
        gate: None,
    });
    assert!(out.ok, "{}", out.output);
    assert!(out.output.contains("TAIL"), "结尾要保留：{}", out.output);
    assert!(
        out.output.contains("开头") && out.output.contains("结尾"),
        "截断标记要如实说清：{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn real_shell_timeout_and_cancel_kill_the_tree() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 超时契约");
        return;
    };
    if !shell_fence_ready() {
        eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 超时契约");
        return;
    }
    let dir = crate::tests::scratch("proc-shell-stop");
    let tools = real_runner(exe, &dir, 30);
    let spec = spec_for(&dir);
    let started = Instant::now();
    let out = tools.run_shell(&crate::kernel::ports::ShellRun {
        fence: &spec,
        script: "sleep 5",
        timeout: Duration::from_secs(1),
        cancel: None,
        path: None,
        ask: None,
        gate: None,
    });
    assert!(!out.ok, "{}", out.output);
    assert!(out.output.contains("超时"), "{}", out.output);
    assert!(
        started.elapsed() < Duration::from_secs(4),
        "超时后不得继续等"
    );

    let flag = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let set = std::sync::Arc::clone(&flag);
    let spec2 = spec.clone();
    let started = Instant::now();
    let h = std::thread::spawn(move || {
        tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec2,
            script: "sleep 5",
            timeout: Duration::from_secs(30),
            cancel: Some(&set),
            path: None,
            ask: None,
            gate: None,
        })
    });
    std::thread::sleep(Duration::from_millis(300));
    flag.store(true, std::sync::atomic::Ordering::Relaxed);
    let out = h.join().expect("shell 线程");
    assert!(!out.ok, "{}", out.output);
    assert!(out.output.contains("用户停止"), "{}", out.output);
    assert!(started.elapsed() < Duration::from_secs(5), "停止要立刻收树");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn real_shell_fence_denies_outside_paths_and_marks_the_boundary() {
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 shell 围栏契约");
        return;
    };
    if !shell_fence_ready() {
        eprintln!("[探针] 本机围栏不可用（环境结论），跳过 shell 围栏契约");
        return;
    }
    let inside = crate::tests::scratch("proc-shell-inside");
    let outside = crate::tests::scratch("proc-shell-outside");
    let secret = outside.join("secret.txt");
    std::fs::write(&secret, "SHELL-SECRET").expect("写 secret");
    let mut spec = spec_for(&inside);
    spec.private = inside.clone();
    let tools = real_runner(exe, &inside, 30);
    let out = tools.run_shell(&crate::kernel::ports::ShellRun {
        fence: &spec,
        script: &format!("cat {}", secret.display()),
        timeout: Duration::from_secs(10),
        cancel: None,
        path: None,
        ask: None,
        gate: None,
    });
    assert!(!out.ok, "{}", out.output);
    assert!(!out.output.contains("SHELL-SECRET"), "{}", out.output);
    assert!(
        out.output.contains(crate::kernel::api::FENCE_BOUNDARY_MARK),
        "{}",
        out.output
    );
    let _ = std::fs::remove_dir_all(&inside);
    let _ = std::fs::remove_dir_all(&outside);
}

/// 目的：生成的 .ps1 让**未处理的 PowerShell 错误**以非 0 收场；显式容错与原生退出码不变。
/// 约束：这条契约只测脚本包装本身，不装容器围栏（CI 无 ACL 也能跑）；缺 PowerShell 就如实跳过。
#[cfg(windows)]
#[test]
fn generated_shell_script_fails_on_unhandled_powershell_errors() {
    let dir = crate::tests::scratch("proc-shell-script-stop");
    let tools = ProcTools::new(
        PathBuf::from("not-used"),
        code_texts(),
        dir.clone(),
        std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    );
    let program = tools.powershell_program();
    if crate::kernel::detail::host_probe::find_exe(program).is_none() {
        eprintln!("[探针] 本机没有 {}：跳过 PowerShell 脚本契约", program);
        let _ = std::fs::remove_dir_all(&dir);
        return;
    }
    let spec = spec_for(&dir);
    let cases = [
        (
            "Get-Content C:\\definitely-missing-solomni-xyz",
            Some(1),
            "未处理的 cmdlet 错误要以 1 中止",
        ),
        ("Write-Error \"boom\"", Some(1), "Write-Error 要以 1 中止"),
        (
            "Get-Content C:\\definitely-missing-solomni-xyz -ErrorAction SilentlyContinue; Write-Output ok",
            Some(0),
            "显式静默的错误不算失败",
        ),
        ("cmd /c exit 3", Some(3), "原生退出码照旧转发"),
    ];
    for (script, want, why) in cases {
        let (_, path) = tools
            .shell_script_file(&crate::kernel::ports::ShellRun {
                fence: &spec,
                script,
                timeout: Duration::from_secs(10),
                cancel: None,
                path: None,
                ask: None,
                gate: None,
            })
            .expect("生成 shell 脚本");
        let out = std::process::Command::new(program)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-ExecutionPolicy",
                "Bypass",
                "-File",
            ])
            .arg(&path)
            .output()
            .expect("跑 PowerShell");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            want,
            "{}（{}，script = {}）",
            why,
            stderr,
            script
        );
        if want == Some(1) {
            assert!(!stderr.trim().is_empty(), "错误要留在 stderr：{}", script);
        }
        let _ = std::fs::remove_file(&path);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn real_shell_uses_powershell_utf8_exit_code_and_cleans_its_script() {
    if !fence_live() {
        eprintln!("[探针] 未开启真机围栏测试（会写本机 ACL）：跳过 Windows shell 契约");
        return;
    }
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 Windows shell 契约");
        return;
    };
    for program in ["pwsh", "powershell"] {
        if crate::kernel::detail::host_probe::find_exe(program).is_none() {
            eprintln!("[探针] 本机没有 {}：跳过它的 PowerShell 契约", program);
            continue;
        }
        let dir = crate::tests::scratch(&format!("proc-shell-{}", program));
        let home = dir.join(".home");
        let tools = real_runner_with_write(exe.clone(), &home).with_powershell(Some(program));
        let mut spec = spec_for(&dir);
        spec.private = dir.clone();
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec,
            script: "Write-Output '中文'; exit 3",
            timeout: Duration::from_secs(30),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(!out.ok, "{}（{}）", out.output, program);
        assert!(out.output.contains("中文"), "{}（{}）", out.output, program);
        assert!(
            out.output.contains("[shell] 退出码 3"),
            "{}（{}）",
            out.output,
            program
        );
        // 未处理 cmdlet 错误也要非 0。用 Write-Error：它只依赖内核自带 Utility；
        // 容器里的 5.1 找不到 Management（Get-Content 会变 CommandNotFound），不适合验这条契约。
        let out = tools.run_shell(&crate::kernel::ports::ShellRun {
            fence: &spec,
            script: "Write-Error 'boom'",
            timeout: Duration::from_secs(30),
            cancel: None,
            path: None,
            ask: None,
            gate: None,
        });
        assert!(!out.ok, "未处理错误要失败（{}）：{}", program, out.output);
        assert!(
            out.output.contains("[shell] 退出码 1"),
            "{}（{}）",
            out.output,
            program
        );
        let leftovers = std::fs::read_dir(&dir)
            .expect("读 shell 目录")
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".solomni-shell-")
            })
            .count();
        assert_eq!(leftovers, 0, "临时 .ps1 要清掉");
        let _ = confine::release_fence(&spec, &home);
        confine::clean(&home).ok();
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// 目的：Windows 真机上相对路径必须落在私有沙箱 cwd（shell 的 cwd 契约）。
#[cfg(windows)]
#[test]
fn real_windows_shell_writes_relative_files_in_its_private_cwd() {
    if !fence_live() {
        eprintln!("[探针] 未开启真机围栏测试（会写本机 ACL）：跳过 Windows shell cwd 契约");
        return;
    }
    let Some(exe) = built_exe() else {
        eprintln!("[探针] 未找到已构建的产品可执行文件，跳过 Windows shell cwd 契约");
        return;
    };
    let dir = crate::tests::scratch("proc-shell-cwd");
    let home = dir.join(".home");
    let tools = real_runner_with_write(exe, &home);
    let mut spec = spec_for(&dir);
    spec.private = dir.clone();
    let out = tools.run_shell(&crate::kernel::ports::ShellRun {
        fence: &spec,
        script: "Set-Content -Path shell-ok.txt -Value 'CWD-OK' -Encoding utf8; Write-Output (Get-Location)",
        timeout: Duration::from_secs(30),
        cancel: None,
        path: None,
        ask: None,
        gate: None,
    });
    let file = dir.join("shell-ok.txt");
    assert!(
        file.is_file(),
        "shell 的相对路径要落在私有沙箱：{}\n{}",
        file.display(),
        out.output
    );
    assert!(
        std::fs::read_to_string(&file)
            .unwrap_or_default()
            .contains("CWD-OK"),
        "文件内容要写进去"
    );
    assert!(
        out.output.contains(&dir.display().to_string()),
        "Get-Location 要是私有沙箱：{}\n{}",
        dir.display(),
        out.output
    );
    let _ = confine::release_fence(&spec, &home);
    confine::clean(&home).ok();
    let _ = std::fs::remove_dir_all(&dir);
}

/// 目的：围栏准备耗时的同线程交棒语义——写后读到、读过即清、每趟入口清空、非 Windows 恒无值。
/// 约束：直接操作本模块的 thread_local（同线程），不真写本机 ACL。
#[test]
fn fence_prep_handoff_is_read_once_and_reset_each_run() {
    reset_fence_prep();
    assert!(take_fence_prep().is_none(), "没走过围栏准备就没有值");

    let seven = FenceTiming {
        total: Duration::from_secs(7),
        steps: Vec::new(),
    };
    FENCE_PREP.with(|slot| slot.set(Some(seven.clone())));
    assert_eq!(take_fence_prep(), Some(seven.clone()));
    assert_eq!(seven.as_secs(), 7, "回执口径仍是总秒数");
    assert!(
        take_fence_prep().is_none(),
        "读即清：同一趟的交棒只能被取走一次"
    );

    // 上一趟留下没取走的值，下一趟入口会清掉（线程池复用同一线程时不能串味）。
    FENCE_PREP.with(|slot| {
        slot.set(Some(FenceTiming {
            total: Duration::from_secs(9),
            steps: Vec::new(),
        }))
    });
    reset_fence_prep();
    assert!(take_fence_prep().is_none(), "新一趟入口先清空");

    #[cfg(not(windows))]
    assert!(
        take_fence_prep().is_none(),
        "非 Windows 没有围栏准备，恒为空"
    );
}

/// 守门进程起不来：如实回执「启动失败」，不 panic、不假装跑过。
#[test]
fn missing_launcher_binary_is_reported_honestly() {
    let dir = crate::tests::scratch("proc-tools-missing-exe");
    let tools = real_runner(PathBuf::from("definitely-not-here-solomni"), &dir, 5);
    let out = tools.run(&spec_for(&dir), "echo hi", "{}", &[], None, None, None);
    assert!(!out.ok);
    assert!(out.output.contains("工具进程启动失败"), "{}", out.output);
    let _ = std::fs::remove_dir_all(&dir);
}
