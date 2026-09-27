//! 工作区能力测试：清单 / 运行包库 / 执行档位与计划 / 沙箱寻址。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn roster_lists_modules() {
    let core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(BTreeMap::new(), vec!["[]".into()]),
    );
    let r = core.scan();
    let ids: Vec<_> = r.modules.iter().map(|m| m.manifest.id.clone()).collect();
    assert_eq!(ids, vec!["a", "b"]);
}

#[test]
pub(crate) fn shipped_modules_scan_clean() {
    // 随仓模块（modules/）是产品内容的一部分：清单必须全部合法、id 与目录一致。
    let roster = crate::capabilities::workspace::detail::FsModules::new(
        PathBuf::from("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan();
    assert!(!roster.modules.is_empty(), "仓库应自带模块");
    assert!(
        roster.rejected.is_empty(),
        "随仓清单必须全部合法：{:?}",
        roster.rejected
    );
    // 随仓的三件就是演示工作流那三件：少了任何一件，演示就跑不起来。
    let ids: Vec<&str> = roster
        .modules
        .iter()
        .map(|m| m.manifest.id.as_str())
        .collect();
    for id in ["harvest", "indexer", "render"] {
        assert!(ids.contains(&id), "随仓模块少了 {}（现在是 {:?}）", id, ids);
    }
}

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

#[test]
pub(crate) fn agent_system_carries_the_real_roots() {
    // 提示词里给出的根必须是真实绝对路径（模型据此拼路径；外部工具也认它）。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let system = &core.single_identity(&sid).expect("身份块");
    assert!(
        system.contains(&s(&["w", "work"])),
        "system 要含共享区真实根：{}",
        system
    );
    assert!(
        system.contains(&s(&["w", "a"])),
        "system 要含私有沙箱真实根：{}",
        system
    );
    assert!(
        system.contains(&format!("：{}", s(&["a"]))),
        "system 要含模块目录真实根：{}",
        system
    );
    // 路径写法要用真实根拼（模型照抄它）
    assert!(
        system.contains(&format!("{}/note.txt", s(&["w", "work"]))),
        "路径写法要用真实根拼：{}",
        system
    );
    // 护栏：提示词里不得出现任何解析不了的路径写法（模型会照抄它）
    assert!(
        !system.contains("work:/") && !system.contains("sandbox:/"),
        "提示词只该给真实根：{}",
        system
    );
}

#[test]
pub(crate) fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::capabilities::workspace::api::Sandboxes {
        shared: abs(&["w", "work"]),
        list: vec![test_sandbox("甲", &["a"])],
    };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
}

#[test]
pub(crate) fn package_manifest_check_rejects_illegal_forms() {
    let check = crate::capabilities::workspace::domain::packages::check_manifest;
    assert!(
        check(&pkg("python", "3.12.4")).is_ok(),
        "prefix 类默认 kind"
    );
    assert!(
        check(&pkg_yaml("id: Python\nversion: 1\nprefix: opt/p")).is_err(),
        "id 只允许小写"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nkind: magic\nprefix: opt/p")).is_err(),
        "kind 只认 prefix / system"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1")).is_err(),
        "prefix 类必须给 prefix"
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt/../etc")).is_err(),
        "前缀不能含 .."
    );
    assert!(
        check(&pkg_yaml("id: py\nversion: 1\nprefix: opt\\\\rt")).is_err(),
        "前缀用 / 书写形式"
    );
    assert!(
        check(&pkg_yaml("id: cc\nversion: 1\nkind: system")).is_err(),
        "system 类必须给 provides_paths"
    );
    assert!(check(&pkg_yaml(
        "id: cc\nversion: 1\nkind: system\nprovides_paths: [usr/include]"
    ))
    .is_ok());
    assert!(
        check(&pkg_yaml("id: a\nversion: 1\nprefix: opt/a\nrequires: [a]")).is_err(),
        "requires 不能依赖自己"
    );
}

#[test]
pub(crate) fn module_runtimes_are_validated() {
    let mut m = module_of("a");
    m.manifest.runtimes = vec!["python".to_string(), "cc".to_string()];
    assert!(crate::capabilities::workspace::api::check_runtimes(&m.manifest).is_ok());
    m.manifest.runtimes = vec!["Python".to_string()];
    assert!(
        crate::capabilities::workspace::api::check_runtimes(&m.manifest).is_err(),
        "大写不合法"
    );
    m.manifest.runtimes = vec!["python".to_string(), "python".to_string()];
    assert!(
        crate::capabilities::workspace::api::check_runtimes(&m.manifest)
            .unwrap_err()
            .contains("重复"),
        "重复声明要拒收"
    );
}

#[test]
pub(crate) fn module_tools_may_not_take_builtin_names() {
    let mut m = module_of("a");
    m.manifest
        .tools
        .insert("read_txt".to_string(), decl("python tools/read_txt.py"));
    assert!(
        crate::capabilities::workspace::api::check_tools(
            &m.manifest,
            &crate::capabilities::tools::api::names(),
        )
        .is_ok(),
        "普通工具名可用"
    );
    for name in ["read", "write", "search"] {
        m.manifest
            .tools
            .insert(name.to_string(), decl("python tools/x.py"));
        let why = crate::capabilities::workspace::api::check_tools(
            &m.manifest,
            &crate::capabilities::tools::api::names(),
        )
        .unwrap_err();
        assert!(why.contains("保留名"), "内置工具名要拒收：{}", why);
        m.manifest.tools.remove(name);
    }
}

#[test]
pub(crate) fn library_keeps_versions_and_rejects_duplicates() {
    let lib = Library::build(
        vec![
            pkg("python", "3.12.4"),
            pkg("python", "3.11.9"),
            pkg("python", "3.12.4"),
            pkg_yaml("id: bad\nversion: 1"),
        ],
        vec!["x：package.yaml 非法".to_string()],
    );
    let versions: Vec<&str> = lib
        .versions_of("python")
        .iter()
        .map(|p| p.version.as_str())
        .collect();
    assert_eq!(
        versions,
        vec!["3.11.9", "3.12.4"],
        "同 (id, version) 只收一份，版本升序"
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("只收先出现的那份")),
        "{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("prefix")),
        "非法清单要说明原因：{:?}",
        lib.rejected
    );
    assert!(
        lib.rejected.iter().any(|r| r.contains("package.yaml 非法")),
        "适配层拒收原因也要留：{:?}",
        lib.rejected
    );
    let caps = lib.capability_versions();
    assert_eq!(caps.get("python").map(|v| v.len()), Some(2));
}

