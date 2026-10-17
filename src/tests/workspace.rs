//! 工作区能力测试：沙箱寻址与共享区版本化。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn sandbox_resolve_accepts_only_absolute_paths_inside_roots() {
    use crate::capabilities::workspace::api::Place;
    let sb = test_sandbox("a1", &["data"]);
    // 绝对且在根内 → 通过（返回归一化后的绝对路径）
    let (place, path) = sb
        .resolve(&p(&["demo", "work", "notes", "a.txt"]))
        .expect("共享区可达");
    assert_eq!(place, Place::Shared);
    assert_eq!(path, abs(&["demo", "work", "notes", "a.txt"]));
    let (place, path) = sb
        .resolve(&p(&["demo", "a1", "b.txt"]))
        .expect("私有沙箱可达");
    assert_eq!(place, Place::Private);
    assert_eq!(path, abs(&["demo", "a1", "b.txt"]));
    let (place, path) = sb
        .resolve(&p(&["mods", "data", "c.txt"]))
        .expect("成员模块目录可达");
    assert_eq!(place, Place::Module("data".to_string()));
    assert_eq!(path, abs(&["mods", "data", "c.txt"]));
    // 允许的根就是这三处：错误文案要把它们列全
    let roots_line = s(&["demo", "work"]);
    // 拒绝：越界 / 非绝对路径（相对、裸文件名、带冒号前缀的伪路径）/ .. 与 . 段 / 空段 / 空
    let bad: Vec<String> = vec![
        p(&["outside", "x.txt"]),
        "a.txt".to_string(),
        "demo/work/a.txt".to_string(),
        "work:/a.txt".to_string(),
        "sandbox:/b.txt".to_string(),
        "module:data:/c.txt".to_string(),
        format!(
            "{}{}..{}x.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!(
            "{}{}a{}..{}b.txt",
            p(&["demo", "work"]),
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR,
            std::path::MAIN_SEPARATOR
        ),
        format!("{}//a.txt", p(&["demo", "work"])),
        "   ".to_string(),
    ];
    for b in &bad {
        let err = sb.resolve(b).unwrap_err();
        assert!(
            err.contains(&roots_line),
            "错误要把允许的真实根列全（{}）：{}",
            b,
            err
        );
    }
}

/// 目的：撤销一次准备只删**空**目录——建会话失败时不留残骸，也不碰用户的东西。
#[test]
pub(crate) fn workspace_discard_removes_only_empty_dirs() {
    use crate::capabilities::workspace::detail::FsWorkspace;
    let root = crate::tests::scratch("ws-discard");
    let ws = FsWorkspace::new(root.join("session"));
    // 准备出来 → 撤销：三个空目录都该消失。
    ws.prepare("w", &["a".to_string()]).expect("准备");
    ws.discard("w", &["a".to_string()]).expect("撤销");
    assert!(!root.join("session").join("w").exists(), "空目录要删掉");
    // 里面有东西 → 撤销什么都不该删。
    ws.prepare("w2", &["a".to_string()]).expect("准备");
    let keep = root.join("session").join("w2").join("a").join("notes.txt");
    std::fs::write(&keep, b"user data").expect("写文件");
    ws.discard("w2", &["a".to_string()]).expect("撤销");
    assert!(keep.is_file(), "有内容的目录绝不能删");
    let _ = std::fs::remove_dir_all(&root);
}
#[test]
pub(crate) fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::capabilities::workspace::api::Sandboxes {
        shared: abs(&["w", "work"]),
        list: vec![test_sandbox("甲", &["a"])],
        codec: None,
    };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
}

