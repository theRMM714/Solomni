//! 工具执行适配器：把核心放行的工具命令拉进围栏里跑（实现 core 的 ToolRunner 端口）。
//! 机制边界：外层拉起**守门进程**（本程序的 --fence-run 模式）——围栏（可达范围、断网、进程树围栏、
//! 环境白名单）由守门进程装进真正的工具进程；命令行来自 module.yaml，JSON 参数走 stdin（不进命令行，杜绝注入）；
//! 截获 stdout/stderr、超时连根杀掉整棵树、输出截断。

use crate::adapters::confine;
use crate::core::fence::FenceSpec;
use crate::core::ports::{ToolOutcome, ToolRunner};
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcTools {
    /// 守门进程用的可执行文件（组合根注入当前程序路径）。
    pub exe: PathBuf,
    /// 产品私有区（`.home/`）：围栏授权台账落在这里，供 `--fence-clean` 精确回收。
    /// 只有 Windows 的容器围栏需要写目录 ACL，所以下面这三项只在本平台存在。
    #[cfg(windows)]
    pub home: PathBuf,
    /// 是否允许在本机写权限（由组合根按设置与 `SOLOMNI_FENCE_WRITE` 注入；默认否）。
    #[cfg(windows)]
    pub write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 「未授权」的提示只出一次，不刷屏。
    #[cfg(windows)]
    pub disclosed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 工具回执里那些收尾标记的文案（来自提示词册：它们随 [工具结果] 进模型上下文，所以不硬编码）。
    pub texts: crate::core::prompt::ToolTexts,
    /// 单次工具执行的超时（到时连根杀掉整棵树，ok = false）。
    pub timeout: Duration,
    /// 回传给模型/轨迹的输出上限（字符数）。
    pub max_output_chars: usize,
    /// Windows：已经授权过的 (SID, 路径, 权限) 台账（避免每次工具调用重复改目录 ACL）。
    #[cfg(windows)]
    pub prepared: std::sync::Mutex<std::collections::BTreeSet<String>>,
}

