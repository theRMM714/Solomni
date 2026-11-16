//! 目的：启动相命令的用例——操作数解析、回执分支与退出码，以及真实系统维护能力的装配面。
//! 管：boot::run 的每一条命令与每种失败（人读 / --json）、System 的合成替身、真实装配的一次冒烟。
//! 不管：命令表本身（commands.rs）、各能力的内部规则（各能力自己的用例）、入口分派（entry 的用例）。
//! 联动：src/presentation/cli/boot.rs 与 src/capabilities/system/{api,service}.rs；命令表见 systools/commands.yaml。

use serde_json::{Value, json};

use crate::capabilities::builds::api::Built;
use crate::capabilities::packages::api::BaseFacts;
use crate::capabilities::storage::api::{CleanOutcome, GcAction, GcReport};
use crate::capabilities::system::api::{
    BuildRequest, EnsureRequest, Fault, FenceFacts, GcRequest, ResetRequest, System, WorkGcRequest,
    toolchain_json, toolchain_lines,
};
use crate::capabilities::workspace::api::GcReport as WorkspaceGc;
use crate::kernel::api::JobState;
use crate::presentation::cli::boot;

/// 目的：合成一份系统维护面——每条命令要的分支由字段决定，用例不碰磁盘也不起进程。
/// 约束：转录中心那条路径（`presentation::cli::run_boot`）也用它，免得再写一份替身。
pub(crate) struct Fake {
    /// 目的：让哪条方法失败（"gc" / "work_gc" / "build" / "ensure" / "reset" / "storage" / "provision"）。
    fault: Option<&'static str>,
    /// 目的：gc 的返回形状；None = 同步报告。
    gc: Option<CleanOutcome>,
    /// 目的：下一次轮询看到的状态快照。
    job: JobState,
    /// 目的：构建结果；None = 失败。
    build: Option<Built>,
    /// 目的：工作区回收的报告。
    work: Option<WorkspaceGc>,
}

impl Default for Fake {
    fn default() -> Self {
        Fake {
            fault: None,
            gc: None,
            job: JobState::default(),
            build: Some(Built {
                module: "toolbox".to_string(),
                target: "host".to_string(),
                skipped: false,
                output: "ok\n".to_string(),
            }),
            work: Some(WorkspaceGc::default()),
        }
    }
}

impl Fake {
    /// 目的：让指定方法失败的同款替身——别的模块（转录中心那条路径）不必知道字段布局。
    pub(crate) fn failing(what: &'static str) -> Self {
        Fake {
            fault: Some(what),
            ..Fake::default()
        }
    }

    fn maybe<T>(&self, who: &str, ok: T) -> Result<T, Fault> {
        match self.fault {
            Some(f) if f == who => Err(Fault::failed(format!("{} 失败", who))),
            _ => Ok(ok),
        }
    }
    fn usage<T>(&self, who: &str) -> Result<T, Fault> {
        match self.fault {
            Some(f) if f == who => Err(Fault::usage("先给参数")),
            _ => Err(Fault::failed("unreachable")),
        }
    }
}

