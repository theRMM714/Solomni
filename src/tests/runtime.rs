//! 目的：钉应用服务（会话中心与跨单元编排）的不变式。
//! 管：VM 会话与外部 VM、双稳定挂载、工作区策略分层、绑定复核、宿主↔VM 编辑、删除保外部工作区、失败引导清理、思维链随工具行落档。
//! 不管：各单元内部规则（按能力落各自文件）。
//! 联动：src/units/runtime/、tests/doubles、平台探针。

mod proxy;
use super::builders::*;
use super::prelude::*;
// 只有非 macOS 的用例用得到这个面（macOS 上其余用例整体 cfg 掉）。
#[cfg(not(target_os = "macos"))]
use crate::presentation::faces::Faces;
use crate::units::session_model::api::{MEETING, PROXY, SINGLE};

/// 造一台"根在给定目录、补齐面报给定清单"的核心（只为验外部工具路径的取法）。
fn runtime_rooted(
    root: &std::path::Path,
    settings: InMemorySettings,
    provision: Arc<dyn crate::units::provision::api::Provision + Send + Sync>,
) -> Core {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(BTreeMap::new(), vec![]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Core::new(
        root.to_path_buf(),
        registry_service(settings, Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        test_vm(),
        test_packages(Arc::new(InMemoryPackages::empty())),
        provision,
        test_modules(Arc::new(VecSource(vec![module_of("a")]))),
        Arc::new(crate::units::maintenance::builds::api::NoBuilds),
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
        Arc::new(crate::units::residents::api::NoResidents),
        Arc::new(crate::units::catalog::secrets::api::NoSecrets),
        crate::tests::doubles::test_profiles(),
    )
}

/// 目的：造一台"能起虚拟机会话"的核心：探针按用例声明作答，VM 引擎可控（慢 / 失败）。
/// 约束：与它的两条用例同一道门——本平台 guest 后端尚未真机验收时没有调用方。
#[cfg(not(target_os = "macos"))]
fn runtime_for_vm(
    probe: Arc<dyn crate::kernel::ports::HostProbe + Send + Sync>,
    engine: crate::tests::doubles::FakeEngine,
    modules: Vec<Module>,
) -> Core {
    let gateway: Arc<dyn ChatGateway + Send + Sync> = Arc::new(gw(BTreeMap::new(), vec![]));
    let llm = test_llm(
        Arc::clone(&gateway),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
    );
    Core::new(
        crate::tests::root_dir(),
        registry_service(InMemorySettings::with_tier(Tier::Vm), Arc::clone(&llm)),
        test_history(),
        test_workspace(Arc::new(InMemoryWorkspace::new())),
        crate::tests::doubles::test_vm_with_engine(engine),
        test_packages(Arc::new(InMemoryPackages::empty())),
        test_provision(),
        test_modules(Arc::new(VecSource(modules))),
        Arc::new(crate::units::maintenance::builds::api::NoBuilds),
        llm,
        test_tools_svc_with(
            Arc::new(SilentRunner),
            Arc::new(InMemorySysIo::new()),
            Arc::new(NoFenceHost),
        ),
        test_prompt(),
        test_tools_svc(),
        Arc::new(crate::kernel::ports::NoopLog),
        probe,
        Arc::new(crate::units::residents::api::NoResidents),
        Arc::new(crate::units::catalog::secrets::api::NoSecrets),
        crate::tests::doubles::test_profiles(),
    )
}

/// 目的：探针按"机器能力齐 + 虚拟根在场"作答（虚拟根的真文件由 `test_guest` 造）。
/// 约束：与它的两条用例同一道门——本平台 guest 后端尚未真机验收时没有调用方。
#[cfg(not(target_os = "macos"))]
fn vm_capable_probe(
    guest: &str,
) -> std::sync::Arc<dyn crate::kernel::ports::HostProbe + Send + Sync> {
    crate::tests::test_guest(guest);
    crate::tests::doubles::vm_capable_host_probe()
}

/// 目的：创建虚拟机档会话时**核心线程不陪着等 guest**——起 guest（几十秒到几分钟）在调用方线程上跑。
/// 约束：这段判据要虚拟机档真能建起来；本平台 guest 后端尚未真机验收（`VERIFIED_HOSTS` 只有 linux / windows），
///   所以只在已验平台跑——macOS 验收后去掉这个 cfg 并同步那张表。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn creating_a_vm_session_does_not_block_the_core() {
    use crate::tests::doubles::FakeEngine;
    use crate::units::runtime::api::Handle;
    const BOOT: std::time::Duration = std::time::Duration::from_millis(400);
    let guest = crate::tests::test_guest("core-free");
    let probe = vm_capable_probe(&guest);
    let engine = FakeEngine {
        boot_delay: Some(BOOT),
        ..Default::default()
    };
    let core = runtime_for_vm(probe, engine.clone(), vec![module_of("a")]);
    let handle = Handle::spawn(core).expect("起核心");
    let faces = Faces::from_handle(&handle);
    let mut spec = work("w-core-free", SINGLE, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest.clone());
    let creator = {
        let faces = faces.clone();
        std::thread::spawn(move || faces.work.create_work(spec))
    };
    // 让 boot 真的开始，再问核心一件小事：它必须立刻回，而不是等 400ms。
    std::thread::sleep(std::time::Duration::from_millis(80));
    let t0 = std::time::Instant::now();
    let answered = faces.work.tier_choices();
    let waited = t0.elapsed();
    let created = creator.join().expect("线程没崩").expect("建会话");
    assert!(answered.is_ok(), "核心要能应答");
    assert!(
        waited < BOOT / 2,
        "起 guest 期间核心仍要立刻应答（实测等了 {waited:?}，boot 本身 {BOOT:?}）"
    );
    assert_eq!(created.0.sid, "w-core-free");
    assert!(
        engine
            .calls
            .lock()
            .expect("锁")
            .iter()
            .any(|c| c.starts_with("start:")),
        "guest 真的要起来"
    );
}

/// 目的：guest 起不来时**什么都不留**：会话没建、历史里没有它。
/// 约束：同 `creating_a_vm_session_does_not_block_the_core`——要虚拟机档真能建起来，只在已验平台上判。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn a_failed_guest_boot_leaves_no_session_behind() {
    use crate::tests::doubles::FakeEngine;
    use crate::units::runtime::api::Handle;
    let guest = crate::tests::test_guest("boot-fail");
    let probe = vm_capable_probe(&guest);
    let engine = FakeEngine {
        boot_error: Some("起不来（测试注入）".to_string()),
        ..Default::default()
    };
    let core = runtime_for_vm(probe, engine, vec![module_of("a")]);
    let handle = Handle::spawn(core).expect("起核心");
    let faces = Faces::from_handle(&handle);
    let mut spec = work("w-boot-fail", SINGLE, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest);
    let err = faces.work.create_work(spec).expect_err("起不来要如实拒绝");
    assert!(err.contains("起不来"), "{}", err);
    assert!(
        faces.history.open("w-boot-fail").is_err(),
        "会话记录不该留下"
    );
    assert!(
        faces.history.list().expect("历史").is_empty(),
        "历史里不该留下这条会话"
    );
    // 占名也要撤掉：同名重试能通过（而不是被"工作名已存在"挡住）。
    let mut again = work("w-boot-fail", SINGLE, &["a"]);
    again.tier = Tier::Host;
    faces
        .work
        .create_work(again)
        .expect("同一名字在本机档上应该建得起来（失败那次没占住名字）");
}
/// 目的：外部工作区 × 虚拟机档的组合装配：隔离产品根（内含可用虚拟根）+ 产品根之外的目录。
/// 返回（核心、产品根、外部根、虚拟根名）；引擎由调用方给——`specs` 记起 guest 时的挂载计划。
/// 约束：产品根与外部根是兄弟目录，所以"外部根"天然在产品根之外（`resolve_workspace_binding` 才放行）。
#[cfg(not(target_os = "macos"))]
fn core_for_external_vm(
    tag: &str,
    engine: FakeEngine,
) -> (Core, std::path::PathBuf, std::path::PathBuf, String) {
    use crate::units::workspace::detail::FsWorkspace;
    let root = crate::tests::scratch(tag);
    let product = root.join("product");
    let external = root.join("outside");
    std::fs::create_dir_all(&product).expect("建产品根");
    std::fs::create_dir_all(&external).expect("建外部目录");
    std::fs::write(external.join("marker.txt"), b"OUTSIDE").expect("写外部文件");
    // 虚拟根长在**注入的产品根**里：默认产品根解析不到它（`test_guest` 只造在当前目录）。
    let guest = test_guest_at(&product, "vm-ext");
    let core = core_with_workdirs_vm(
        product.clone(),
        vec![module_of("a")],
        gw(BTreeMap::new(), vec![]),
        Arc::new(FsWorkspace::new(product.join("session"))),
        engine,
    );
    (core, product, external, guest)
}