impl ProcTools {
    /// 组合根注入：当前可执行文件（守门进程就是它自己）、提示词册里的收尾标记、产品私有区与写权限开关。
    pub fn new(
        exe: PathBuf,
        texts: crate::core::prompt::ToolTexts,
        home: PathBuf,
        write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> ProcTools {
        // 其它平台没有容器围栏这一步：家目录与写权限开关不参与装配，显式忽略以免被误读成漏用。
        #[cfg(not(windows))]
        let _ = (&home, &write_allowed);
        ProcTools {
            exe,
            #[cfg(windows)]
            home,
            #[cfg(windows)]
            write_allowed,
            #[cfg(windows)]
            disclosed: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            texts,
            timeout: Duration::from_secs(30),
            max_output_chars: 16_000,
            #[cfg(windows)]
            prepared: std::sync::Mutex::new(std::collections::BTreeSet::new()),
        }
    }
}

impl ToolRunner for ProcTools {
    fn run(&self, fence: &FenceSpec, command: &str, args_json: &str) -> ToolOutcome {
        // Windows：容器围栏要先把「可达范围」授权给容器 SID。
        // **默认不写本机任何权限项**：只有用户显式授权（设置里的 fence_write，或环境变量 SOLOMNI_FENCE_WRITE=1）才做；
        // 授权做成了才让守门进程去装容器——没做成就是无围栏执行，这一点如实进工具回执的 stderr。
        #[cfg(windows)]
        let prepared = {
            use std::sync::atomic::Ordering;
            let mut prepared = false;
            if self.write_allowed.load(Ordering::Relaxed) {
                match confine::prepare_fence(fence, command, &self.prepared, &self.home) {
                    Ok(()) => prepared = true,
                    Err(e) => eprintln!("[围栏] 授权未完成（{}）：本次按无围栏执行", e),
                }
            } else if !self.disclosed.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[围栏] 容器围栏未启用（没有授权在本机写权限）：外部工具按无围栏执行。要启用：在设置里打开，或在 .home/settings.yaml 写 fence_write: true"
                );
            }
            if !prepared {
                // 未授权时段先问机制：环境不允许就如实降级，我们写错了就拒绝执行（见 refuse_when_broken）。
                if let Some(outcome) =
                    refuse_when_broken(&self.texts, confine::verify(fence, command))
                {
                    return outcome;
                }
            }
            prepared
        };
        // 其它平台没有容器围栏，也就没有"要先授权"这一步。
        #[cfg(not(windows))]
        let prepared = false;
        // 容器 profile 的台账落点：只有 Windows 的守门进程会写它（外层不知道 profile 建成了没有）。
        #[cfg(windows)]
        let home = Some(self.home.clone());
        #[cfg(not(windows))]
        let home: Option<std::path::PathBuf> = None;
        let job = confine::FenceJob {
            spec: fence.clone(),
            prepared,
            home,
        };
        let mut cmd = confine::launcher(&self.exe, &job, command);
        cmd.current_dir(&fence.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // 环境白名单：不继承父进程环境（密钥与无关凭据不进工具进程）；HOME/TEMP 落进该 agent 的私有沙箱。
        cmd.env_clear();
        for (k, v) in confine::fence_env(fence) {
            cmd.env(k, v);
        }
        // 独立进程组：Unix 上超时/停止能杀整棵树；Windows 侧由守门进程的 Job Object 兜住。
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                return ToolOutcome {
                    ok: false,
                    output: format!("工具进程启动失败：{}", e),
                }
            }
        };
        // stdin 独立线程送参：参数再大也不与 stdout 读取互锁。
        let mut stdin = child.stdin.take().expect("stdin 已声明管道");
        let payload = format!("{}\n", args_json);
        let writer = thread::spawn(move || {
            use std::io::Write;
            let _ = stdin.write_all(payload.as_bytes());
            let _ = stdin.flush();
        });
        let mut stdout_pipe = child.stdout.take().expect("stdout 已声明管道");
        let mut stderr_pipe = child.stderr.take().expect("stderr 已声明管道");
        let out_reader = thread::spawn(move || read_to_string(&mut stdout_pipe));
        let err_reader = thread::spawn(move || read_to_string(&mut stderr_pipe));

        // 轮询等待至截止；到时连根杀掉整棵树（管道随之关闭，读线程自然结束）。
        let deadline = Instant::now() + self.timeout;
        let mut timed_out = false;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if Instant::now() >= deadline {
                        timed_out = true;
                        kill_tree(&mut child);
                        break;
                    }
                    thread::sleep(Duration::from_millis(20));
                }
                Err(_) => break,
            }
        }
        let _ = writer.join();
        let out = out_reader.join().unwrap_or_default();
        let err = err_reader.join().unwrap_or_default();
        let code = child.try_wait().ok().flatten().and_then(|s| s.code());
        self.assemble(out, err, timed_out, code)
    }
}

/// 未授权时段遇到机制自检结论时的放行规矩：**只有本机装不上能降级**。
/// 自检已确认机制有效却仍装不上 = 我们写错了——那种情况按无围栏跑，等于用户以为有围栏、实际什么都没有，
/// 所以拒绝执行（命令不落进程），回执用提示词册里的固定说法（它随工具结果进模型上下文）。
#[cfg(windows)]
fn refuse_when_broken(
    texts: &crate::core::prompt::ToolTexts,
    verdict: confine::FenceVerdict,
) -> Option<ToolOutcome> {
    match verdict {
        confine::FenceVerdict::Broken(why) => {
            eprintln!(
                "[围栏] 容器围栏机制装不上（{}）：本次拒绝执行，不按无围栏跑",
                why
            );
            Some(ToolOutcome {
                ok: false,
                output: texts.tool_fence_failed.clone(),
            })
        }
        confine::FenceVerdict::Enforced | confine::FenceVerdict::EnvUnavailable(_) => None,
    }
}