/// 目的：虚拟机档"能不能选"只看**机器能力**，与会话级选哪个虚拟根无关——Web 换档死锁的回归锚。
/// 约束：虚拟根只在保存 /「开始」时校验（构造承载判据时合并进去）。
#[test]
pub(crate) fn vm_availability_is_machine_only_and_base_is_session_level() {
    use crate::capabilities::vm::domain::exec::host_verified;
    use crate::tests::doubles::{core_with_gateway_probe, FixedProbe};
    // 机器能力探测替身按声明回答；虚拟根由 test_guest 造在 .vm/guest/<名> 下。
    let base = crate::tests::base_dir("vm-avail-base");
    // 机器能力齐：有虚拟机监视器，PATH 里有 QEMU 与 virtiofsd（guest 项按已验平台数据表）。
    let mut probe = FixedProbe {
        hypervisor: true,
        ..Default::default()
    };
    probe.exes.push("qemu-system-x86_64".to_string());
    probe.exes.push("virtiofsd".to_string());
    probe.dirs.push(base.clone());
    probe.files.push(base.join("vmlinuz"));
    // 就绪检测问的是"虚拟根在不在场"：替身按声明的根作答（子树算在场，文件按声明）。
    let guest_dir = crate::tests::root_dir()
        .join(".vm")
        .join("guest")
        .join("vm-avail");
    probe.guests.push(guest_dir.clone());
    // 虚拟根现在是"磁盘镜像 + 可选内核"：镜像必须在场（会话的可写层叠在它上面）。
    probe.files.push(guest_dir.join("rootfs.ext4"));
    probe.files.push(guest_dir.join("vmlinuz"));
    let mut core = core_with_gateway_probe(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        Arc::new(probe),
    );
    let choices = core.tier_choices();
    let expected = host_verified(std::env::consts::OS);
    assert_eq!(
        choices.vm_available, expected,
        "vm_available 只看机器能力：{:?}",
        choices.vm_requirements
    );
    assert!(
        !choices.vm_requirements.iter().any(|r| r.id == "base"),
        "机器能力清单不含会话级虚拟根：{:?}",
        choices.vm_requirements
    );
    if !expected {
        return;
    }
    // 建一条本机档会话（没选虚拟根）：会话配置仍说虚拟机档可选——虚拟根只在保存/开始时校验。
    let spec = crate::tests::single_work("wl", &["a"]);
    core.create_work(spec).expect("建本机档会话");
    let cfg = core.session_config("wl").expect("配置视图");
    assert!(cfg.guest.is_none(), "新会话没有选虚拟根");
    assert!(
        cfg.vm_available,
        "机器能力齐就该能选：{:?}",
        cfg.vm_requirements
    );
    assert!(
        !cfg.vm_requirements.iter().any(|r| r.id == "base"),
        "配置视图的机器能力清单也不含虚拟根：{:?}",
        cfg.vm_requirements
    );
    // 建虚拟机档：没选虚拟根 → 如实拒绝；选了在场的一格 → 建得起来（虚拟根是会话级，不是机器能力）。
    let mut no_guest = crate::tests::single_work("wv", &["a"]);
    no_guest.tier = Tier::Vm;
    let err = core
        .create_work(no_guest)
        .expect_err("虚拟机档没选虚拟根要如实拒绝");
    assert!(err.contains("虚拟根"), "{}", err);
    let guest = crate::tests::test_guest("vm-avail");
    let mut with_guest = crate::tests::single_work("wv2", &["a"]);
    with_guest.tier = Tier::Vm;
    with_guest.guest = Some(guest.clone());
    core.create_work(with_guest)
        .expect("机器能力齐 + 虚拟根在场就该建得起来");
    let _ = std::fs::remove_dir_all(&base);
}