/// 目的：一次起 guest 的**稳定挂载计划**：会话树根与外部工作区根各一条，私沙被会话树根覆盖。
/// 约束：退回 `shared.parent()` 会命中"外部根的父目录"（外部根是被挂的兄弟目录）——那一条必须不出现。
#[cfg(not(target_os = "macos"))]
fn assert_two_stable_mounts(
    engine: &FakeEngine,
    product: &std::path::Path,
    external: &std::path::Path,
    work_name: &str,
) {
    let specs = engine.specs.lock().expect("锁").clone();
    let spec = specs.first().expect("起过一次 guest");
    let hosts: Vec<std::path::PathBuf> = spec.mounts.iter().map(|m| m.host.clone()).collect();
    let session_root = product.join("session").join(work_name);
    let external =
        crate::kernel::api::path::native(&external.canonicalize().expect("规范化外部根"));
    assert!(
        hosts.contains(&session_root),
        "会话树根（私沙 + .work 的父目录）要单独挂：{:?}",
        hosts
    );
    assert!(
        hosts.contains(&external),
        "外部工作区根要单独挂：{:?}",
        hosts
    );
    assert!(
        !hosts.iter().any(|h| Some(h.as_path()) == external.parent()),
        "不能退回 shared.parent()（会把外部根的父目录整棵挂进去）：{:?}",
        hosts
    );
    // 私沙不在挂载树里 → guest 内译成镜像根 → helper 起围栏时 fail-closed，所以这条要钉住。
    let private = session_root.join("a");
    assert!(
        hosts.iter().any(|h| private.starts_with(h)),
        "私沙要被某条挂载覆盖：{:?}",
        hosts
    );
}

/// 目的：外部工作区 × 虚拟机档建会话时，起 guest 的挂载计划是"会话树 + 外部根"两条稳定根。
/// 约束：两条根互不包含（绑定校验保证外部根在产品根之外），所以不能再用 `shared.parent()` 猜挂载点。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn external_workspace_vm_session_boots_with_two_stable_mounts() {
    let engine = FakeEngine::new();
    let (mut core, product, external, guest) = core_for_external_vm("vm-ext-boot", engine.clone());
    let mut spec = work("wx", SINGLE, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest);
    spec.workspace = Some(external.clone());
    let sid = core
        .create_work(spec)
        .expect("外部工作区 + 虚拟机档可建")
        .sid;
    assert_eq!(sid, "wx");
    let (meta, _) = core.lifecycle.history_open(&sid).expect("会话");
    assert_eq!(
        meta.workspace.as_ref().map(|b| b.root.clone()),
        Some(crate::kernel::api::path::native(
            &external.canonicalize().expect("规范化外部路径")
        )),
        "绑定写进根会话 meta（规范化绝对路径）"
    );
    let binding = core
        .turn
        .workspace_binding_of(&meta, core.probe.as_ref())
        .expect("加载复检要过（绑定没被手改）");
    assert_eq!(
        binding,
        Some(crate::kernel::api::path::native(
            &external.canonicalize().expect("规范化外部路径")
        )),
        "复检返回的绑定根要与 meta 一致"
    );
    let session = product.join("session").join("wx");
    assert!(session.join(".work-root").is_file(), "布局投影已落盘");
    assert!(!session.join("work").exists(), "外部绑定不建会话内 work/");
    assert_two_stable_mounts(&engine, &product, &external, "wx");
}