#[test]
pub(crate) fn package_conflicts_flag_overlapping_paths() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: a\nversion: 1\nkind: system\nprovides_paths: [usr/lib]"),
            pkg_yaml("id: b\nversion: 1\nkind: system\nprovides_paths: [usr/lib/x86_64]"),
            pkg("node", "20.11.1"),
        ],
        Vec::new(),
    );
    let refs: Vec<&PackageManifest> = lib.packages.iter().collect();
    let got = crate::capabilities::workspace::domain::packages::conflicts(&refs);
    assert_eq!(got.len(), 1, "只有那对写进同一处的包冲突：{:?}", got);
    assert_eq!(got[0].0, "usr/lib");
    assert!(
        got[0].1.contains("a@1") && got[0].2.contains("b@1"),
        "{:?}",
        got
    );
}

/// 界面上的"能不能选"与创建/编辑的拒绝走同一个函数，所以这里钉住的就是那两处共同的事实。
/// 现在的判据是**逐项清单**：缺哪几项、每项怎么补，都要能读出来。
#[test]
pub(crate) fn vm_tier_readiness_gates_creation_and_editing() {
    // 本机档：任何机器上都能承载（不装载运行包、不要 guest）。
    let host = exec::tier_readiness(
        &ExecSpec::default(),
        None,
        &crate::adapters::HostProbeAdapter,
    );
    assert!(host.ready(), "本机档没有前置条件");
    assert!(host.requirements.is_empty(), "本机档不该有虚拟机前置清单");
    assert!(exec::tier_refusal(
        &ExecSpec::default(),
        None,
        &crate::adapters::HostProbeAdapter
    )
    .is_none());

    // 虚拟机档：guest 本体尚未接入是**所有机器**共同缺的一项，所以现在谁都不能建。
    let ghost = ExecSpec {
        tier: Tier::Vm,
        base: Some("definitely-not-a-real-base-root".to_string()),
        ..Default::default()
    };
    let r = exec::tier_readiness(&ghost, None, &crate::adapters::HostProbeAdapter);
    assert!(!r.ready(), "前置不齐就不成立：{:?}", r);
    let unmet: Vec<&str> = r.unmet().iter().map(|x| x.id).collect();
    assert!(
        unmet.contains(&"guest"),
        "guest 未接入要如实列出：{:?}",
        unmet
    );
    assert!(
        unmet.contains(&"base"),
        "填错的基础根要如实列出：{:?}",
        unmet
    );
    for item in r.unmet() {
        assert!(
            !item.how.is_empty(),
            "每一项没满足都要给出怎么补：{:?}",
            item
        );
        assert!(!item.detail.is_empty(), "每一项都要有现状描述：{:?}", item);
    }
    let why = exec::tier_refusal(&ghost, None, &crate::adapters::HostProbeAdapter)
        .expect("不成立就要给可读理由");
    assert!(why.contains("虚拟机档现在不可用"), "{}", why);

    // 基础根在场：base 这一项要认出来（其余项照旧按事实）。
    let dir = crate::tests::scratch("tier-readiness");
    let real = ExecSpec {
        tier: Tier::Vm,
        base: Some(dir.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let r2 = exec::tier_readiness(&real, None, &crate::adapters::HostProbeAdapter);
    let base_item = r2
        .requirements
        .iter()
        .find(|x| x.id == "base")
        .expect("清单里要有基础根这一项");
    assert!(base_item.met, "在场的基础根要认出来：{:?}", base_item);
    // 严格：guest 未接入时**任何机器**都不能建虚拟机档会话。
    assert!(!r2.ready(), "guest 未接入期间一律不可用");
    let _ = std::fs::remove_dir_all(&dir);
}

/// QEMU 检测：登记了就用登记的路径，没登记就看 PATH；产品不自带、不下载。
#[test]
pub(crate) fn vm_requirements_report_qemu_registration() {
    let base = crate::tests::scratch("vm-req-qemu");
    let base_str = base.to_string_lossy().into_owned();
    let spec = ExecSpec {
        tier: Tier::Vm,
        base: Some(base_str.clone()),
        ..Default::default()
    };

    // 登记了一个不存在的路径：必须报"找不到"，并给出怎么补。
    let r = exec::tier_readiness(
        &spec,
        Some("definitely-not-qemu.exe"),
        &crate::adapters::HostProbeAdapter,
    );
    let qemu = r
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(!qemu.met, "不存在的路径不算找到：{:?}", qemu);
    assert!(!qemu.how.is_empty(), "没找到就要给怎么补：{:?}", qemu);

    // 登记一个真实存在的文件：要认出来（QEMU 是不是真的不重要——这里只钉"登记生效"）。
    let fake = base.join("qemu-system-x86_64");
    std::fs::write(&fake, b"stub").unwrap();
    let r2 = exec::tier_readiness(
        &spec,
        Some(fake.to_string_lossy().as_ref()),
        &crate::adapters::HostProbeAdapter,
    );
    let qemu2 = r2
        .requirements
        .iter()
        .find(|x| x.id == "qemu")
        .expect("清单里要有 QEMU 这一项");
    assert!(qemu2.met, "登记的路径在场就要认出来：{:?}", qemu2);
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
pub(crate) fn exec_host_tier_ignores_packages() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let plan = exec::plan(&ExecSpec::default(), &modules, &Library::default())
        .expect("本机档不装载运行包，不会因缺包失败");
    assert_eq!(plan.tier, Tier::Host);
    assert!(plan.packages.is_empty());
    assert!(plan.base.is_none());
    assert!(!plan.net, "默认不放行出站网络");
    let summary = exec::plan_summary(&plan);
    assert!(
        summary.contains("本机") && summary.contains("不放行"),
        "{}",
        summary
    );
}

#[test]
pub(crate) fn exec_vm_tier_reports_missing_ambiguous_and_unavailable() {
    let modules = vec![module_with_runtimes("a", &["python"])];
    let empty = Library::default();
    let spec = vm_spec();
    assert_eq!(
        exec::vm_diagnoses(&modules, &empty, &spec),
        vec![Diagnosis::Missing {
            module: "a".to_string(),
            capability: "python".to_string()
        }]
    );
    let un = exec::unavailable(&spec, &modules, &empty);
    assert_eq!(
        un.get("a"),
        Some(&vec!["python".to_string()]),
        "虚拟机档缺包 = 该模块工具不可用"
    );
    assert!(
        exec::unavailable(&ExecSpec::default(), &modules, &empty).is_empty(),
        "本机档一律可用"
    );
    // 缺包不拦会话：计划里没有可装载的包，那一步的降级由 unavailable 收口。
    let partial = exec::plan(&spec, &modules, &empty).expect("缺包不该拦会话");
    assert!(partial.packages.is_empty());
    let said = exec::diagnose_text(&exec::vm_diagnoses(&modules, &empty, &spec));
    assert!(
        said.contains("runtimes/"),
        "缺包的说法要告诉用户把包放哪：{}",
        said
    );
    // 多版本且未定版 = 不替用户选
    let two = Library::build(
        vec![pkg("python", "3.12.4"), pkg("python", "3.11.9")],
        Vec::new(),
    );
    assert!(exec::vm_diagnoses(&modules, &two, &spec)
        .iter()
        .any(|d| matches!(d, Diagnosis::Ambiguous { .. })));
    // 定版指定的版本不在库里 = 如实报
    let bad = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "9.9".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &bad)
        .iter()
        .any(|d| matches!(d, Diagnosis::UnknownPin { .. })));
    // 多版本未定版 = 选型不成立：派生计划如实拒绝（这是用户要解决的选型问题，不是"缺包"）
    let refused = exec::plan(&spec, &modules, &two).unwrap_err();
    assert!(
        exec::diagnose_text(&refused).contains("多个版本"),
        "{}",
        exec::diagnose_text(&refused)
    );
    // 定版之后可以成立
    let ok = ExecSpec {
        pins: BTreeMap::from([("python".to_string(), "3.12.4".to_string())]),
        ..vm_spec()
    };
    assert!(exec::vm_diagnoses(&modules, &two, &ok).is_empty());
    assert_eq!(exec::plan(&ok, &modules, &two).unwrap().packages.len(), 1);
}