/// 虚拟机档的承载校验：前置条件不具备时，**创建与编辑都如实拒绝**（用户环境问题，不是选型问题）。
/// 与界面上的"能不能选"同源（`exec::tier_readiness`），两处不会各说各话。
#[test]
pub(crate) fn vm_tier_is_refused_when_the_machine_cannot_carry_it() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let gateway: Arc<dyn ChatGateway + Send + Sync> =
        Arc::new(gw(member.clone(), vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Vm), Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    // 创建路径的档位来自**用户的选择**（WorkSpec.tier；基础根留空）：成立与否随本机而定，
    // 这里钉的是**接线**——机器承载不了就必须拒绝，且什么都不留下。
    let default_vm = ExecSpec {
        tier: Tier::Vm,
        ..ExecSpec::default()
    };
    let mut vm_spec = work("vm-default", WorkMode::Single, &["a"]);
    vm_spec.tier = Tier::Vm;
    let opened = core.create_work(vm_spec);
    if exec::tier_readiness(
        &crate::tests::root_dir(),
        &default_vm,
        None,
        None,
        &crate::kernel::detail::HostProbeAdapter,
    )
    .ready()
    {
        opened.expect("本机能承载虚拟机档时不该拒绝");
    } else {
        let err = opened.expect_err("本机承载不了虚拟机档就不许建");
        assert!(err.contains("虚拟机档现在不可用"), "{}", err);
        assert!(!core.session_exists("vm-default"), "拒绝就该什么都不留下");
    }

    // 编辑路径：基础根由用户给定（这里给一个不存在的），所以这一条不随机器变——必须拒绝、档位保持原样。
    let gateway2: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(member, vec!["[]".into()]));
    let llm2 = test_llm(
        Arc::clone(&gateway2),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core2 = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::new(), Arc::clone(&llm2)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        llm2,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    let sid = core2
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let err = core2
        .edit_session(
            &sid,
            SessionEdit {
                agents: vec![ConfigAgent {
                    name: "a".to_string(),
                    modules: vec!["a".to_string()],
                    model: String::new(),
                    permissions: None,
                }],
                tier: "vm".to_string(),
                guest: Some("definitely-not-a-real-guest".to_string()),
                pins: BTreeMap::new(),
                net: false,
            },
        )
        .expect_err("虚拟根不在场就不许改入虚拟机档");
    assert!(
        err.contains("虚拟机档现在不可用") && err.contains("虚拟根"),
        "{}",
        err
    );
    let cfg = core2.session_config(&sid).unwrap();
    assert_eq!(cfg.tier, "host", "被拒后档位保持原样");
    // 界面读的是**已保存的**配置（用户没提交的 base 输入后端看不到），两个字段必须自洽：
    // 可用时没有理由、不可用时必须有理由——界面据此决定禁用与说明。
    assert_eq!(
        cfg.vm_available,
        cfg.vm_unavailable_reason.is_empty(),
        "能不能选与为什么不能选必须说同一件事"
    );
}

#[test]
pub(crate) fn runtime_report_is_tier_aware() {
    let cores = |pkgs: Arc<InMemoryPackages>, modules: Vec<Module>| {
        core_with_pkgs(
            modules,
            gw(BTreeMap::new(), vec!["[]".into()]),
            Arc::new(SilentRunner),
            Arc::new(FakeCatalog::new(vec!["m".to_string()])),
            Arc::new(InMemoryHistory::new()),
            Arc::new(InMemorySysIo::new()),
            pkgs,
        )
    };
    let core = cores(
        Arc::new(InMemoryPackages::empty()),
        vec![module_with_runtimes("a", &["python"])],
    );
    let host = core.runtime_report(Tier::Host);
    assert_eq!(host.tier, "host");
    assert_eq!(host.declared.get("a"), Some(&vec!["python".to_string()]));
    assert_eq!(
        host.missing.get("a"),
        Some(&vec!["python".to_string()]),
        "档位无关的事实照实报"
    );
    assert!(host.available.is_empty());
    assert!(host.diagnoses.is_empty(), "本机档不做虚拟机档诊断");
    assert_eq!(core.runtime_report(Tier::Vm).diagnoses.len(), 1);
    // 包库里有包 = 缺失消失、诊断清空
    let with_pkg = cores(
        Arc::new(InMemoryPackages::with(&[
            "id: python\nversion: 3.12.4\nprefix: opt/rt/python3.12",
        ])),
        vec![module_with_runtimes("a", &["python"])],
    );
    let r = with_pkg.runtime_report(Tier::Vm);
    assert!(r.missing.is_empty(), "{:?}", r.missing);
    assert_eq!(r.available.get("python"), Some(&vec!["3.12.4".to_string()]));
    assert!(r.diagnoses.is_empty(), "{:?}", r.diagnoses);
}

