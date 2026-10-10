//! 目的：工具执行适配器——把本能力放行的工具命令拉进围栏里跑（实现 kernel 共享的 ProcessRunner 端口）。
//! 管：外层拉起守门进程（本程序的 --fence-run 模式）；由它把围栏与环境白名单装进真正的工具进程；stdin 送参、stdout/stderr 截获、超时连根杀树、输出截断。
//! 不管：命令行与可达范围的策略（由调用方派生后传入）；命令来自 module.yaml，JSON 参数走 stdin（不进命令行）。
//! 联动：围栏机制在 confine；ProcessRunner 端口形状见 src/kernel/ports.rs。

use crate::kernel::api::FenceSpec;
use crate::kernel::api::ToolOutcome;
use crate::kernel::detail::confine;
use crate::kernel::ports::{ProcessRunner, ProcessTexts};
use std::path::PathBuf;
use std::process::Stdio;
use std::thread;
use std::time::{Duration, Instant};

pub struct ProcTools {
    /// 目的：守门进程用的可执行文件（组合根注入当前程序路径）。
    pub exe: PathBuf,
    /// 目的：产品私有区（`.home/`）：围栏授权台账落在这里，供 `--fence-clean` 精确回收。
    ///   只有 Windows 的容器围栏需要写目录 ACL，所以下面这三项只在本平台存在。
    #[cfg(windows)]
    pub home: PathBuf,
    /// 目的：是否允许在本机写权限（由组合根按设置与 `SOLOMNI_FENCE_WRITE` 注入；默认否）。
    #[cfg(windows)]
    pub write_allowed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 目的：「未授权」的提示只出一次，不刷屏。
    #[cfg(windows)]
    pub disclosed: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// 目的：工具回执里那些收尾标记的文案（由上层适配后经 kernel 端口注入：它们随 [工具结果] 进模型上下文）。
    ///   **共享一份**（调用方给出的 `Arc`）：这里不再各存一份深拷贝。
    pub texts: std::sync::Arc<dyn ProcessTexts>,
    /// 目的：单次工具执行的超时（到时连根杀掉整棵树，ok = false）。
    pub timeout: Duration,
    /// 目的：回传给模型/轨迹的输出上限（字符数）。
    pub max_output_chars: usize,
}

impl ProcTools {
    /// 目的：组合根注入：当前可执行文件（守门进程就是它自己）、提示词册里的收尾标记、产品私有区与写权限开关。
    pub fn new(
        exe: PathBuf,
        texts: std::sync::Arc<dyn ProcessTexts>,
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
        }
    }
}

impl ProcessRunner for ProcTools {
    // 提问端口只在 Windows 的容器围栏那一路用；unix 侧没有 prepare_fence，参数如实闲置。
    #[cfg_attr(not(windows), allow(unused_variables))]
    fn run(
        &self,
        fence: &FenceSpec,
        command: &str,
        args_json: &str,
        env: &[(String, String)],
        ask: Option<&dyn crate::kernel::ports::AskUser>,
    ) -> ToolOutcome {
        // Windows：容器围栏要先把「可达范围」授权给容器 SID。
        // **默认不写本机任何权限项**：只有用户显式授权（设置里的 fence_write，或环境变量 SOLOMNI_FENCE_WRITE=1）才做。
        #[cfg(windows)]
        let (prepared, unfenced_note) = {
            use std::sync::atomic::Ordering;
            let mut prepared = false;
            let mut note: Option<String> = None;
            if self.write_allowed.load(Ordering::Relaxed) {
                let prep = confine::prepare_fence(fence, command, &self.home);
                // 可选落点授不上只记事实（不牵动这次执行）：脚印打在 stderr，回执里不打扰模型。
                for n in &prep.notes {
                    eprintln!("[围栏] {}（可选落点，本次照常执行）", n);
                }
                // 必要落点授不上：**不许降级**——问用户（没有可回答的前端就按 fail-closed 拒绝）。
                if prep.ok() {
                    prepared = true;
                } else if let Some(blocked) = &prep.blocked {
                    match self.fence_blocked(ask, fence, blocked) {
                        FenceGo::Unfenced { note: n } => note = Some(n),
                        FenceGo::Refused(outcome) => return outcome,
                    }
                }
            } else if !self.disclosed.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "[围栏] 容器围栏未启用（用户没授权在本机写权限）：外部工具按无围栏执行（能力等级见启动报告）。要启用：在设置里打开，或在 .home/settings.yaml 写 fence_write: true"
                );
            }
            if !prepared && note.is_none() {
                // 未授权时段先问机制：环境不允许就如实降级，我们写错了就拒绝执行（见 refuse_when_broken）。
                if let Some(outcome) =
                    refuse_when_broken(&*self.texts, confine::verify(fence, command))
                {
                    return outcome;
                }
            }
            (prepared, note)
        };
        // 其它平台没有容器围栏，也就没有"要先授权"这一步。
        #[cfg(not(windows))]
        let (prepared, unfenced_note): (bool, Option<String>) = (false, None);
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
        for (k, v) in confine::fence_env(fence, command) {
            cmd.env(k, v);
        }
        // 该模块隐私字段的注入项：只走环境，不进命令行（值也不进提示词 / 转录 / 日志）。
        for (k, v) in env {
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
        let mut outcome = self.assemble(out, err, timed_out, code);
        // 用户裁决"本轮无围栏跑一次"：这次执行在回执里**如实标为无围栏**（模型与用户都看得到）。
        if let Some(note) = unfenced_note {
            outcome.output.push('\n');
            outcome.output.push_str(&note);
        }
        outcome
    }
}