/// 目的：工作区绑定按档位分档——本机档拒绝网络工作区（且在 canonicalize 之前，不触网），
///   虚拟机档接受 UNC、拒绝共享根与文件系统根。
/// 约束：不可路由的地址（TEST-NET-1）一旦真去连就会挂很久；用时耗把「不触网」变成可判定属性。
#[cfg(windows)]
#[test]
pub(crate) fn workspace_policy_splits_network_roots_by_tier() {
    use crate::units::pure::exec_model::api::Tier;
    use crate::units::workspace::api::policy;
    use std::path::Path;
    use std::time::{Duration, Instant};

    let unreachable = Path::new(r"\\192.0.2.1\share\proj");
    let started = Instant::now();
    let err = policy(unreachable, Tier::Host).expect_err("本机档必须拒绝网络工作区");
    assert!(err.contains("本机档不能用网络工作区"), "{err}");
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "本机档的拒绝不能触网：耗时 {:?}",
        started.elapsed()
    );

    policy(Path::new(r"\\server\share\proj"), Tier::Vm).expect("虚拟机档接受 UNC");
    let err = policy(Path::new(r"\\server\share"), Tier::Vm).expect_err("共享根要拒");
    assert!(err.contains("网络共享的根目录"), "{err}");
    let err = policy(Path::new(r"D:\"), Tier::Vm).expect_err("文件系统根要拒");
    assert!(err.contains("文件系统根目录"), "{err}");
}

/// 目的：加载复检在这台机器上也能跑：越界拒绝、产品外的目录放行、暂时不可达不挡（不因平台差异漏判）。
#[test]
pub(crate) fn workspace_recheck_rejects_bounds_and_allows_unreachable_roots() {
    use crate::units::pure::exec_model::api::Tier;
    use crate::units::workspace::api::{product_root, recheck};

    let root = crate::tests::scratch("recheck-cross");
    let product = root.join("product");
    let outside = root.join("outside");
    std::fs::create_dir_all(&product).expect("建产品根");
    std::fs::create_dir_all(&outside).expect("建外部目录");
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    let product = product_root(&product, &probe);

    let err = recheck(&product, Tier::Host, &product, &probe).expect_err("产品根自身要拒");
    assert!(err.contains("互相包含"), "{}", err);
    recheck(&outside, Tier::Host, &product, &probe).expect("产品外的目录要放行");
    recheck(&root.join("gone"), Tier::Host, &product, &probe).expect("暂时不可达不挡");

    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：启动目录收成工作区——在哪个目录启动就把哪个目录当工作区；产品根自身、不可达、非目录都如实拒绝。
/// 约束：`ambient` 与建会话的 `resolve` 共用一条阶梯，但**不探可写**：启动目录可不可写不阻塞建会话。
#[test]
pub(crate) fn ambient_workspace_takes_the_startup_directory_through_the_same_ladder() {
    use crate::units::pure::exec_model::api::Tier;
    use crate::units::workspace::api::{ambient, product_root};

    let root = crate::tests::scratch("ambient-workspace");
    let product = root.join("product");
    let startup = root.join("proj");
    std::fs::create_dir_all(&product).expect("建产品根");
    std::fs::create_dir_all(&startup).expect("建启动目录");
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    let product = product_root(&product, &probe);

    let got = ambient(&startup, Tier::Host, &product, &probe).expect("产品外的目录能当工作区");
    assert_eq!(
        got.file_name().and_then(|n| n.to_str()),
        Some("proj"),
        "{}",
        got.display()
    );

    let err = ambient(&product, Tier::Host, &product, &probe).expect_err("产品根自身要拒");
    assert!(err.contains("互相包含"), "{err}");
    let err = ambient(&root.join("gone"), Tier::Host, &product, &probe).expect_err("不可达要拒");
    assert!(err.contains("启动目录不可达"), "{err}");
    let file = root.join("note.txt");
    std::fs::write(&file, b"x").expect("建文件");
    let err = ambient(&file, Tier::Host, &product, &probe).expect_err("文件不能当工作区");
    assert!(err.contains("启动目录不是目录"), "{err}");

    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：加载复检——手改 meta 塞进策略违例的绑定要如实拒绝；只是目录不可达不算违例。
#[cfg(windows)]
#[test]
pub(crate) fn loaded_binding_is_rechecked_against_the_tier() {
    use crate::units::pure::exec_model::api::Tier;
    use crate::units::workspace::api::{product_root, recheck};
    use std::path::Path;

    let probe = crate::kernel::detail::HostProbeAdapter;
    let product = super::scratch("conduct-recheck").join("product");
    std::fs::create_dir_all(&product).expect("建产品根");
    let product = product_root(&product, &probe);
    let err = recheck(
        Path::new(r"\\server\share\proj"),
        Tier::Host,
        &product,
        &probe,
    )
    .expect_err("本机档的 UNC 绑定要拒");
    assert!(err.contains("本机档不能用网络工作区"), "{err}");
    let err = recheck(&product, Tier::Host, &product, &probe).expect_err("越界绑定要拒");
    assert!(err.contains("互相包含"), "{err}");
    recheck(&product.join("gone"), Tier::Host, &product, &probe).expect("不可达不挡开会话");
}

/// 目的：先建本机档 + 外部绑定，再切进虚拟机档——起 guest 的挂载计划与直接建虚拟机档一致。
/// 约束：改配走的是 `EditGuest::Boot`（不是创建那段），所以两条稳定根要在两处都成立。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn vm_edit_from_host_to_vm_keeps_external_workspace_mounts() {
    let engine = FakeEngine::new();
    let (mut core, product, external, guest) = core_for_external_vm("vm-ext-edit", engine.clone());
    let mut spec = work("wx", SINGLE, &["a"]);
    spec.workspace = Some(external.clone());
    let sid = core.create_work(spec).expect("本机档 + 外部工作区可建").sid;
    assert!(
        engine.specs.lock().expect("锁").is_empty(),
        "本机档不起 guest"
    );
    core.edit_session(
        &sid,
        SessionEdit {
            agents: vec![ConfigAgent {
                name: "a".to_string(),
                modules: vec!["a".to_string()],
                model: String::new(),
                chara: String::new(),
            }],
            tier: "vm".to_string(),
            guest: Some(guest),
            pins: BTreeMap::new(),
            permission: None,
            shared: None,
        },
    )
    .expect("切进虚拟机档");
    assert_two_stable_mounts(&engine, &product, &external, "wx");
}

/// 目的：删掉绑定了外部工作区的虚拟机档会话：guest 被销毁、会话目录被清掉，外部目录里的文件一个不少。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn external_workspace_vm_delete_stops_guest_and_keeps_external() {
    let engine = FakeEngine::new();
    let (mut core, product, external, guest) =
        core_for_external_vm("vm-ext-delete", engine.clone());
    let mut spec = work("wx", SINGLE, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest);
    spec.workspace = Some(external.clone());
    let sid = core.create_work(spec).expect("建会话").sid;
    let session = product.join("session").join("wx");
    assert!(session.is_dir(), "会话目录在");
    core.lifecycle
        .history_delete(&mut core.turn, &sid)
        .expect("删除会话");
    let calls = engine.calls.lock().expect("锁").clone();
    assert!(calls.iter().any(|c| c == "start:wx"), "{:?}", calls);
    assert!(
        calls.iter().any(|c| c == "stop:wx"),
        "删除会话要销毁 guest：{:?}",
        calls
    );
    assert!(!session.exists(), "会话目录要清掉");
    assert_eq!(
        std::fs::read(external.join("marker.txt")).expect("外部文件还在"),
        b"OUTSIDE",
        "删除会话不删外部工作区"
    );
}

/// 目的：起 guest 失败时，外部绑定的半成品要一并撤掉（会话目录连同 .work 不残留），外部文件不少。
/// 约束：绑定会先落 `.work-root` 并把外部文件导入成初始提交（`.work/`）；只删空目录的撤销会留下残档。
#[cfg(not(target_os = "macos"))]
#[test]
pub(crate) fn failed_guest_boot_cleans_external_workspace_store() {
    let engine = FakeEngine {
        boot_error: Some("起不来（测试注入）".to_string()),
        ..Default::default()
    };
    let (mut core, product, external, guest) = core_for_external_vm("vm-ext-boot-fail", engine);
    let mut spec = work("wx", SINGLE, &["a"]);
    spec.tier = Tier::Vm;
    spec.guest = Some(guest);
    spec.workspace = Some(external.clone());
    let err = core.create_work(spec).expect_err("起不来要如实拒绝");
    assert!(err.contains("起不来"), "{}", err);
    let session = product.join("session").join("wx");
    assert!(
        !session.exists(),
        "失败的准备不能让会话目录（含 .work）留下：{:?}",
        session
    );
    assert!(
        core.lifecycle.history_list().is_empty(),
        "历史里不该留下这条会话"
    );
    assert_eq!(
        std::fs::read(external.join("marker.txt")).expect("外部文件还在"),
        b"OUTSIDE",
        "撤销失败准备不碰外部目录"
    );
}
/// 目的：虚拟机档的外部工具路径**每次判据取用时现算**：设置里没登记就采用补齐面报的"已装在项目内"那一个。
/// 约束：补齐作业（Web 或另一个终端）装完就该立刻可用——"装完还得重启"会让用户以为补齐没成功；
///   显式登记永远优先，登记了就不看项目内那份（登记是用户的意思）。
#[test]
pub(crate) fn vm_tool_paths_follow_provisioned_packages_without_restart() {
    let probe = "data/.vm/qemu/bin/qemu-system-x86_64";
    let root = crate::tests::scratch("vm-tool-fallback");
    let exe = crate::kernel::domain::layout::vm(&root)
        .join("qemu")
        .join("bin")
        .join("qemu-system-x86_64");
    std::fs::create_dir_all(exe.parent().expect("父目录")).expect("建目录");
    std::fs::write(&exe, b"stub").expect("写占位可执行文件");
    let provision =
        test_provision_with_root(vec![installed_dep("vm.qemu.linux", probe)], Some(&root));

    // 没登记：按项目内补齐的那一个判 → 这一项成立，且文案说清是按哪条路找到的。
    let core = runtime_rooted(
        &root,
        InMemorySettings::with_tier(Tier::Host),
        Arc::clone(&provision),
    );
    let choices = core.tier_choices();
    let qemu = choices
        .vm_requirements
        .iter()
        .find(|r| r.id == "qemu")
        .expect("qemu 是一项前置");
    assert!(qemu.met, "没登记时按项目内补齐的那一个判：{}", qemu.detail);
    assert!(
        qemu.detail.contains("项目内"),
        "要说清不是 PATH 找到的：{}",
        qemu.detail
    );

    // 显式登记优先：登记了一个不在场的路径 → 这一项不成立（不去偷偷用项目内那份）。
    let registered = runtime_rooted(
        &root,
        InMemorySettings::with_qemu_path("no/such/qemu"),
        Arc::clone(&provision),
    );
    let qemu = registered
        .tier_choices()
        .vm_requirements
        .into_iter()
        .find(|r| r.id == "qemu")
        .expect("qemu 是一项前置");
    assert!(!qemu.met, "登记优先：{:?}", qemu);
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：裁决的**建议由核心 AI 给**（随 plan 那一次调用一起产出，不额外花调用）：
///   它出现在推的 Decision 事件里（前端卡片上的"建议"就是它）。
#[test]
pub(crate) fn plan_review_carries_the_core_advice() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"agree\",\"text\":\"同意\"}".to_string()],
    );
    let mut core = core_with(
        vec![module_of("a")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案：做\",\"advice\":\"我建议开工\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做完\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(execute_work("w", &["a"], "任务"))
        .unwrap()
        .sid;
    let events = advance(&mut core, &sid);
    let advice = events.iter().find_map(|e| match e {
        SessionEvent::DecisionCard { card, gate, .. } if gate == "plan_review" => {
            Some(card.message.detail.clone())
        }
        _ => None,
    });
    assert_eq!(
        advice.as_deref(),
        Some("我建议开工"),
        "方案待审的裁决要带上核心 AI 给的建议：{:?}",
        events.iter().map(|e| e.to_json()).collect::<Vec<_>>()
    );
}

#[test]
pub(crate) fn work_upload_conflict_and_safe_name() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core.create_work(work("up", SINGLE, &["a"])).unwrap().sid;
    assert!(core.work_upload(&sid, "x.txt", b"hello", false).unwrap());
    assert!(
        !core.work_upload(&sid, "x.txt", b"again", false).unwrap(),
        "同名不覆盖"
    );
    assert!(
        core.work_upload(&sid, "x.txt", b"again", true).unwrap(),
        "显式覆盖"
    );
    assert!(
        core.work_upload("ghost", "x.txt", b"x", false)
            .unwrap_err()
            .contains("无此会话")
    );
    assert!(
        core.work_upload(&sid, "../evil.txt", b"x", false)
            .unwrap_err()
            .contains("路径分隔符")
    );
}