impl System for Fake {
    fn doctor(&self) -> Value {
        json!({ "platform": "test" })
    }
    fn storage(&self) -> Result<Value, Fault> {
        match self.fault {
            Some("storage") => Err(Fault::failed("盘面读不到")),
            _ => Ok(
                json!({ "categories": [ { "id": "logs", "label": "日志", "bytes": 10, "items": [1, 2] } ] }),
            ),
        }
    }
    fn gc(&self, _req: GcRequest) -> Result<CleanOutcome, Fault> {
        if self.fault == Some("gc") {
            return self.usage("gc");
        }
        Ok(self.gc.clone().unwrap_or_else(|| {
            CleanOutcome::Report(GcReport {
                applied: true,
                before_bytes: 100,
                after_bytes: 40,
                freed_bytes: 60,
                over_cap: false,
                actions: vec![GcAction {
                    category: "logs".to_string(),
                    id: String::new(),
                    label: "旧日志".to_string(),
                    bytes: 60,
                    reason: "超期".to_string(),
                    applied: true,
                    error: None,
                }],
                skipped: Vec::new(),
            })
        }))
    }
    fn provision(&self) -> Result<Value, Fault> {
        self.maybe("provision", json!({ "ok": true }))
    }
    fn ensure(&self, _req: EnsureRequest, on_line: &mut dyn FnMut(&str)) -> Result<(), Fault> {
        if self.fault == Some("ensure") {
            return self.usage("ensure");
        }
        on_line("装好 python");
        Ok(())
    }
    fn reset(&self, _req: ResetRequest, on_line: &mut dyn FnMut(&str)) -> Result<GcReport, Fault> {
        if self.fault == Some("reset") {
            return self.usage("reset");
        }
        on_line("已还原");
        Ok(GcReport::default())
    }
    fn work_gc(&self, _req: WorkGcRequest) -> Result<WorkspaceGc, Fault> {
        if self.fault == Some("work_gc") {
            return self.usage("work_gc");
        }
        match &self.work {
            Some(w) => Ok(w.clone()),
            None => Err(Fault::failed("没有这个工作")),
        }
    }
    fn build(&self, _req: BuildRequest) -> Result<Built, Fault> {
        if self.fault == Some("build") {
            return self.usage("build");
        }
        match &self.build {
            Some(b) => Ok(b.clone()),
            None => Err(Fault::failed("模块不存在")),
        }
    }
    fn job(&self) -> JobState {
        self.job.clone()
    }
    fn setup(
        &self,
        mode: crate::capabilities::system::api::SetupMode,
        on_line: &mut dyn FnMut(&str),
    ) -> Result<Value, Fault> {
        if self.fault == Some("setup") {
            return Err(Fault::failed("便携维护失败"));
        }
        on_line(&format!("{} 完成", mode.id()));
        Ok(json!({
            "platform": "test",
            "root": "/tmp",
            "mode": mode.id(),
            "lines": [format!("{} 完成", mode.id())],
            "path": { "bin": "bin", "has_bin": true, "shim_present": false, "shim_current": false },
            "fence": { "supported": true, "ready": false, "missing": ["产品根读属性"], "error": null },
        }))
    }
}

fn faces(words: &[&str]) -> Vec<String> {
    words.iter().map(|w| w.to_string()).collect()
}

#[test]
fn read_only_commands_print_receipts() {
    let fake = Fake::default();
    for json in [false, true] {
        assert_eq!(boot::run("doctor", &[], &fake, json), 0);
        assert_eq!(boot::run("storage", &[], &fake, json), 0);
        assert_eq!(boot::run("provision", &[], &fake, json), 0);
    }
}

#[test]
fn provision_ensure_and_reset_report_both_ways() {
    let fake = Fake::default();
    for json in [false, true] {
        assert_eq!(
            boot::run(
                "provision",
                &faces(&["--ensure=toolchain", "--yes"]),
                &fake,
                json
            ),
            0
        );
        assert_eq!(
            boot::run("provision", &faces(&["--ensure", "--yes"]), &fake, json),
            0
        );
        assert_eq!(
            boot::run(
                "provision",
                &faces(&["--reset", "dev", "--yes"]),
                &fake,
                json
            ),
            0
        );
    }
}

#[test]
fn gc_reports_or_starts_a_job() {
    let fake = Fake::default();
    for json in [false, true] {
        assert_eq!(boot::run("gc", &faces(&["--yes"]), &fake, json), 0);
    }
    // 后台作业：第一眼还在跑，轮询一次就结束（覆盖增量打行与退出码分支）。
    let started = Fake {
        gc: Some(CleanOutcome::Started(JobState {
            running: true,
            seq: 1,
            lines: vec!["开始".to_string()],
            error: None,
        })),
        job: JobState {
            running: false,
            seq: 2,
            lines: vec!["开始".to_string(), "完成".to_string()],
            error: None,
        },
        ..Fake::default()
    };
    assert_eq!(boot::run("gc", &faces(&["--yes"]), &started, false), 0);
    let failed = Fake {
        gc: Some(CleanOutcome::Started(JobState {
            running: true,
            seq: 1,
            lines: Vec::new(),
            error: None,
        })),
        job: JobState {
            running: false,
            seq: 2,
            lines: Vec::new(),
            error: Some("磁盘满了".to_string()),
        },
        ..Fake::default()
    };
    assert_eq!(boot::run("gc", &faces(&["--yes"]), &failed, false), 1);
    assert_eq!(boot::run("gc", &faces(&["--yes"]), &failed, true), 1);
}