/// 目的：必要落点授不上时的处置结论：按用户裁决**无围栏跑一次**，或**不执行**（fail-closed）。
#[cfg(windows)]
enum FenceGo {
    /// 无围栏跑这一次（回执里如实标注：这次没有容器那层强制）。
    Unfenced { note: String },
    /// 不执行：回执已经写清了为什么（问不到人、拒了、或者这一环连选项都构不出）。
    Refused(ToolOutcome),
}

/// 目的：一个**必要**落点授不上时的处置：**不许降级**——问用户；拿不到回答就按这一问声明的默认项 /
///   fail-closed 收场（回执如实说清是"用户拒绝"还是"没人答"）。
/// 参数：`ask` = 这一趟的提问端口（`None` = 这一趟没有可回答的前端）；`blocked` = 哪一环、哪个目录、缺什么前提。
/// 返回：`Unfenced`（照"无围栏跑一次"办；回执里如实标注）或 `Refused`（不执行）。
/// 约束：**构不出可用选项**（除"放弃"外没有一条真能执行的）时**不发起裁决**，
///   改为停掉这个会话 + 落一条警告（契约禁止置灰，见 docs/session/session-model.md 的「请用户裁决」）。
#[cfg(windows)]
impl ProcTools {
    fn fence_blocked(
        &self,
        ask: Option<&dyn crate::kernel::ports::AskUser>,
        fence: &FenceSpec,
        blocked: &crate::kernel::domain::fence::FenceBlocked,
    ) -> FenceGo {
        use crate::kernel::domain::fence::{fence_ask, OPT_FENCE_UNFENCED};
        let why = blocked.line();
        eprintln!("[围栏] 必要落点授不上（{}）：本次不许按无围栏跑", why);
        // 无围栏跑一次真能不能跑起来：判据是这次命令的起点在不在（domain 不读盘，所以在这里读）。
        let unfenced_possible = !fence.cwd.as_os_str().is_empty() && fence.cwd.is_dir();
        let Some(request) = fence_ask(blocked, &fence.agent, unfenced_possible) else {
            // 构不出可用选项：不发起裁决，改为停掉这个会话 + 落一条警告。
            match ask {
                Some(ask) => ask.halt(&why),
                None => eprintln!("[围栏] 这一趟没有可回答的前端：不停会话，只如实拒绝这次调用"),
            }
            return FenceGo::Refused(
                self.fence_refusal(blocked, &crate::kernel::api::AskOutcome::NoOptions),
            );
        };
        // 统一入口：有前端就问它，没有前端就按这一问声明的默认项收场——发起方只看"照哪个选项办"。
        let outcome = crate::kernel::ports::ask_user(ask, &request);
        match outcome.decided() {
            // 用户按卡选了（或没人答时的默认项是）"本轮无围栏跑一次"：回执里如实标注这次没有容器那层强制。
            Some(OPT_FENCE_UNFENCED) => FenceGo::Unfenced {
                note: self.fence_note(blocked),
            },
            // 拒绝 / 按放弃收场 / 没人答 / 停会话解成拒绝：不执行。
            _ => FenceGo::Refused(self.fence_refusal(blocked, &outcome)),
        }
    }