#[test]
pub(crate) fn module_without_runtime_is_denied_with_reason() {
    // 虚拟机档 + 空包库：模块声明的运行包没人提供 → 改入那一刻如实拒绝（缺包不再降级）。
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"改用内置工具\"}".into(),
        ],
    );
    let mut mod_a = module_with_runtimes("a", &["python"]);
    mod_a
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    // 设置里的默认档位是**创建**时的档位来源：这里用本机档建（虚拟机档现在一律不可选），
    // 建好之后再把这条件裁成"已存在的虚拟机档会话"。
    let hist = Arc::new(InMemoryHistory::new());
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(member, vec!["[]".into()]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    let mut core = Conductor::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Host), Arc::clone(&llm)),
        test_history_of(Arc::clone(&hist)),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![mod_a]))),
        llm,
        test_tools_svc_with(
            Arc::clone(&runner) as Arc<dyn ProcessRunner>,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        Arc::new(crate::kernel::detail::HostProbeAdapter),
        Arc::new(crate::capabilities::residents::api::NoResidents),
        Arc::new(crate::capabilities::secrets::api::NoSecrets),
    );
    // 虚拟机档 + 空包库 + 基础根没声明运行能力：模块声明的 python 没人提供 →
    // **改入那一刻就拒绝**（不再降级：没有工具用户完不成工作），一个进程都不落。
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.force_tier(&sid, Tier::Vm);
    let guest = crate::tests::test_guest("module-without-runtime");
    let mut rebuild = edit_of(vec![("a", &["a"], "")]);
    rebuild.tier = "vm".to_string();
    rebuild.guest = Some(guest.clone());
    let refused = core.edit_session(&sid, rebuild).unwrap_err();
    assert!(
        refused.contains("python") && refused.contains("runtimes/"),
        "要说清缺哪个能力、怎么补：{}",
        refused
    );
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "拒绝时不落任何进程"
    );
    // 同一个模块在本机档照旧执行（本机档不装载运行包）。
    let mut member2 = BTreeMap::new();
    member2.insert(
        "a".to_string(),
        vec![
            TOOL_CALL.into(),
            "{\"type\":\"say\",\"text\":\"跑完了\"}".into(),
        ],
    );
    let mut mod_b = module_with_runtimes("a", &["python"]);
    mod_b
        .manifest
        .tools
        .insert("grep".to_string(), decl("python tools/grep.py"));
    let runner2 = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    let mut core2 = core_with_runner(
        vec![mod_b],
        gw(member2, vec!["[]".into()]),
        Arc::clone(&runner2),
    );
    let sid2 = core2
        .create_work(work("w2", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    with_live(|l| core2.single_say(&sid2, "干活", l)).unwrap();
    assert_eq!(
        runner2.calls.lock().expect("锁").len(),
        1,
        "本机档不受包库影响"
    );
}

// ---------- 配置视图：读、改、冻结 ----------

#[test]
pub(crate) fn session_config_reports_tier_missing_and_runtimes_dir() {
    let hist = Arc::new(InMemoryHistory::new());
    seed_session(
        &hist,
        "w",
        "single",
        vec![agent_meta("a", &["a"], Some("m"))],
        ExecSpec {
            tier: Tier::Vm,
            ..Default::default()
        },
    );
    let core = core_with_pkgs(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
    );
    let cfg = core.session_config("w").unwrap();
    assert_eq!(cfg.sid, "w");
    assert_eq!(cfg.mode, "single");
    assert!(!cfg.started, "没有内容的会话 = 还没开过");
    assert_eq!(cfg.tier, "vm");
    assert_eq!(cfg.agents[0].model, "m");
    assert_eq!(
        cfg.runtime.missing.get("a"),
        Some(&vec!["python".to_string()])
    );
    assert!(
        cfg.runtimes_dir.ends_with("/runtimes"),
        "{}",
        cfg.runtimes_dir
    );
    assert!(core.session_config("没有这个会话").is_err());
}

// ---------- 共享区版本化工作区（work_pull / work_commit / work_status） ----------

type WorkFace = Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync>;

/// 装配一个只用内存端口的工作区能力面：目录布局 + 版本库都是内存替身。
fn work_ws() -> (WorkFace, Arc<InMemoryWorkspace>, Arc<InMemoryWorkStore>) {
    let dirs = Arc::new(InMemoryWorkspace::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs.clone(), store.clone());
    (ws, dirs, store)
}

fn commit_req(
    paths: &[&str],
    deletes: &[&str],
    message: &str,
    time: i64,
) -> crate::capabilities::workspace::api::CommitRequest {
    crate::capabilities::workspace::api::CommitRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        deletes: deletes.iter().map(|s| s.to_string()).collect(),
        message: message.to_string(),
        time,
        line: 0,
    }
}