/// 目的：便携维护的三条分支——省略模式只报状态、写模式交给能力、未知模式退 2、能力失败按自己的码退。
#[test]
fn setup_reports_status_and_refuses_unknown_modes() {
    let fake = Fake::default();
    for json in [false, true] {
        assert_eq!(boot::run("setup", &[], &fake, json), 0);
        assert_eq!(boot::run("setup", &faces(&["status"]), &fake, json), 0);
        assert_eq!(boot::run("setup", &faces(&["path"]), &fake, json), 0);
        assert_eq!(boot::run("setup", &faces(&["bogus"]), &fake, json), 2);
    }
    let broken = Fake {
        fault: Some("setup"),
        ..Fake::default()
    };
    assert_eq!(boot::run("setup", &faces(&["path"]), &broken, false), 1);
    assert_eq!(boot::run("setup", &faces(&["path"]), &broken, true), 1);
}

#[test]
fn gc_without_confirmation_previews() {
    let fake = Fake::default();
    assert_eq!(
        boot::run("gc", &faces(&["--category=logs"]), &fake, false),
        2
    );
    assert_eq!(
        boot::run("gc", &faces(&["--category", "logs"]), &fake, true),
        2
    );
    // 预览里查不到这个类别：仍然只是「还差一次确认」，照样给 2。
    assert_eq!(
        boot::run("gc", &faces(&["--category=nope"]), &fake, false),
        2
    );
    // 状态取不到时预览直接失败。
    let blind = Fake {
        fault: Some("storage"),
        ..Fake::default()
    };
    assert_eq!(
        boot::run("gc", &faces(&["--category=logs"]), &blind, false),
        1
    );
    assert_eq!(
        boot::run("gc", &faces(&["--category=logs"]), &blind, true),
        1
    );
}

#[test]
fn work_gc_covers_preview_and_apply() {
    let applied = WorkspaceGc {
        applied: true,
        objects: 3,
        bytes: 30,
        reachable_objects: 2,
        reachable_bytes: 20,
        unreachable_objects: 1,
        unreachable_bytes: 10,
        ignored_files: 1,
        removed_objects: 1,
        removed_bytes: 10,
        removed_ignored: 1,
        ..WorkspaceGc::default()
    };
    let dry = Fake {
        work: Some(WorkspaceGc::default()),
        ..Fake::default()
    };
    for json in [false, true] {
        assert_eq!(boot::run("work-gc", &faces(&["dev"]), &dry, json), 0);
        assert_eq!(
            boot::run("work-gc", &faces(&["dev", "--yes"]), &dry, json),
            0
        );
    }
    let fake = Fake {
        work: Some(applied),
        ..Fake::default()
    };
    assert_eq!(
        boot::run("work-gc", &faces(&["dev", "--yes"]), &fake, false),
        0
    );
    assert_eq!(boot::run("work-gc", &faces(&["dev"]), &fake, true), 0);
    // 缺工作名 / 能力面拒绝。
    assert_eq!(boot::run("work-gc", &[], &fake, false), 2);
    let refused = Fake {
        fault: Some("work_gc"),
        ..Fake::default()
    };
    assert_eq!(
        boot::run("work-gc", &faces(&["dev", "--yes"]), &refused, false),
        2
    );
    assert_eq!(
        boot::run("work-gc", &faces(&["dev", "--yes"]), &refused, true),
        2
    );
}