    /// 目的：不执行时的回执——哪一环、哪个目录、缺什么前提、怎么补（文案来自提示词册，它随工具结果进模型上下文）。
    /// 参数：`outcome` 是这一问的如实收场；用户拒绝与"没人答"分别追加一句，模型看得出差别。
    fn fence_refusal(
        &self,
        blocked: &crate::kernel::domain::fence::FenceBlocked,
        outcome: &crate::kernel::api::AskOutcome,
    ) -> ToolOutcome {
        use crate::kernel::api::AskOutcome;
        let path = where_text(blocked);
        let part = blocked.part.label().to_string();
        let mut output = self
            .texts
            .fence_blocked(&part, &path, &blocked.why, blocked.part.fix());
        let extra = match outcome {
            // 用户答了（选了"放弃"，或卡上的别的选项）：如实说用户没有放行这次调用。
            AskOutcome::Chosen(_) => Some(self.texts.denied_by_user()),
            // 没人答、按声明默认项收场：如实说这一下**不是用户答的**、按哪个选项办的。
            AskOutcome::Defaulted(id) => Some(self.texts.no_answerer_defaulted(&part, &path, id)),
            // 没人答、也没声明默认项：如实说"没人答"，别让模型以为用户拒了。
            AskOutcome::NoAnswer => Some(self.texts.no_answerer_refused(&part, &path)),
            // 用户按了停止（整队作废 = 拒绝）/ 构不出可用选项（端口已停会话 + 落警告）。
            AskOutcome::Stopped | AskOutcome::NoOptions => None,
        };
        if let Some(line) = extra {
            output.push('\n');
            output.push_str(&line);
        }
        ToolOutcome { ok: false, output }
    }

    /// 目的：无围栏跑一次时回执里的如实标注（模型下一轮看得到"这次没有容器那层强制"）。
    fn fence_note(&self, blocked: &crate::kernel::domain::fence::FenceBlocked) -> String {
        self.texts
            .fence_unfenced(blocked.part.label(), &where_text(blocked), &blocked.why)
    }
}

/// 目的：这一环授不上的**哪个目录**：没有具体目录的一环（容器身份 / 授权台账）如实说没有。
#[cfg(windows)]
fn where_text(blocked: &crate::kernel::domain::fence::FenceBlocked) -> String {
    if blocked.path.as_os_str().is_empty() {
        "没有具体目录".to_string()
    } else {
        blocked.path.to_string_lossy().into_owned()
    }
}

/// 目的：未授权时段遇到机制自检结论时的放行规矩——**只有本机装不上能降级**；
///   自检已确认机制有效却仍装不上 = 我们写错了（按无围栏跑等于用户以为有围栏、实际什么都没有），
///   所以拒绝执行（命令不落进程），回执用提示词册里的固定说法。
#[cfg(windows)]
pub(crate) fn refuse_when_broken(
    texts: &dyn ProcessTexts,
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
                output: texts.fence_failed(),
            })
        }
        confine::FenceVerdict::Enforced | confine::FenceVerdict::EnvUnavailable(_) => None,
    }
}