fn sandbox_of(ws: &WorkFace, work: &str, agent: &str) -> PathBuf {
    ws.roots(work, &[agent.to_string()])
        .unwrap()
        .agents
        .get(agent)
        .cloned()
        .unwrap()
}

fn shared_of(ws: &WorkFace, work: &str) -> PathBuf {
    ws.roots(work, &[]).unwrap().shared
}

/// 三方比较的核心承诺：a、b 同基线各改同一文件，后者提交必须**报冲突**而不是覆盖；
/// 任一提交点都能还原出当时的共享区内容。
#[test]
pub(crate) fn work_commit_reports_conflicts_and_restore_returns_a_commit_point() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();

    // a、b 都对准同一版（v1）。
    assert_eq!(
        ws.work_pull("w", "a", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );
    assert_eq!(
        ws.work_pull("w", "b", &["note.md".to_string()])
            .unwrap()
            .updated,
        vec!["note.md"]
    );

    // a 改并提交 → 提交点 2。
    let sb_a = sandbox_of(&ws, "w", "a");
    store.write_under(&sb_a, "note.md", b"v2").unwrap();
    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "a 改", 2))
        .expect("a 的提交");
    assert_eq!(rep.id, 2);

    // b 基于 v1 改成 v3：上游已变 → 冲突，整个提交被拒。
    let sb_b = sandbox_of(&ws, "w", "b");
    store.write_under(&sb_b, "note.md", b"v3").unwrap();
    let err = ws
        .work_commit("w", "b", "w--b", &commit_req(&["note.md"], &[], "b 改", 3))
        .unwrap_err();
    assert!(
        err.contains("note.md") && err.contains("冲突"),
        "要逐条点名冲突：{}",
        err
    );
    // 冲突没有留下半个提交：head 仍在 2。
    assert_eq!(
        store.head(&ws.roots("w", &[]).unwrap().store).unwrap(),
        Some(2)
    );

    // 还原到提交点 1：共享区内容回到 v1。
    ws.work_restore("w", 1).unwrap();
    let shared = shared_of(&ws, "w");
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
}