/// 目的：回档主会话 → 各 agent 会话按**同一个回合 id** 同步截断（见 session-model.md 五）。
#[test]
pub(crate) fn rewinding_the_main_session_truncates_agent_sessions_by_turn() {
    // 从事件里读转录行（id / 文本 / 回合 id）。
    fn lines_of(evs: &[serde_json::Value]) -> Vec<(u64, String, u64)> {
        let mut out = Vec::new();
        for ev in evs {
            if let Some(ls) = ev.get("lines").and_then(|l| l.as_array()) {
                for l in ls {
                    if let Ok(v) =
                        serde_json::from_value::<crate::units::transcript::api::LineView>(l.clone())
                    {
                        out.push((v.id, v.render(), v.turn));
                    }
                }
            }
        }
        out
    }

    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我先说\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    member.insert(
        "b".to_string(),
        vec![
            "{\"type\":\"say\",\"text\":\"我也说一句\"}".to_string(),
            "{\"type\":\"agree\",\"text\":\"同意\"}".to_string(),
        ],
    );
    let mut core = core_with(
        vec![module_of("a"), module_of("b")],
        gw(
            member,
            vec![
                "{\"type\":\"tool\",\"name\":\"plan\",\"args\":{\"plan\":\"方案\",\"nodes\":[{\"id\":\"n1\",\"title\":\"做\",\"objective\":\"做\",\"assignee\":\"a\",\"deps\":[]}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"verdict\",\"args\":{\"clear\":true,\"why\":\"照他说的开工\"}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"node_verdict\",\"args\":{\"verdicts\":[{\"node\":\"n1-1\",\"ok\":true,\"note\":\"够用\"}]}}".to_string(),
                "{\"type\":\"tool\",\"name\":\"checklist\",\"args\":{\"items\":[{\"item\":\"做\",\"status\":\"pass\"}]}}".to_string(),
            ],
        ),
    );
    let sid = core
        .create_work(collab_work("w", &["a", "b"], false, "做个东西"))
        .unwrap()
        .sid;
    answer_card(&mut core, &sid, OPT_BEGIN, "").unwrap();
    let child = format!("{}--a", sid);

    // 主会话里第一条**带回合**的发言行（开场那次）——回档就保留到它。
    let (_, main_evs) = core.lifecycle.history_open(&sid).unwrap();
    let (line_id, turn) = lines_of(&main_evs)
        .into_iter()
        .find(|(_, text, t)| text.contains("[a:say]") && *t > 0)
        .map(|(id, _, t)| (id, t))
        .expect("开场该有带回合的发言行");

    let (_, before) = core.lifecycle.history_open(&child).unwrap();
    let max_before = lines_of(&before)
        .iter()
        .map(|(_, _, t)| *t)
        .max()
        .unwrap_or(0);
    assert!(
        max_before > turn,
        "回档前 agent 会话该有更晚的回合：{before:?}"
    );

    crate::tests::lifecycle::rewind(
        &mut core,
        &sid,
        crate::units::lifecycle::api::RewindTarget::Delete(line_id + 1),
    )
    .unwrap();

    let (_, after) = core.lifecycle.history_open(&child).unwrap();
    let max_after = lines_of(&after)
        .iter()
        .map(|(_, _, t)| *t)
        .max()
        .unwrap_or(0);
    assert!(
        max_after <= turn,
        "回档后 agent 会话该截到同一回合（小于等于 {turn}，实为 {max_after}）：{after:?}"
    );
}