/// 目的：杀掉整棵进程树——工具进程 fork 出来的子孙一并收掉（不留孤儿）。
pub(crate) fn kill_tree(child: &mut std::process::Child) {
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
            output.push_str(&self.texts.stderr_header());
            output.push('\n');
            output.push_str(&err);
        }
        if timed_out {
            output.push('\n');
            output.push_str(&self.texts.timeout());
        }
        // 围栏没装上：守门进程用固定退出码报明（命令没被执行），这里如实告诉模型。
        if code == Some(confine::FENCE_FAILED) {
            ok = false;
            output.push('\n');
            output.push_str(&self.texts.fence_failed());
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
        let tail = self
            .texts
            .truncated(&count.to_string(), &self.max_output_chars.to_string());
        format!("{}\n{}", head, tail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::api::FenceSpec;
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
        std::env::remove_var("SOLOMNI_PROBE_SECRET");
    }

    /// 回执里的标记文案必须来自提示词册（它们随 [工具结果] 进模型上下文，所以不能在代码里另写一份）。
    #[test]
    fn receipt_markers_come_from_the_prompt_book() {
        use crate::capabilities::prompt::api::Prompt;
        use crate::capabilities::prompt::ports::PromptSource;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prompts = crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(
            root.join("prompts"),
        )
        .load()
        .expect("内置提示词册必须合法");
        let texts = prompts.tools();
        let tools = ProcTools::new(
            PathBuf::from("solomni"),
            std::sync::Arc::new(crate::capabilities::tools::detail::PromptProcessTexts::new(
                std::sync::Arc::clone(&texts),
            )),
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

    /// 本机可用的 node（没有就如实跳过需要它的用例）。解释器基线只在 Windows 的容器围栏上需要。
    #[cfg(windows)]
    fn node() -> Option<&'static str> {
        let ok = std::process::Command::new("node")
            .arg("-v")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if ok {
            Some("node")
        } else {
            None
        }
    }

    fn prompt_texts() -> std::sync::Arc<dyn crate::kernel::ports::ProcessTexts> {
        use crate::capabilities::prompt::api::Prompt;
        use crate::capabilities::prompt::ports::PromptSource;
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let prompts = crate::capabilities::prompt::detail::yaml_prompts::YamlPrompts::new(
            root.join("prompts"),
        )
        .load()
        .expect("内置提示词册必须合法");
        std::sync::Arc::new(crate::capabilities::tools::detail::PromptProcessTexts::new(
            prompts.tools(),
        ))
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
        let out = tools.run(
            &spec_for(&dir),
            &format!("{} echo_env.py", py),
            "{}",
            &[],
            None,
        );
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

    /// 解释器基线要在**真进程**上成立：`node <文件>` 的工具进程拿到 `NODE_OPTIONS`（容器里 node 靠它跳过
    /// realpath，否则脚本执行前就 EPERM 死）；别的解释器看不到它——注入面只覆盖真正需要它的那一种命令。
    #[cfg(windows)]
    #[test]
    fn real_node_tool_process_carries_the_realpath_skip() {
        let Some(exe) = built_exe() else {
            eprintln!(
                "[探针] 未找到已构建的 solomni 可执行文件（先 cargo build），跳过解释器基线契约"
            );
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
        let out = tools.run(&spec_for(&dir), command, "{}", &[], None);
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
            prompt_texts(),
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
        use crate::kernel::domain::fence::{FencePart, OPT_FENCE_ABORT, OPT_FENCE_UNFENCED};
        if !fence_live() {
            eprintln!(
                "[探针] 未开启真机围栏测试：本探针要写本机权限项，已跳过；要真跑加 --fence-live"
            );
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
        // ① 用户选"本轮无围栏跑一次"：命令真跑，回执**如实标为无围栏**。
        let ask = RecordingAsk::new(Some(OPT_FENCE_UNFENCED));
        let out = tools.run(&spec, &command, "{}", &[], Some(&ask));
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
        let out = tools.run(&spec, &command, "{}", &[], Some(&ask));
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
        let out = tools.run(&spec, &command, "{}", &[], None);
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

    /// 守门进程起不来：如实回执「启动失败」，不 panic、不假装跑过。
    #[test]
    fn missing_launcher_binary_is_reported_honestly() {
        let dir = crate::tests::scratch("proc-tools-missing-exe");
        let tools = real_runner(PathBuf::from("definitely-not-here-solomni"), &dir, 5);
        let out = tools.run(&spec_for(&dir), "echo hi", "{}", &[], None);
        assert!(!out.ok);
        assert!(out.output.contains("工具进程启动失败"), "{}", out.output);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