/// 按行锚精确物化共享区：agent 第 1 行提交 v1、第 3 行提交 v3；
/// 回档到第 1 行 → 共享区回到 v1；再丢弃非祖先提交 → 第 3 行的提交记录真的没了。
#[test]
pub(crate) fn work_rewind_to_materializes_by_line_anchor_and_discards_later_commits() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");
    let shared = shared_of(&ws, "w");
    let root = ws.roots("w", &[]).unwrap().store;

    // 第 1 行：a 提交 v1。
    store.write_under(&sb, "note.md", b"v1").unwrap();
    let mut r1 = commit_req(&["note.md"], &[], "v1", 1);
    r1.line = 1;
    let id1 = ws.work_commit("w", "a", "w--a", &r1).unwrap().id;

    // 第 3 行：a 再提交 v3。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let mut r3 = commit_req(&["note.md"], &[], "v3", 3);
    r3.line = 3;
    let id3 = ws.work_commit("w", "a", "w--a", &r3).unwrap().id;
    assert_eq!((id1, id3), (1, 2));

    // 回档到第 1 行（保留行 = 1）：只保留锚 ≤ 1 的提交 → v1。
    let mut keep = std::collections::BTreeMap::new();
    keep.insert("a".to_string(), 1u64);
    assert_eq!(ws.work_head("w").unwrap(), Some(id3));
    assert_eq!(ws.work_rewind_to("w", &keep).unwrap(), Some(id1));
    assert_eq!(
        store.read_under(&shared, "note.md").unwrap().as_deref(),
        Some(&b"v1"[..])
    );
    assert_eq!(ws.work_head("w").unwrap(), Some(id1));

    // 丢弃第 1 行之后的提交：第 3 行的提交记录真的没了。
    ws.work_discard_after("w", Some(id1)).unwrap();
    assert_eq!(store.list_commits(&root).unwrap(), vec![id1]);

    // 恢复到空共享区。
    ws.work_restore_point("w", None).unwrap();
    assert!(store.read_under(&shared, "note.md").unwrap().is_none());
    assert_eq!(ws.work_head("w").unwrap(), None);
}

/// 用户投喂 = 权威提交；agent 拉取后按基线提交，新路径是新增。
#[test]
pub(crate) fn work_user_commit_is_authoritative_and_pull_tracks_the_baseline() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    // 权威覆盖：同名再投喂直接成为新的 head，不报冲突。
    let id = ws.work_commit_user("w", "note.md", b"v2", 2, 0).unwrap();
    assert_eq!(id, 2);

    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.updated, vec!["note.md"]);
    let sb = sandbox_of(&ws, "w", "a");
    assert_eq!(
        store.read_under(&sb, "note.md").unwrap().as_deref(),
        Some(&b"v2"[..])
    );

    // 沙箱里改成本地基线之上的新内容 → modify。
    store.write_under(&sb, "note.md", b"v3").unwrap();
    let rep = ws
        .work_commit(
            "w",
            "a",
            "w--a",
            &commit_req(&["note.md"], &[], "改成 v3", 3),
        )
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(rep.changes[0].path, "note.md");
}

/// 拉取绝不静默覆盖本地已有内容；提交空集与空说明都如实拒绝。
#[test]
pub(crate) fn work_pull_never_clobbers_and_empty_commits_are_refused() {
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    let sb_a = sandbox_of(&ws, "w", "a");
    let sb_b = sandbox_of(&ws, "w", "b");
    const DRAFT: &[u8] = "本地草稿".as_bytes();
    const B_VERSION: &[u8] = "b 版".as_bytes();
    const USER_EDIT: &[u8] = "用户改".as_bytes();

    // 无基线的同沙箱路径：按"已占用"处理，不覆盖。
    store.write_under(&sb_a, "note.md", DRAFT).unwrap();
    let r = ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    assert_eq!(r.occupied, vec!["note.md"]);
    assert_eq!(
        store.read_under(&sb_a, "note.md").unwrap().as_deref(),
        Some(DRAFT)
    );

    // 有基线、本地改了、上游也改了 → 冲突，不覆盖本地。
    ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    store.write_under(&sb_b, "note.md", B_VERSION).unwrap(); // 本地改
    ws.work_commit_user("w", "note.md", USER_EDIT, 2, 0)
        .unwrap(); // 上游也改
    let r = ws.work_pull("w", "b", &["note.md".to_string()]).unwrap();
    assert_eq!(r.conflicts, vec!["note.md"]);
    assert_eq!(
        store.read_under(&sb_b, "note.md").unwrap().as_deref(),
        Some(B_VERSION)
    );

    // 空提交与空说明都被如实拒绝。
    let empty = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &[], "说明", 3))
        .unwrap_err();
    assert!(empty.contains("没有可提交的改动"), "{}", empty);
    let no_msg = ws
        .work_commit("w", "a", "w--a", &commit_req(&["note.md"], &[], "   ", 3))
        .unwrap_err();
    assert!(no_msg.contains("提交说明"), "{}", no_msg);
    let bad_pull = ws.work_pull("w", "a", &[]).unwrap_err();
    assert!(bad_pull.contains("路径"), "{}", bad_pull);
}