#[test]
pub(crate) fn repeated_malformed_envelopes_each_get_a_failed_row() {
    // 非法信封**没有次数上限**了：每个都各记一条失败行回注给模型，直到它不再发信封（或用户停）。
    const N: usize = 12; // 远大于任何合理上限：证明没有次数上限
    let broken = broken_tool(&s(&["w", "work", "README.md"]));
    let mut script: Vec<String> = (0..N).map(|_| broken.clone()).collect();
    script.push("到此为止。".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "一直错",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        N,
        "每次非法信封各记一条 + 末行正文：{:?}",
        rows
    );
    assert!(
        rows.last()
            .map(|r| r.1.contains("到此为止。"))
            .unwrap_or(false),
        "模型给出正文就收尾：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 不进文本：{:?}",
        rows
    );
}

#[test]
pub(crate) fn prose_then_unclosed_tool_envelope_is_malformed_and_keeps_prose() {
    // 正文在前、坏信封在后（未闭合）：也要判 malformed，且正文照常显示、JSON 不上屏。
    let raw = "好的。{\"type\":\"tool\",\"name\":\"write\",\"args\":{\"path\":\"a\"}";
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec![
            raw.to_string(),
            "{\"type\":\"say\",\"text\":\"改好了\"}".to_string(),
        ],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "写一下",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.iter().filter(|r| r.2).count(),
        1,
        "应有一条 tool 行：{:?}",
        rows
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "JSON 绝不能上屏：{:?}",
        rows
    );
    assert!(
        rows.iter().any(|r| r.1 == "[a] 好的。"),
        "坏信封之前的正文要照常显示：{:?}",
        rows
    );
    let views = tool_views(&events);
    assert!(!views[0].ok && views[0].name == "write", "{:?}", views[0]);
    assert!(
        views[0].args.contains("\"path\":\"a\""),
        "参数尽力打捞：{:?}",
        views[0].args
    );
    assert!(
        views[0].output.contains("还差 }"),
        "未闭合要说清还差哪个字符：{}",
        views[0].output
    );
    // 历史：assistant(原文) + [工具结果]（模型下一轮能自己改）
    let h = core.single_history(&sid).unwrap();
    assert!(h.iter().any(|m| m.role == "assistant" && m.content == raw));
    assert!(
        h.iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        h
    );
}