#[test]
pub(crate) fn exec_vm_plan_pins_versions_and_orders_prefix_before_system() {
    let lib = Library::build(
        vec![
            pkg_yaml("id: cc\nversion: 13.2.0\nkind: system\nprovides_paths: [usr/bin, usr/include]\nrequires: [binutils]"),
            pkg_yaml("id: binutils\nversion: 2.42\nprefix: opt/rt/binutils2.42"),
            pkg("python", "3.12.4"),
        ],
        Vec::new(),
    );
    let modules = vec![module_with_runtimes("a", &["python", "cc"])];
    let plan = exec::plan(&vm_spec(), &modules, &lib).expect("虚拟机档可成立");
    let ids: Vec<&str> = plan.packages.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        vec!["binutils", "python", "cc"],
        "先独立前缀，后写进系统路径的包；包的 requires 走闭包"
    );
    assert_eq!(plan.packages[0].version, "2.42", "计划里是定版后的精确版本");
    assert_eq!(
        plan.packages[0].prefix, "opt/rt/binutils2.42",
        "独立前缀随计划走（装配按它挂载）"
    );
    assert_eq!(plan.packages[2].kind, "system", "写进系统路径的包排在最后");
    assert_eq!(plan.base.as_deref(), Some("base-linux"));
    assert!(!plan.net, "默认不放行出站网络");
}

