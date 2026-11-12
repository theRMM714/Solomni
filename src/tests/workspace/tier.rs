//! 目的：虚拟机档能不能选只看机器能力。
//! 管：tier_readiness 与会话级虚拟根解耦。
//! 不管：VM 装配（`src/tests/vm`）。
//! 联动：`src/capabilities/workspace/domain/tier.rs`。

use super::super::builders::*;
use super::super::prelude::*;

/// 目的：虚拟机档"能不能选"只看**机器能力**，与会话级选哪个虚拟根无关——Web 换档死锁的回归锚。
/// 约束：虚拟根只在保存 /「开始」时校验（构造承载判据时合并进去）。
#[test]
pub(crate) fn vm_availability_is_machine_only_and_base_is_session_level() {
    use crate::capabilities::vm::domain::exec::host_verified;
    use crate::tests::doubles::core_with_gateway_probe;
    // 虚拟根由 test_guest 造在 .vm/guest/<名> 下，并声明三平台客户端；探测用真实路径 + 本机可起虚拟机。
    crate::tests::test_guest("vm-avail");
    let mut core = core_with_gateway_probe(
        vec![module_of("a")],
        gw(BTreeMap::new(), Vec::new()),
        crate::tests::doubles::vm_capable_host_probe(),
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
    let _ = std::fs::remove_dir_all(
        crate::tests::root_dir()
            .join(".vm")
            .join("guest")
            .join("vm-avail"),
    );
}

/// 目的：虚拟机档的承载校验：前置条件不具备时，**创建与编辑都如实拒绝**（用户环境问题，不是选型问题）。与界面上的"能不能选"同源（`exec::tier_readiness`），两处不会各说各话。
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
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
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
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm2,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
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
                    chara: String::new(),
                }],
                tier: "vm".to_string(),
                guest: Some("definitely-not-a-real-guest".to_string()),
                pins: BTreeMap::new(),
                permission: None,
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
        vec![module_with_runtimes("a", &["no-such-capability"])],
    );
    let host = core.runtime_report(Tier::Host);
    assert_eq!(host.tier, "host");
    assert_eq!(
        host.declared.get("a"),
        Some(&vec!["no-such-capability".to_string()])
    );
    assert_eq!(
        host.missing.get("a"),
        Some(&vec!["no-such-capability".to_string()]),
        "宿主确实没有这个能力时照实报"
    );
    assert!(host.available.is_empty());
    assert!(
        matches!(
            host.diagnoses.as_slice(),
            [
                crate::capabilities::conductor::api::RuntimeDiagnosis::Package(
                    crate::capabilities::packages::api::Problem::Missing { .. }
                )
            ]
        ),
        "本机档缺能力也要如实诊断：{:?}",
        host.diagnoses
    );
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
pub(crate) fn host_runtime_report_uses_effective_path() {
    let probe = Arc::new(FixedProbe {
        exes: vec!["python".to_string()],
        ..Default::default()
    });
    let core = core_with_pkgs_probe(
        vec![module_with_runtimes("a", &["python"])],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemorySysIo::new()),
        Arc::new(InMemoryPackages::empty()),
        probe,
    );
    let host = core.runtime_report(Tier::Host);
    assert!(
        host.missing.is_empty(),
        "宿主 PATH 上有 python 就不算缺：{:?}",
        host.missing
    );
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
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(vec![mod_a]))),
        Arc::new(crate::capabilities::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::clone(&runner) as Arc<dyn ProcessRunner>,
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        crate::tests::doubles::test_host_probe(),
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