#[test]
pub(crate) fn prose_then_tool_envelope_keeps_prose_line_and_rebuilds_identically() {
    // 同一轮里「先写正文再发信封」：正文与思维链要落进文本行，且正文行不得含 JSON；
    // 重建上下文必须与实时历史逐条一致（工具轮的文本行不另推 assistant）。
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    let n = s(&["w", "a", "n.txt"]);
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), vec![
        format!("我先看看这个文件。{{\"type\":\"tool\",\"module\":\"a\",\"name\":\"write\",\"args\":{{\"path\":\"{}\",\"content\":\"hi\"}}}}", n),
        "{\"type\":\"say\",\"text\":\"写好了\"}".to_string(),
    ]);
    let mut core = core_with_all(
        vec![module_of("a")],
        gw(member, vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "记一笔",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let rows = transcript_rows(&events);
    assert_eq!(
        rows.len(),
        4,
        "用户行 / 正文行 / tool 行 / 答复行：{:?}",
        rows
    );
    assert!(
        rows[1].1.contains("我先看看这个文件。"),
        "正文要落进文本行：{:?}",
        rows[1]
    );
    assert!(!rows[1].1.contains('{'), "正文行不得含 JSON：{:?}", rows[1]);
    assert!(
        !rows[1].2 && rows[2].2 && !rows[3].2,
        "tool 行紧随正文行：{:?}",
        rows
    );
    assert!(
        !rows[2].1.contains('{'),
        "tool 行也不含 JSON：{:?}",
        rows[2]
    );
    assert_eq!(
        io.get(&["w", "a", "n.txt"]).as_deref(),
        Some("hi"),
        "工具真的跑了"
    );

    let live_h = core.single_history(&sid).unwrap();
    assert!(
        live_h.iter().any(|m| m.role == "assistant"
            && m.content.contains("我先看看这个文件。")
            && m.content.contains("\"type\":\"tool\"")),
        "这一轮的 assistant 消息 = assistant(raw)，正文与信封都在：{:?}",
        live_h
    );
    assert!(
        live_h
            .iter()
            .any(|m| m.role == "user" && m.content.contains("[工具结果] write")),
        "{:?}",
        live_h
    );

    // 「重启」：同一份落盘历史交给新的 Core，重建后必须与实时历史逐条一致。
    drop(core);
    let mut core2 = core_with_all(
        vec![module_of("a")],
        gw(BTreeMap::new(), vec!["[]".into()]),
        Arc::new(SilentRunner),
        Arc::new(FakeCatalog::new(vec!["m".to_string()])),
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    crate::tests::lifecycle::rewind(
        &mut core2,
        &sid,
        crate::units::lifecycle::api::RewindTarget::Delete(4),
    )
    .unwrap(); // 保留全部 4 行
    let rebuilt = core2.single_history(&sid).expect("重建后应在内存里");
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| (m.role.clone(), m.content.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live_h),
        "重建上下文必须与实时历史逐条一致"
    );
}

#[test]
pub(crate) fn tool_envelope_after_prose_runs_and_json_never_shows() {
    // 正文之后跟信封：**照常执行**（没有强制收尾），显示文本取信封之外的正文，JSON 绝不进转录。
    let runner = Arc::new(RecordingRunner {
        calls: Mutex::new(Vec::new()),
        out: "ok".into(),
        ok: true,
    });
    const N: usize = 10; // 远大于任何合理上限：证明没有调用次数上限
    let mut script: Vec<String> = (0..N).map(|_| TOOL_CALL.to_string()).collect();
    script.push("到此为止。{\"type\":\"tool\",\"name\":\"scan\",\"args\":{}}".to_string());
    // 脚本替身会**重复最后一条**：末条必须是"不再发起工具调用"的正文，否则就是死循环。
    script.push("收工。".to_string());
    let mut member = BTreeMap::new();
    member.insert("a".to_string(), script);
    let mut mod_a = module_of("a");
    mod_a
        .manifest
        .tools
        .insert("scan".to_string(), decl("python tools/scan.py"));
    let mut core = core_with_runner(
        vec![mod_a],
        gw(member, vec!["[]".into()]),
        Arc::clone(&runner),
    );
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "跑满",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    assert_eq!(
        runner.calls.lock().expect("锁").len(),
        N + 1,
        "每次调用都执行（正文之后的那个信封也执行）"
    );
    let rows = transcript_rows(&events);
    assert_eq!(rows.iter().filter(|r| r.2).count(), N + 1, "{:?}", rows);
    // 信封之外的正文如实收录成一条正文行（信封本身走工具行，绝不当正文显示）。
    assert!(
        rows.iter().any(|r| !r.2 && r.1.contains("到此为止。")),
        "信封之外的正文要如实收录：{:?}",
        rows
    );
    let last = rows.last().expect("应有末行");
    assert!(
        last.1.contains("收工。") && !last.1.contains('{') && !last.1.contains("\"type\""),
        "末行是模型的收尾正文且不含 JSON：{:?}",
        last
    );
    assert!(
        !rows.iter().any(|r| r.1.contains("\"type\"")),
        "整条转录都不得出现 JSON：{:?}",
        rows
    );
}

/// 目的：一次回复里的**多个**原生调用：实时历史与重建历史必须逐条一致（含 tool_calls 与 tool_call_id）。
///   钉住这一条：实时与重建都要**每条调用各推一条回执**，不能实时只推第一条、重建却每条都推。
#[test]
pub(crate) fn native_multi_call_rebuilds_identically_to_live() {
    use crate::units::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\nA2\n");
    io.seed(&["w", "a", "b.txt"], "B1\nB2\n");
    let a = s(&["w", "a", "a.txt"]);
    let b = s(&["w", "a", "b.txt"]);
    let scripts = || {
        vec![
            NativeStep::Calls(vec![
                ToolCall {
                    id: "c1".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", a),
                },
                ToolCall {
                    id: "c2".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", b),
                },
            ]),
            NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
        ]
    };
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![scripts()]),
            seen: Default::default(),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut()
        .probe_model_tools("m")
        .expect("探测（把这条通道判成原生）");
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "读两个文件",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let live = core.single_history(&sid).unwrap();

    // 转录行：两条工具行同属一次回复（回复号 = 该回复第一条工具行的 id）。
    let views = tool_views(&events);
    assert_eq!(views.len(), 2, "两个调用两条工具行");
    assert_eq!(
        views[0].reply, views[1].reply,
        "同一回复的两条工具行必须同号"
    );
    assert_eq!(views[0].call_id, "c1", "工具行记下供应商给的调用 id");
    assert_eq!(views[1].call_id, "c2");

    // 「重启」：同一份落盘历史交给新核心，按转录重建上下文——必须与实时逐条一致。
    drop(core);
    let mut core2 = native_core(
        NativeGateway {
            scripts: Mutex::new(Vec::new()),
            seen: Default::default(),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core2.registry_mut().probe_model_tools("m").expect("探测");
    let rows = transcript_rows(&events).len() as u64;
    crate::tests::lifecycle::rewind(
        &mut core2,
        &sid,
        crate::units::lifecycle::api::RewindTarget::Delete(rows),
    )
    .unwrap();
    let rebuilt = core2.single_history(&sid).unwrap();
    let key = |h: &[Msg]| {
        h.iter()
            .map(|m| {
                (
                    m.role.clone(),
                    m.content.clone(),
                    m.tool_calls.len(),
                    m.tool_call_id.clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        key(&rebuilt),
        key(&live),
        "重建上下文必须与实时历史逐条一致（含 tool_calls / tool_call_id）"
    );
    assert_eq!(
        live.iter().filter(|m| !m.tool_calls.is_empty()).count(),
        1,
        "一次回复只推一条助手消息"
    );
}

/// 目的：读事实随工具行落盘，切换开关重建会话后与实时逐条一致（发送视图拿到的判定输入不变）。
/// 约束：正文必须同源——回放按 `tool_result_body` 现算，差一个字节就定位不到那条消息。
#[test]
pub(crate) fn read_facts_rebuild_identically_to_live() {
    use crate::units::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    io.seed(&["w", "a", "a.txt"], "A1\nA2\n");
    let a = s(&["w", "a", "a.txt"]);
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![vec![
                NativeStep::Calls(vec![
                    ToolCall {
                        id: "c1".to_string(),
                        name: "read".to_string(),
                        args_json: format!("{{\"path\":\"{}\"}}", a),
                    },
                    // 同一版本再读一次：台账按（路径, 版本, 行窗口）去重，不重复记。
                    ToolCall {
                        id: "c2".to_string(),
                        name: "read".to_string(),
                        args_json: format!("{{\"path\":\"{}\"}}", a),
                    },
                ]),
                NativeStep::Text("{\"type\":\"say\",\"text\":\"读了\"}".to_string()),
            ]]),
            seen: Default::default(),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut().probe_model_tools("m").expect("探测");
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "读 a.txt",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    let live = core.single_reads(&sid).expect("单 agent 会话");
    assert_eq!(live.len(), 1, "同版本重复读只留一条读事实");
    assert_eq!(
        (live[0].fact.from, live[0].fact.to),
        (1, 2),
        "记下读过的行窗口"
    );
    assert!(!live[0].fact.blob.is_empty(), "快照要记内容哈希");
    assert!(
        live[0].body.contains("A1"),
        "正文用于在发送视图里定位消息：{}",
        live[0].body
    );
    assert!(
        tool_views(&events)[0]
            .observations
            .iter()
            .any(|f| !f.read_facts().is_empty()),
        "工具行要带上读事实"
    );

    // 开关打开会丢掉内存会话：下一次访问按转录重建（回放必须与实时同一份台账）。
    core.set_context_freshness(&sid, true, None)
        .expect("开开关");
    core.turn
        .ensure_session(&sid, core.registry.as_ref(), core.probe.as_ref())
        .expect("重建");
    let rebuilt = core.single_reads(&sid).expect("重建后的单 agent 会话");
    assert_eq!(rebuilt, live, "回放的读事实必须与实时逐条一致");
}

/// 目的：读过的文件在别处被改后，**下一次发给模型的消息**里那条工具结果要换成过期占位符。
/// 约束：只换发送视图，转录本身逐字不动（回档 / 折叠 / 面板都按原样读）；判据门槛为保底 0%、省下的字符过 FRESHNESS_MIN_SAVED_CHARS。
///   这里的"改过"用沙箱副本对不上来构造（内存版本库里没存这份内容）；按内容比对的过期判定本身由 judge_reads 的用例钉住。
#[test]
pub(crate) fn stale_read_turns_into_a_placeholder_in_the_next_send_view() {
    use crate::units::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    // 400 行 ≈ 12K 字符：整份一次读完（< MAX_READ_CHARS），换掉后省下的字符远超 8000。
    let mut big = String::new();
    for i in 1..=400 {
        big.push_str(&format!(
            "第{:03}行 这是一段用于撑大上下文以便触发展开替换的正文。\n",
            i
        ));
    }
    io.seed(&["w", "a", "big.txt"], &big);
    let path = s(&["w", "a", "big.txt"]);
    let seen: SeenLog = Default::default();
    let mut core = native_core(
        NativeGateway {
            // 栈式发放：只给第一回合的脚本（先 read 再发言），第二回合自然落到「脚本用完了」。
            scripts: Mutex::new(vec![vec![
                NativeStep::Calls(vec![ToolCall {
                    id: "c1".to_string(),
                    name: "read".to_string(),
                    args_json: format!("{{\"path\":\"{}\"}}", path),
                }]),
                NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
            ]]),
            seen: Arc::clone(&seen),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut().probe_model_tools("m").expect("探测");
    let mut spec = work("w", SINGLE, &["a"]);
    spec.freshness = Some((true, 0));
    let sid = core.create_work(spec).unwrap().sid;

    with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "读 big.txt",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    assert_eq!(core.single_reads(&sid).expect("单 agent 会话").len(), 1);

    // 读过之后文件在别处被改：沙箱存档里对不上这份内容（判过期），下一次发送视图必须带占位符。
    with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "继续",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();

    let sent = seen
        .lock()
        .expect("锁")
        .last()
        .cloned()
        .expect("第二回合要发请求");
    let placeholder = sent
        .iter()
        .find(|m| m.content.starts_with("[已过期：") && m.content.contains("big.txt"));
    let note = placeholder.unwrap_or_else(|| {
        panic!(
            "发给模型的消息里没有过期占位符：{:?}",
            sent.iter()
                .map(|m| m.content.chars().take(60).collect::<String>())
                .collect::<Vec<_>>()
        )
    });
    assert!(
        note.content.contains("第1-400行"),
        "占位符要带行窗口：{}",
        note.content
    );

    // 转录不动：原文还在，只是发送视图换掉。
    let history = core.single_history(&sid).expect("单 agent 历史");
    assert!(
        history.iter().any(|m| m.content.contains("第001行")),
        "转录必须保留原文（占位符只换发送视图）"
    );

    let view = core
        .work
        .session_view(
            &core.turn,
            core.registry.as_ref(),
            core.probe.as_ref(),
            &sid,
        )
        .expect("会话视图");
    let stats = view.freshness_stats.expect("这一轮要留下替换统计");
    assert_eq!(stats.judged, 1, "判出 1 条过期读");
    assert_eq!(stats.replaced, 1, "换掉 1 条");
    assert!(!stats.skipped, "省下的量够付缓存失效");
    assert_eq!(stats.keep_percent, 0, "保底 0%");
    assert!(
        stats.saved_tokens > 2_000,
        "省下的 tokens：{}",
        stats.saved_tokens
    );
}

/// 目的：大文件只能分段读时，每段读各自记账、各自判过期、各自出占位符——分段读不再是盲区。
#[test]
pub(crate) fn stale_read_of_a_large_file_is_replaced_segment_by_segment() {
    use crate::units::llm::api::ToolCall;
    let hist = Arc::new(InMemoryHistory::new());
    let io = Arc::new(InMemorySysIo::new());
    // 400 行 ≈ 12K 字符；单次 read 的 14K 预算虽够，但这里显式分段读，构造 Ranges 覆盖。
    let mut big = String::new();
    for i in 1..=400 {
        big.push_str(&format!(
            "第{:03}行 这是一段用于撑大上下文以便触发展开替换的正文。\n",
            i
        ));
    }
    io.seed(&["w", "a", "big.txt"], &big);
    let path = s(&["w", "a", "big.txt"]);
    let seen: SeenLog = Default::default();
    let mut core = native_core(
        NativeGateway {
            scripts: Mutex::new(vec![vec![
                NativeStep::Calls(vec![
                    ToolCall {
                        id: "c1".to_string(),
                        name: "read".to_string(),
                        args_json: format!("{{\"path\":\"{}\",\"offset\":1,\"limit\":150}}", path),
                    },
                    ToolCall {
                        id: "c2".to_string(),
                        name: "read".to_string(),
                        args_json: format!(
                            "{{\"path\":\"{}\",\"offset\":151,\"limit\":150}}",
                            path
                        ),
                    },
                ]),
                NativeStep::Text("{\"type\":\"say\",\"text\":\"读完了\"}".to_string()),
            ]]),
            seen: Arc::clone(&seen),
        },
        Arc::clone(&hist),
        Arc::clone(&io),
    );
    core.registry_mut().probe_model_tools("m").expect("探测");
    let mut spec = work("w", SINGLE, &["a"]);
    spec.freshness = Some((true, 0));
    let sid = core.create_work(spec).unwrap().sid;

    with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "读 big.txt",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
    assert_eq!(
        core.single_reads(&sid).expect("单 agent 会话").len(),
        2,
        "两段读各记一条读事实"
    );

    with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "继续",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();

    let sent = seen
        .lock()
        .expect("锁")
        .last()
        .cloned()
        .expect("第二回合要发请求");
    let notes: Vec<String> = sent
        .iter()
        .filter(|m| m.content.starts_with("[已过期：") && m.content.contains("big.txt"))
        .map(|m| m.content.clone())
        .collect();
    assert_eq!(
        notes.len(),
        2,
        "每段各换一张占位符：{:?}",
        sent.iter()
            .map(|m| m.content.chars().take(60).collect::<String>())
            .collect::<Vec<_>>()
    );
    assert!(
        notes.iter().any(|n| n.contains("第1-150行")),
        "前段占位符：{:?}",
        notes
    );
    assert!(
        notes.iter().any(|n| n.contains("第151-300行")),
        "后段占位符：{:?}",
        notes
    );

    let history = core.single_history(&sid).expect("单 agent 历史");
    assert!(
        history.iter().any(|m| m.content.contains("第001行")),
        "转录必须保留原文（占位符只换发送视图）"
    );

    let view = core
        .work
        .session_view(
            &core.turn,
            core.registry.as_ref(),
            core.probe.as_ref(),
            &sid,
        )
        .expect("会话视图");
    let stats = view.freshness_stats.expect("这一轮要留下替换统计");
    assert_eq!(stats.judged, 2, "两段都过期");
    assert_eq!(stats.replaced, 2, "两条工具结果都换掉");
    assert!(!stats.skipped, "省下的量够付缓存失效");
}

/// 目的：一个模型都没登记时**不能建会话**——不能再用内置假模型把流程跑起来。
#[test]
fn creating_a_session_without_any_model_is_refused_and_leaves_no_history() {
    let mut core = core_with_settings(InMemorySettings::without_models());
    let err = core
        .create_work(work("no-model", SINGLE, &[]))
        .expect_err("没有模型必须拒绝");
    assert!(err.contains("还没有任何模型"), "{}", err);
    assert!(
        core.lifecycle.history_list().is_empty(),
        "失败不应留下会话记录"
    );
}

/// 目的：模型在册但核心默认没设时，没显式模型的 agent 不能建会话；错误要指名缺什么。
#[test]
fn creating_a_session_without_core_default_is_refused() {
    let mut core =
        core_with_settings_modules(InMemorySettings::without_core(), vec![module_of("a")]);
    let err = core
        .create_work(work("no-core", SINGLE, &["a"]))
        .expect_err("没有核心默认必须拒绝");
    assert!(err.contains("agent a 没有可用模型通道"), "{}", err);
    assert!(err.contains("核心 AI 未设定默认模型"), "{}", err);
    assert!(
        core.lifecycle.history_list().is_empty(),
        "失败不应留下会话记录"
    );
}

/// 目的：代理会话跑在核心默认上；没设默认就不建。
#[test]
fn creating_a_proxy_without_core_default_is_refused() {
    let mut core = core_with_settings(InMemorySettings::without_core());
    let spec = WorkSpec {
        flows: None,
        freshness: None,
        name: "no-core-proxy".to_string(),
        kind: PROXY.to_string(),
        agents: Vec::new(),
        task: None,
        delegate: false,
        permission: None,
        tier: Tier::Host,
        guest: None,
        workspace: None,
    };
    let err = core
        .create_work(spec)
        .expect_err("代理没有核心默认必须拒绝");
    assert!(err.contains("代理会话没有可用模型通道"), "{}", err);
    assert!(
        core.lifecycle.history_list().is_empty(),
        "失败不应留下会话记录"
    );
}

/// 目的：协作的成员即使都显式指定了模型，核心自己还要通道（编排 / 整理 / 验收）。
#[test]
fn creating_a_collab_without_core_default_is_refused_even_with_explicit_models() {
    let mut core = core_with_settings_modules(
        InMemorySettings::without_core(),
        vec![module_of("a"), module_of("b")],
    );
    let mut spec = work("no-core-collab", MEETING, &["a", "b"]);
    spec.task = Some("做个东西".to_string());
    for a in &mut spec.agents {
        a.model = Some("m".to_string());
    }
    let err = core
        .create_work(spec)
        .expect_err("协作没有核心默认必须拒绝");
    assert!(err.contains("核心没有可用模型通道"), "{}", err);
    assert!(
        core.lifecycle.history_list().is_empty(),
        "失败不应留下会话记录"
    );
}