/// 杀掉整棵进程树：工具进程 fork 出来的子孙一并收掉（不留孤儿）。
fn kill_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        // 负 pid = 整个进程组（守门进程是组长，组员含 shell 与工具本身）。
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.wait();
    }
    #[cfg(not(unix))]
    {
        // Windows：守门进程一死，它 Job Object 里的整棵树随之消亡（KILL_ON_JOB_CLOSE）。
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn read_to_string(r: &mut impl std::io::Read) -> String {
    let mut buf = Vec::new();
    let _ = r.read_to_end(&mut buf);
    String::from_utf8_lossy(&buf).into_owned()
}

impl ProcTools {
    /// 把工具进程的原始输出与事实拼成回执：成功与否看退出码；
    /// stderr 段头、超时、围栏没装上、截断这些**标记文案全部来自提示词册**——它们随工具结果进模型上下文。
    fn assemble(
        &self,
        out: String,
        err: String,
        timed_out: bool,
        code: Option<i32>,
    ) -> ToolOutcome {
        let mut ok = !timed_out && code == Some(0);
        let mut output = out;
        if !err.is_empty() {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(&self.texts.tool_stderr_header);
            output.push('\n');
            output.push_str(&err);
        }
        if timed_out {
            output.push('\n');
            output.push_str(&self.texts.tool_timeout);
        }
        // 围栏没装上：守门进程用固定退出码报明（命令没被执行），这里如实告诉模型。
        if code == Some(confine::FENCE_FAILED) {
            ok = false;
            output.push('\n');
            output.push_str(&self.texts.tool_fence_failed);
        }
        ToolOutcome {
            ok,
            output: self.truncate(&output),
        }
    }

    /// 按字符截断（不劈开 UTF-8），尾部如实注明（文案来自提示词册）。
    fn truncate(&self, s: &str) -> String {
        let count = s.chars().count();
        if count <= self.max_output_chars {
            return s.to_string();
        }
        let head: String = s.chars().take(self.max_output_chars).collect();
        let tail = self.texts.render(
            &self.texts.tool_truncated,
            &[
                ("chars", count.to_string()),
                ("limit", self.max_output_chars.to_string()),
            ],
        );
        format!("{}\n{}", head, tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::fence::FenceSpec;
    // 测试夹具按 &Path 收参（clippy 的 ptr_arg）：Path 显式写在测试模块里，
    // 顶层只按需导入 PathBuf——否则顶层会多出一次"只被 glob 用到"的导入。
    use std::path::{Path, PathBuf};

    /// 环境白名单：父进程的无关变量（密钥之类）不进子进程；HOME / TEMP 落在该 agent 的私有沙箱里。
    #[test]
    fn env_whitelist_drops_foreign_vars_and_moves_home_into_the_sandbox() {
        std::env::set_var("SOLOMNI_PROBE_SECRET", "leak-me");
        let private = PathBuf::from("demo").join("agent-a");
        let spec = FenceSpec {
            agent: "a".to_string(),
            rw: vec![PathBuf::from("demo").join("work"), private.clone()],
            cwd: PathBuf::from("mods").join("m0"),
            ro: Vec::new(),
            net: false,
        };
        let env: Vec<(String, String)> = crate::adapters::confine::fence_env(&spec)
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
        std::env::remove_var("SOLOMNI_PROBE_SECRET");
    }

    /// 回执里的标记文案必须来自提示词册（它们随 [工具结果] 进模型上下文，所以不能在代码里另写一份）。
    #[test]
    fn receipt_markers_come_from_the_prompt_book() {
        use crate::core::ports::PromptSource;
        let prompts = crate::adapters::YamlPrompts::new(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts"),
        )
        .load()
        .expect("内置提示词册必须合法");
        let texts = prompts.core.tool_texts;
        let tools = ProcTools::new(
            PathBuf::from("solomni"),
            texts.clone(),
            PathBuf::from("target").join("test-scratch"),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );

        let ok = tools.assemble("正常输出".to_string(), String::new(), false, Some(0));
        assert!(ok.ok);
        assert_eq!(ok.output, "正常输出", "没有异常就不加任何标记");

        let with_err = tools.assemble("正文".to_string(), "警告".to_string(), false, Some(0));
        assert!(
            with_err.output.contains(&texts.tool_stderr_header) && with_err.output.contains("警告")
        );

        let timed = tools.assemble(String::new(), String::new(), true, None);
        assert!(!timed.ok);
        assert!(
            timed.output.contains(&texts.tool_timeout),
            "{}",
            timed.output
        );

        let fenced = tools.assemble(
            String::new(),
            String::new(),
            false,
            Some(confine::FENCE_FAILED),
        );
        assert!(!fenced.ok);
        assert!(
            fenced.output.contains(&texts.tool_fence_failed),
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

    /// 未授权时段的放行规矩：**只有本机装不上能降级**。
    /// 自检已确认机制有效却仍装不上 = 我们写错了，那一路必须拒绝执行——
    /// 按无围栏跑等于用户以为有围栏、实际什么都没有（回执文案取自提示词册）。
    #[cfg(windows)]
    #[test]
    fn broken_mechanism_refuses_execution_instead_of_degrading() {
        let texts = prompt_texts();
        let broken = refuse_when_broken(
            &texts,
            confine::FenceVerdict::Broken("profile 写错".to_string()),
        )
        .expect("我们写错了必须拒绝执行");
        assert!(!broken.ok, "拒绝执行时 ok 必须为假");
        assert_eq!(
            broken.output, texts.tool_fence_failed,
            "回执用册子里的固定说法"
        );
        assert!(
            refuse_when_broken(&texts, confine::FenceVerdict::Enforced).is_none(),
            "机制装上了就没有拒绝的理由"
        );
        assert!(
            refuse_when_broken(
                &texts,
                confine::FenceVerdict::EnvUnavailable("内核不支持".to_string())
            )
            .is_none(),
            "本机不允许是环境结论：如实降级照跑，不拒绝"
        );
    }

    // ---------- 真实工具进程（T2 真实适配器边界；见 docs/testing/port-matrix.md 的 ToolRunner 行） ----------

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
        if p.is_file() {
            Some(p)
        } else {
            None
        }
    }

    /// 本机可用的 python（没有就如实跳过需要解释器的用例）。
    fn python() -> Option<&'static str> {
        for name in ["python", "python3"] {
            if let Ok(o) = std::process::Command::new(name)
                .arg("-c")
                .arg("print(1)")
                .output()
            {
                if o.status.success() {
                    return Some(name);
                }
            }
        }
        None
    }

    fn prompt_texts() -> crate::core::prompt::ToolTexts {
        use crate::core::ports::PromptSource;
        let prompts = crate::adapters::YamlPrompts::new(
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts"),
        )
        .load()
        .expect("内置提示词册必须合法");
        prompts.core.tool_texts
    }

    /// 造一个按「cwd = 隔离根」跑真工具的 runner（命令用裸文件名，避免命令行里出现引号）。
    fn real_runner(exe: PathBuf, home: &std::path::Path, timeout_secs: u64) -> ProcTools {
        let mut t = ProcTools::new(
            exe,
            prompt_texts(),
            home.to_path_buf(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        );
        t.timeout = Duration::from_secs(timeout_secs);
        t
    }

    fn spec_for(dir: &Path) -> FenceSpec {
        FenceSpec {
            agent: "proc".to_string(),
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
            eprintln!("[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过工具进程环境白名单契约");
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
        std::env::set_var("SOLOMNI_PROBE_ENV_LEAK", "leak-me");
        let tools = real_runner(exe, &dir, 60);
        let out = tools.run(&spec_for(&dir), &format!("{} echo_env.py", py), "{}");
        std::env::remove_var("SOLOMNI_PROBE_ENV_LEAK");
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

    /// 命令里的**程序名**写成带正斜杠的相对路径也必须跑得起来：Windows 的 cmd 不认程序名里的 `/`
    /// （`build/indexer build` 会被它读成命令 `build` + 开关 `/indexer`），守门进程要按平台把程序名里的 `/` 转成 `\`。
    /// 参数里的正斜杠不受影响——`node tools/report.js` 这类命令靠的就是它。
    #[test]
    fn real_tool_process_runs_a_relative_program_path() {
        let Some(exe) = built_exe() else {
            eprintln!(
                "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过相对程序名契约"
            );
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
        let out = tools.run(&spec_for(&dir), command, "{}");
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
        if !crate::adapters::confine::capability().tree {
            eprintln!("[探针] 本机进程树围栏不可用，跳过超时杀树契约");
            return;
        }
        let dir = crate::tests::scratch("proc-tools-timeout");
        std::fs::write(dir.join("sleep60.py"), "import time\ntime.sleep(60)\n").expect("写脚本");
        let tools = real_runner(exe, &dir, 2);
        let started = Instant::now();
        let out = tools.run(&spec_for(&dir), &format!("{} sleep60.py", py), "{}");
        assert!(!out.ok, "超时必须如实回执失败：{}", out.output);
        assert!(
            out.output.contains(&tools.texts.tool_timeout),
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

    /// 守门进程起不来：如实回执「启动失败」，不 panic、不假装跑过。
    #[test]
    fn missing_launcher_binary_is_reported_honestly() {
        let dir = crate::tests::scratch("proc-tools-missing-exe");
        let tools = real_runner(PathBuf::from("definitely-not-here-solomni"), &dir, 5);
        let out = tools.run(&spec_for(&dir), "echo hi", "{}");
        assert!(!out.ok);
        assert!(out.output.contains("工具进程启动失败"), "{}", out.output);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