#[test]
fn build_prints_output_and_rejects_bad_operands() {
    let fake = Fake::default();
    for json in [false, true] {
        assert_eq!(boot::run("build", &faces(&["toolbox"]), &fake, json), 0);
    }
    let skipped = Fake {
        build: Some(Built {
            module: "toolbox".to_string(),
            target: "host".to_string(),
            skipped: true,
            output: String::new(),
        }),
        ..Fake::default()
    };
    assert_eq!(boot::run("build", &faces(&["toolbox"]), &skipped, false), 0);
    // 缺模块名 / --guest 给了空值。
    assert_eq!(boot::run("build", &[], &fake, false), 2);
    assert_eq!(
        boot::run("build", &faces(&["toolbox", "--guest", ""]), &fake, false),
        2
    );
    // 能力面报错。
    assert_eq!(boot::run("build", &faces(&["toolbox"]), &skipped, true), 0);
    let missing = Fake {
        build: None,
        ..Fake::default()
    };
    assert_eq!(boot::run("build", &faces(&["nope"]), &missing, false), 1);
    assert_eq!(boot::run("build", &faces(&["nope"]), &missing, true), 1);
}

#[test]
fn failures_carry_their_own_exit_code() {
    let failing = Fake {
        fault: Some("storage"),
        ..Fake::default()
    };
    assert_eq!(boot::run("storage", &[], &failing, false), 1);
    assert_eq!(boot::run("storage", &[], &failing, true), 1);
    let no_provision = Fake {
        fault: Some("provision"),
        ..Fake::default()
    };
    assert_eq!(boot::run("provision", &[], &no_provision, false), 1);
    assert_eq!(boot::run("provision", &[], &no_provision, true), 1);
    let no_ensure = Fake {
        fault: Some("ensure"),
        ..Fake::default()
    };
    assert_eq!(
        boot::run(
            "provision",
            &faces(&["--ensure=toolchain"]),
            &no_ensure,
            true
        ),
        2
    );
    let no_reset = Fake {
        fault: Some("reset"),
        ..Fake::default()
    };
    assert_eq!(
        boot::run("provision", &faces(&["--reset", "dev"]), &no_reset, false),
        2
    );
    let no_gc = Fake {
        fault: Some("gc"),
        ..Fake::default()
    };
    assert_eq!(boot::run("gc", &faces(&["--yes"]), &no_gc, true), 2);
}

#[test]
fn unknown_operands_and_commands_are_refused() {
    let fake = Fake::default();
    assert_eq!(boot::run("doctor", &faces(&["--nope"]), &fake, false), 2);
    assert_eq!(boot::run("doctor", &faces(&["-x"]), &fake, false), 2);
    assert_eq!(boot::run("nope", &[], &fake, false), 1);
}