/// work_status 把"本地有修改 / 上游有更新 / 冲突"分开说，而不是笼统一句。
#[test]
pub(crate) fn work_status_separates_local_and_upstream_changes() {
    use crate::capabilities::workspace::domain::workstore::StatusState;
    use crate::capabilities::workspace::ports::WorkStore;
    let (ws, _dirs, store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();
    let sb = sandbox_of(&ws, "w", "a");

    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries.len(), 1);
    assert_eq!(st.entries[0].state, StatusState::Clean);

    store.write_under(&sb, "note.md", b"v2").unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::LocalModified);

    ws.work_commit_user("w", "note.md", b"v3", 2, 0).unwrap();
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert_eq!(st.entries[0].state, StatusState::Conflict);
    assert!(st.head.is_some());
    assert!(st.tree.contains(&"note.md".to_string()));
}

/// 删除必须显式：path 从沙箱消失不等于删共享文件；deletes 会真的从主副本删掉。
#[test]
pub(crate) fn work_delete_is_explicit_in_the_commit_request() {
    let (ws, _dirs, _store) = work_ws();
    ws.prepare("w", &["a".to_string()]).unwrap();
    ws.work_commit_user("w", "note.md", b"v1", 1, 0).unwrap();
    ws.work_pull("w", "a", &["note.md".to_string()]).unwrap();

    let rep = ws
        .work_commit("w", "a", "w--a", &commit_req(&[], &["note.md"], "删掉", 2))
        .unwrap();
    assert_eq!(rep.changes.len(), 1);
    assert_eq!(
        rep.changes[0].kind,
        crate::capabilities::workspace::api::ChangeKind::Delete
    );
    let st = ws.work_status("w", "a", &[]).unwrap();
    assert!(!st.tree.contains(&"note.md".to_string()));
}
// ---------- 主副本对 agent 只读：工具层与围栏层两处收口 ----------

/// 只读共享区：读得到、写不进；写自己的沙箱照旧；围栏也不含主副本。
/// 这是"agent 绕不过 pull/commit 直接写共享区"的工具层证据，围栏层的证据由三平台探针给。
#[test]
pub(crate) fn read_only_shared_refuses_builtin_writes_and_leaves_the_fence() {
    use crate::kernel::api::FenceSpec;
    let sb = test_sandbox_readonly("a", &[]);
    let io = InMemorySysIo::new();
    io.seed(&["demo", "work", "note.txt"], "内容");
    // 进 JSON / 补丁的路径一律用**书写形式**（/ 分隔；Windows 反斜杠在 JSON 里非法）。
    let work = s(&["demo", "work", "note.txt"]);

    // 读仍然可以（默认只读 = 可读不可写）。
    let rd = run_builtin(&sb, &io, "read", &format!("{{\"path\":\"{}\"}}", work));
    assert!(rd.ok, "只读不等于读不到：{}", rd.output);

    // 写 / 改 / 补丁一律被工具层拒绝。
    let wr = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", work),
    );
    assert!(!wr.ok && wr.output.contains("只读"), "{}", wr.output);
    let ed = run_builtin(
        &sb,
        &io,
        "edit",
        &format!(
            "{{\"path\":\"{}\",\"old_string\":\"内容\",\"new_string\":\"新\"}}",
            work
        ),
    );
    assert!(!ed.ok && ed.output.contains("只读"), "{}", ed.output);
    let patch = format!(
        "*** Update File: {}\n*** SEARCH\n内容\n*** REPLACE\n新\n*** End File\n",
        work
    );
    let pa = run_builtin(&sb, &io, "patch", &patch);
    assert!(!pa.ok && pa.output.contains("只读"), "{}", pa.output);

    // 自己的沙箱照旧可写。
    let own = s(&["demo", "a", "out.txt"]);
    let ok = run_builtin(
        &sb,
        &io,
        "write",
        &format!("{{\"path\":\"{}\",\"content\":\"x\"}}", own),
    );
    assert!(ok.ok, "{}", ok.output);

    // 围栏：只读时主副本不进 rw；可写沙箱时进 rw（核心 / 权限放开那一路）。
    let spec = FenceSpec::from_sandbox(&sb, false);
    assert!(!spec.rw.contains(&sb.shared), "主副本不该进 rw");
    assert!(spec.rw.contains(&sb.private));
    // 会话租约从沙箱带出：同名 agent 的多个并发会话靠它区分围栏归属。
    assert_eq!(spec.lease, sb.session, "围栏租约 = 沙箱的会话 id");
    let writable = test_sandbox("a", &[]);
    let spec2 = FenceSpec::from_sandbox(&writable, false);
    assert!(spec2.rw.contains(&writable.shared));
}