#[test]
pub(crate) fn diagnose_text_spells_out_every_reason() {
    let missing = exec::diagnose_text(&[Diagnosis::Missing {
        module: "a".to_string(),
        capability: "python".to_string(),
    }]);
    assert!(
        missing.contains("模块 a") && missing.contains("python") && missing.contains("runtimes/"),
        "{}",
        missing
    );
    let ambiguous = exec::diagnose_text(&[Diagnosis::Ambiguous {
        capability: "python".to_string(),
        versions: vec!["3.11.9".to_string(), "3.12.4".to_string()],
    }]);
    assert!(
        ambiguous.contains("多个版本") && ambiguous.contains("3.12.4"),
        "{}",
        ambiguous
    );
    let bad = exec::diagnose_text(&[Diagnosis::UnknownPin {
        capability: "python".to_string(),
        version: "9.9".to_string(),
    }]);
    assert!(bad.contains("定版 9.9"), "{}", bad);
    let clash = exec::diagnose_text(&[Diagnosis::Conflict {
        path: "usr/lib".to_string(),
        a: "a@1".to_string(),
        b: "b@1".to_string(),
    }]);
    assert!(
        clash.contains("usr/lib") && clash.contains("a@1") && clash.contains("b@1"),
        "{}",
        clash
    );
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
    let mut core = Core::new(
        registry_service(InMemorySettings::with_tier(Tier::Vm), Arc::clone(&llm)),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        llm,
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        test_prompt(),
        test_tools(),
        Arc::new(crate::kernel::log::NoopLog),
        Arc::new(crate::adapters::HostProbeAdapter),
    );
    // 创建路径的档位来自设置（基础根留空）：成立与否随本机而定，这里钉的是**接线**——
    // 机器承载不了就必须拒绝，且什么都不留下。
    let default_vm = ExecSpec {
        tier: Tier::Vm,
        ..ExecSpec::default()
    };
    let opened = core.create_work(work("vm-default", WorkMode::Single, &["a"]));
    if exec::tier_readiness(&default_vm, None, &crate::adapters::HostProbeAdapter).ready() {
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
    let mut core2 = Core::new(
        registry_service(InMemorySettings::new(), Arc::clone(&llm2)),
        Arc::new(InMemoryHistory::new()),
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![module_of("a")])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        llm2,
        Arc::new(SilentRunner),
        Arc::new(InMemorySysIo::new()),
        test_prompt(),
        test_tools(),
        Arc::new(crate::kernel::log::NoopLog),
        Arc::new(crate::adapters::HostProbeAdapter),
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
                }],
                tier: "vm".to_string(),
                base: Some("definitely-not-a-real-base-root".to_string()),
                pins: BTreeMap::new(),
                net: false,
            },
        )
        .expect_err("基础根不在场就不许改入虚拟机档");
    assert!(
        err.contains("虚拟机档现在不可用") && err.contains("基础根不在场"),
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
    // 虚拟机档 + 空包库：模块声明的运行包没装载 → 工具不落进程，回执如实说缺哪个能力。
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
    let mut core = Core::new(
        registry_service(InMemorySettings::with_tier(Tier::Host), Arc::clone(&llm)),
        Arc::clone(&hist) as Arc<dyn HistoryStore + Send + Sync>,
        Arc::new(InMemoryWorkspace::new()),
        Arc::new(VecSource(vec![mod_a])),
        Arc::new(InMemoryPackages::empty()),
        Arc::new(NoFenceHost),
        llm,
        Arc::clone(&runner) as Arc<dyn ToolRunner + Send + Sync>,
        Arc::new(InMemorySysIo::new()),
        test_prompt(),
        test_tools(),
        Arc::new(crate::kernel::log::NoopLog),
        Arc::new(crate::adapters::HostProbeAdapter),
    );
    // 虚拟机档现在一律不可选（guest 本体尚未接入），所以**创建**走本机档；
    // 建好之后把落盘档位改成 vm——这正是"档位承载检查"与"缺包不拦会话"两件事的交界：
    // 已存在的会话照常打开、按 vm 档判工具可用性。
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    hist.force_tier(&sid, Tier::Vm);
    // 编辑一次会把内存里的会话丢掉；下一次访问按落盘 meta（已是 vm 档）重建，
    // 于是"工具可用性按 vm 档判"这条路才真的被走到。
    let base_dir = crate::tests::scratch("module-without-runtime-base");
    let mut rebuild = edit_of(vec![("a", &["a"], "")]);
    rebuild.tier = "vm".to_string();
    rebuild.base = Some(base_dir.to_string_lossy().into_owned());
    core.edit_session(&sid, rebuild).unwrap();
    // 访问一次（前端打开会话就是这一步）把会话按新配置重建；single_say 只认内存里已建好的会话。
    with_live(|l| core.continue_flow(&sid, l)).unwrap();
    let events = with_live(|l| core.single_say(&sid, "干活", l)).unwrap();
    assert!(
        runner.calls.lock().expect("锁").is_empty(),
        "缺运行包时不落进程"
    );
    let texts = test_prompts().tools();
    let expect = texts.render(
        &texts.module_unavailable,
        &[
            ("module", "a".to_string()),
            ("capability", "python".to_string()),
        ],
    );
    let feedback = core
        .single_history(&sid)
        .unwrap()
        .iter()
        .find(|m| m.role == "user" && m.content.contains("[工具结果] a.grep"))
        .map(|m| m.content.clone())
        .unwrap_or_default();
    assert!(feedback.contains(&expect), "回执要用册子文案：{}", feedback);
    let lines = tool_line_texts(&events);
    assert!(
        lines
            .iter()
            .any(|l| l.contains("grep") && l.contains("失败")),
        "失败也要发 tool 行：{:?}",
        lines
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