/// 目的：真实装配一次系统维护面（临时根）——启动相命令对着真能力也得给出退出码。
#[test]
fn real_assembly_serves_every_boot_command() {
    let root =
        std::env::temp_dir().join(format!("solomni-boot-{}-{}", std::process::id(), line!()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join(".home")).expect("建 .home");
    std::fs::create_dir_all(root.join("modules")).expect("建 modules");
    std::fs::write(
        root.join(".home").join("settings.yaml"),
        "sandbox:\n  policy:\n    network: false\n",
    )
    .expect("写设置");

    let service =
        crate::entry::assembly::system(&root, &BaseFacts::default()).expect("装配系统能力");
    // 直接走用例面：自检、盘点、状态、回收（预览 + 各分类）、工作区回收、构建。
    let _ = service.doctor();
    let _ = service.fence();
    let _ = service.storage();
    let _ = service.provision();
    let _ = service.gc(GcRequest::default());
    for kind in [
        "build-cache",
        "download-cache",
        "overlays",
        "logs",
        "compact",
        "guest-reset",
    ] {
        let _ = service.gc(GcRequest {
            kind: Some(kind.to_string()),
            guest: None,
            module: None,
            confirm: false,
        });
    }
    let _ = service.gc(GcRequest {
        kind: Some("nope".to_string()),
        guest: None,
        module: None,
        confirm: false,
    });
    let _ = service.work_gc(WorkGcRequest {
        work: "nope".to_string(),
        confirm: false,
    });
    let _ = service.build(BuildRequest {
        module: "nope".to_string(),
        guest: None,
    });
    let _ = service.job();
    let _ = service.doctor_with(FenceFacts {
        fs: false,
        net: false,
        tree: false,
        note: "用例".to_string(),
    });

    // 呈现层的每条命令也对着真能力跑一遍（不看文案，只看退出码）。
    for json in [false, true] {
        assert_eq!(boot::run("doctor", &[], &service, json), 0);
        assert_eq!(boot::run("storage", &[], &service, json), 0);
        assert_eq!(boot::run("provision", &[], &service, json), 0);
        assert_eq!(
            boot::run("gc", &faces(&["--category=logs"]), &service, json),
            2
        );
        assert_eq!(boot::run("work-gc", &[], &service, json), 2);
        assert_eq!(boot::run("build", &[], &service, json), 2);
        assert_eq!(boot::run("setup", &faces(&["status"]), &service, json), 0);
        assert_eq!(boot::run("setup", &faces(&["bogus"]), &service, json), 2);
    }
    boot::print_gc(&GcReport::default());
    boot::print_gc(&GcReport {
        applied: true,
        before_bytes: 1,
        actions: vec![GcAction {
            category: "logs".to_string(),
            id: String::new(),
            label: "旧日志".to_string(),
            bytes: 1,
            reason: "超期".to_string(),
            applied: true,
            error: Some("删不掉".to_string()),
        }],
        ..GcReport::default()
    });
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：装配核心之前就该回答的旗语——总表、单命令手册、版本，别的命令照常往下走。
#[test]
fn early_exit_answers_help_and_version_before_assembly() {
    let argv = |v: &[&str]| crate::presentation::cli::argv::parse(&faces(v)).expect("可解析");
    assert_eq!(crate::early_exit(&argv(&["--help"])), Some(0));
    assert_eq!(crate::early_exit(&argv(&["--all", "--help"])), Some(0));
    assert_eq!(crate::early_exit(&argv(&["--gc", "--help"])), Some(0));
    assert_eq!(crate::early_exit(&argv(&["--version"])), Some(0));
    assert!(crate::early_exit(&argv(&["--gc", "--yes"])).is_none());
}

#[test]
fn toolchain_facts_render_for_both_audiences() {
    let mut facts = BaseFacts {
        active: vec![("node".to_string(), "22.11.0".to_string())],
        fallback: std::collections::BTreeMap::new(),
    };
    facts.fallback.insert(
        "python".to_string(),
        crate::capabilities::packages::api::HostFallback {
            path: std::path::PathBuf::from("/usr/bin/python3"),
            fenced: true,
            why: String::new(),
        },
    );
    facts.fallback.insert(
        "cc".to_string(),
        crate::capabilities::packages::api::HostFallback {
            path: std::path::PathBuf::from("/usr/bin/cc"),
            fenced: false,
            why: "没有写 ACL 的权限".to_string(),
        },
    );
    let lines = toolchain_lines(&facts);
    assert!(lines.iter().any(|l| l.contains("项目内：node 22.11.0")));
    assert!(lines.iter().any(|l| l.contains("项目内未安装：python、cc")));
    assert!(lines.iter().any(|l| l.contains("围栏可用")));
    assert!(lines.iter().any(|l| l.contains("围栏不可授权")));
    let json = toolchain_json(&facts);
    assert_eq!(json["missing"], json!(["python", "cc"]));
    assert_eq!(json["active"][0]["id"], json!("node"));
    // 一条都没有时只报「未安装」与「宿主也没有」。
    let empty = toolchain_lines(&BaseFacts::default());
    assert!(empty.iter().any(|l| l.contains("宿主 PATH 也没有")));
}

/// 目的：写操作的就地确认：转录中心里问一句，命令行上没人可问（照旧只预览）。
/// 约束：有会话在跑时不许写——连问都不问，只给预览（存储在实例里，另一套实例看不到它的作业）。
#[test]
fn writes_are_confirmed_in_place_and_refused_while_sessions_run() {
    /// 目的：按剧本作答的「人」：记下被问了几次，回答固定。
    struct Scripted {
        answer: bool,
        asked: std::cell::RefCell<Vec<String>>,
    }

    impl boot::Ask for Scripted {
        fn anyone(&self) -> bool {
            true
        }

        fn confirm(&mut self, question: &str) -> bool {
            self.asked.borrow_mut().push(question.to_string());
            self.answer
        }
    }

    let fake = Fake::default();
    let asked = |ask: &Scripted| ask.asked.borrow().len();

    // 点头：问一次，然后真的执行（--yes 由确认补上）。
    let mut yes = Scripted {
        answer: true,
        asked: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(boot::run_asked("gc", &[], &fake, false, &mut yes, true), 0);
    assert_eq!(asked(&yes), 1, "写操作要先问一句");

    // 摇头：只预览，退出码不再算错（人已经明确作答）。
    let mut no = Scripted {
        answer: false,
        asked: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(boot::run_asked("gc", &[], &fake, false, &mut no, true), 0);
    assert_eq!(asked(&no), 1);

    // 有会话在跑：不问、不写，只给预览。
    let mut idle = Scripted {
        answer: true,
        asked: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(
        boot::run_asked("gc", &[], &fake, false, &mut idle, false),
        0
    );
    assert_eq!(asked(&idle), 0, "不许写的时候连问都不问");

    // 只读命令不问。
    let mut read_only = Scripted {
        answer: false,
        asked: std::cell::RefCell::new(Vec::new()),
    };
    assert_eq!(
        boot::run_asked("doctor", &[], &fake, false, &mut read_only, true),
        0
    );
    assert_eq!(asked(&read_only), 0);
}

/// 目的：命令行入口的「没有人可问」：不认领任何人，也从不点头（缺 --yes 就照旧只预览）。
#[test]
fn never_ask_claims_nobody_and_never_agrees() {
    let mut ask = boot::NeverAsk;
    assert!(!boot::Ask::anyone(&ask));
    assert!(!boot::Ask::confirm(&mut ask, "确认？[y/N]"));
}

/// 目的：确认问话要点出这次动的是什么——存储类别、工作名、虚拟根、依赖补齐各说各的。
#[test]
fn confirm_questions_name_the_target() {
    /// 目的：记下问话、回答固定不点头的「人」。
    struct Recorder(std::cell::RefCell<Vec<String>>);

    impl boot::Ask for Recorder {
        fn anyone(&self) -> bool {
            true
        }

        fn confirm(&mut self, question: &str) -> bool {
            self.0.borrow_mut().push(question.to_string());
            false
        }
    }

    let fake = Fake::default();
    let ask = |command: &str, operands: &[String]| -> String {
        let mut rec = Recorder(std::cell::RefCell::new(Vec::new()));
        assert_eq!(
            boot::run_asked(command, operands, &fake, true, &mut rec, true),
            0,
            "摇头只预览，退出码不再算错"
        );
        let asked = rec.0.borrow();
        assert_eq!(asked.len(), 1, "写操作问一次");
        asked[0].clone()
    };

    assert!(ask("gc", &faces(&["--category=logs"])).contains("存储类别 logs"));
    assert!(ask("gc", &[]).contains("按策略回收存储"));
    assert!(ask("work-gc", &faces(&["dev"])).contains("工作 dev"));
    assert!(ask("work-gc", &[]).contains("确认回收？"));
    assert!(ask("provision", &faces(&["--reset", "dev"])).contains("虚拟根 dev"));
    assert!(ask("provision", &faces(&["--ensure"])).contains("下载并补齐依赖"));
}

/// 目的：`--setup status` 的三行排版两档都成型——字段齐时照实说，缺字段按「没有」说。
#[test]
fn setup_report_lines_read_every_field() {
    let full = json!({
        "platform": "windows",
        "root": "D:/root",
        "path": { "bin": "D:/root/bin", "has_bin": true, "shim_present": true, "shim_current": true },
        "fence": { "supported": true, "ready": true, "missing": [], "error": null },
    });
    let lines = boot::setup_lines(&full);
    assert_eq!(lines.len(), 3);
    assert!(lines[0].contains("windows") && lines[0].contains("D:/root"));
    assert!(lines[1].contains("D:/root/bin") && lines[1].contains("shim：在"));
    assert!(lines[1].contains("shim 最新：是"));
    assert!(lines[2].contains("已就绪") && lines[2].contains("缺 0 处"));

    let missing = json!({
        "path": { "shim_present": false },
        "fence": { "ready": false, "missing": ["产品根读属性", "工具链落点"] },
    });
    let lines = boot::setup_lines(&missing);
    assert!(lines[1].contains("目录在：否") && lines[1].contains("shim：没写"));
    assert!(lines[2].contains("未就绪") && lines[2].contains("缺 2 处"));
}