/// env.rs 装出来的 **agent 会话沙箱**默认就是只读共享区（生产接线，不是测试助手自己设的）。
#[test]
pub(crate) fn agent_sandboxes_from_a_work_are_read_only_for_the_shared_area() {
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec!["[]".into()]);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let meta = core.history_open(&sid).unwrap().0;
    let roster = core.scan();
    let sandboxes = core.sandboxes(&meta, &roster).unwrap();
    let sb = sandboxes.list.first().expect("至少一个 agent 沙箱");
    assert!(!sb.shared_writable, "生产里 agent 沙箱默认只读共享区");
    let (place, path) = sb.resolve(&p(&["w", "work", "x.txt"])).unwrap();
    assert!(!sb.can_write(&place, &path));
}

/// 目的：版本库写闸按**工作**分：不同工作互不阻塞，同一个工作严格串行。
#[test]
pub(crate) fn version_gate_is_per_work() {
    let ws = crate::capabilities::workspace::service::WorkspaceService::new(
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(InMemoryWorkStore::new()),
    );
    let a = ws.gate("a");
    let b = ws.gate("b");
    let _hold_a = a.lock().unwrap_or_else(|e| e.into_inner());
    assert!(b.try_lock().is_ok(), "不同工作互不阻塞");
    let a2 = ws.gate("a");
    assert!(a2.try_lock().is_err(), "同一个工作要串行");
}

/// 目的：虚拟机档下模型给的 guest 写法要能寻址——译成宿主路径**再过同一道根校验**；越界仍拒。
#[test]
pub(crate) fn sandbox_resolves_guest_written_paths() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::api::Place;
    // 跨方言才有翻译（同方言时桥是恒等，走的是宿主写法那条路）。
    let guest = if cfg!(windows) {
        PathDialect::Posix
    } else {
        PathDialect::Windows
    };
    let bridge = PathBridge::new(std::env::consts::OS, guest);
    let mut sb = test_sandbox("a1", &["data"]);
    sb.path_codec = Some(Arc::new(bridge));
    // 私有沙箱在 guest 里的写法要认，并落回宿主私有沙箱。
    let guest_note = format!("{}/note.txt", bridge.to_guest(&sb.private));
    let (place, path) = sb.resolve(&guest_note).expect("guest 写法该认");
    assert_eq!(place, Place::Private);
    assert_eq!(path, sb.private.join("note.txt"));
    // 宿主写法直通（系统工具跑在宿主侧，本来就成立）。
    let host_note = sb.private.join("host.txt").to_string_lossy().into_owned();
    let (place, _) = sb.resolve(&host_note).expect("宿主写法也认");
    assert_eq!(place, Place::Private);
    // 镜像根下、但不在任何允许根里的路径：翻译得出来也要被根校验拒。
    let outside = bridge.to_guest(&abs(&["elsewhere", "x.txt"]));
    let why = sb.resolve(&outside).expect_err("越界要拒");
    assert!(why.contains("系统工具允许的根"), "{}", why);
}
