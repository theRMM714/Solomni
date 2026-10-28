//! 目的：构建能力的契约——入口在场即跳过、缺失才编、编完复验、缓存命中与失败如实报。
//! 管：本机档（宿主围栏）与虚拟机档（guest 里编）的构建用例；夹具模块的跨平台构建脚本。
//! 不管：CLI 参数解析与退出码（`diagnostics::build` 是薄壳）、模块清单校验（在 `modules.rs`）。
//! 联动：契约见 docs/builds/README.md 与 MODULE_SPEC 的「构建」；状态落 `.vm/build-state/<模块>.json`。

use crate::capabilities::builds::api::{Builds, Target};
use crate::capabilities::builds::service::BuildsService;
use crate::capabilities::modules::api::Module;
use crate::capabilities::modules::ports::ModuleSource;

/// 目的：测试用的构建能力面——包库指向夹具的 `runtimes/`、工具链事实为空；本机档用例不碰 vm。
/// 参数：`root` = 夹具产品根；`runner` = 外部进程执行（守门进程就是本程序）。
/// 返回：可调用的构建能力面。
fn builds_service(
    root: &std::path::Path,
    runner: crate::kernel::detail::process::ProcTools,
) -> BuildsService {
    use crate::capabilities::packages::api::BaseFacts;
    use std::sync::Arc;
    let packages: Arc<dyn crate::capabilities::packages::api::Packages + Send + Sync> = Arc::new(
        crate::capabilities::packages::service::PackagesService::with_base(
            Arc::new(crate::capabilities::packages::detail::FsPackages::new(
                root.join("runtimes"),
            )),
            BaseFacts::default(),
        ),
    );
    BuildsService::new(
        root.to_path_buf(),
        packages,
        crate::diagnostics::build_vm(root),
        Arc::new(crate::capabilities::provision::ProvisionService::new(root)),
        Arc::new(runner),
        Arc::new(crate::kernel::detail::host_probe::HostProbeAdapter),
        Arc::new(crate::kernel::ports::NoopLog),
        Vec::new(),
    )
}

/// 目的：按 id 从夹具模块目录里扫出模块（构建能力只吃清单事实）。
/// 参数：`root` = 夹具产品根；`id` = 模块 id。
/// 返回：那个模块。
/// 约束：扫不到就 panic——夹具写错了要立刻看见。
fn module_of(root: &std::path::Path, id: &str) -> Module {
    crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan()
    .modules
    .into_iter()
    .find(|m| m.manifest.id == id)
    .unwrap_or_else(|| panic!("场景里没有模块 {}", id))
}

/// 目的：构建契约用的夹具模块——清单只声明 `build.command: tools/build`，**平台各一份脚本**并排放
///   （与 MODULE_SPEC「构建」里模块作者的跨平台写法、随仓 indexer 一致：unix 由 sh 直接跑 `tools/build`，
///   Windows 由 cmd 按 PATHEXT 落到 `tools/build.cmd`）。
/// 参数：`module` = 模块目录（不存在就建）。
/// 返回：模块工具声明的入口在**本平台**落成的产物路径（入口判据按壳的候选认，Windows 落 `.cmd`）。
/// 约束：脚本只写 **ASCII**——cmd 按控制台代码页解释批处理字节，非 ASCII 会吞掉下一行（见 MODULE_SPEC 的「跨平台」）。
fn host_build_module(module: &std::path::Path) -> std::path::PathBuf {
    std::fs::create_dir_all(module.join("tools")).expect("建模块目录");
    std::fs::write(
        module.join("module.yaml"),
        "id: hello
brief: 构建契约用例
system: 用法：打招呼
build:
  command: tools/build
tools:
  hi:
    command: build/hello
",
    )
    .expect("写模块清单");
    // 构建命令每跑一次就往 `build/runs.txt` 记一行：`build/` 不进源指纹，所以它是"命令真的又跑了一次"的
    // 硬证据（比产物 mtime 可靠——Windows 的 `copy` 保留源文件时间戳，mtime 不前进并不等于没重编）。
    std::fs::write(
        module.join("tools").join("build"),
        "#!/bin/sh
set -e
mkdir -p build
printf '#!/bin/sh\necho HELLO-OK\n' > build/hello
chmod +x build/hello
echo run >> build/runs.txt
",
    )
    .expect("写 unix 构建脚本");
    // Windows 那份：cmd 按 PATHEXT 找到同名 `.cmd`（unix 那份它不看，反之亦然——两个文件并排就是跨平台写法）。
    std::fs::write(
        module.join("tools").join("build.cmd"),
        "@echo off\r\nif not exist build mkdir build\r\n> build\\hello.cmd echo @echo off\r\n>> build\\hello.cmd echo echo HELLO-OK\r\n>> build\\runs.txt echo run\r\n",
    )
    .expect("写 Windows 构建脚本");
    // unix 的脚本要能直接跑（产品在入口判据与执行时都按它判）。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let script = module.join("tools").join("build");
        let mut perm = std::fs::metadata(&script).expect("读权限位").permissions();
        perm.set_mode(0o755);
        std::fs::set_permissions(&script, perm).expect("加执行位");
    }
    let name = if cfg!(windows) { "hello.cmd" } else { "hello" };
    module.join("build").join(name)
}

/// 目的：构建命令真跑了几次——夹具脚本每跑一次就往 `build/runs.txt` 记一行。
/// 参数：`module` = 模块目录。
/// 返回：行数（没跑过 = 0）。
fn build_runs(module: &std::path::Path) -> usize {
    std::fs::read_to_string(module.join("build").join("runs.txt"))
        .map(|text| text.lines().filter(|l| !l.trim().is_empty()).count())
        .unwrap_or(0)
}

/// 目的：构建期望失败的命令——两个平台各自的壳语法，都只写 ASCII。
#[cfg(unix)]
const FAILING_BUILD: &str = "echo boom >&2; exit 7";
#[cfg(windows)]
const FAILING_BUILD: &str = "echo boom 1>&2 & exit /b 7";

#[test]
fn host_build_makes_the_entry_and_hits_the_cache() {
    use crate::kernel::detail::process::ProcTools;
    use crate::kernel::ports::HostProbe;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过 --build 契约");
        return;
    };
    let root = crate::tests::scratch("host-build");
    let module_dir = root.join("modules").join("hello");
    // 一个不需要编译器的"构建"：脚本写一个能直接跑的产物（两平台都跑得动，不赌宿主工具链）。
    let artifact = host_build_module(&module_dir);
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    let module = module_of(&root, "hello");
    // 第一次：真编出来（产物在场 + 能直接跑 + 状态落盘）。
    let first = builds
        .build(&module, &Target::Host, false)
        .expect("第一次构建要成功");
    assert!(!first.skipped, "第一次不该算缓存命中");
    assert!(
        artifact.is_file(),
        "产物要落进模块目录：{}",
        artifact.display()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&artifact)
            .expect("读权限")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "产物要能直接跑（拿到 {:o}）", mode);
    }
    let state = root.join(".vm").join("build-state").join("hello.json");
    assert!(state.is_file(), "状态要落盘：{}", state.display());
    // 会话语义：这条入口此时应当放行（入口判据真在、真能跑）。
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    let scanned = crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan();
    assert!(
        crate::capabilities::vm::api::missing_entries_for(
            &scanned.modules,
            std::env::consts::OS,
            &|p| { probe.is_executable(p) }
        )
        .is_empty(),
        "构建完入口判据要放行"
    );
    // 第二次：指纹没变 → 跳过（产物不被重写，构建命令也不再跑）。
    let before = std::fs::metadata(&artifact)
        .expect("读 mtime")
        .modified()
        .expect("mtime");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let second = builds
        .build(&module, &Target::Host, false)
        .expect("第二次要跳过（仍算成功）");
    assert!(second.skipped, "指纹没变要跳过");
    let after = std::fs::metadata(&artifact)
        .expect("读 mtime")
        .modified()
        .expect("mtime");
    assert_eq!(before, after, "命中缓存时不该重编（产物不该被重写）");
    assert_eq!(build_runs(&module_dir), 1, "命中缓存时构建命令不该再跑");
    // 源变了 → 真重编（构建命令又跑了一次）。
    std::fs::write(module_dir.join("tools").join("src.txt"), "changed\n").expect("改源文件");
    let third = builds
        .build(&module, &Target::Host, false)
        .expect("改源后要重编");
    assert!(!third.skipped, "源变了不该算缓存命中");
    assert_eq!(build_runs(&module_dir), 2, "源变了构建命令要真重跑");
}

#[test]
fn host_build_failure_is_honest_and_leaves_no_state() {
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过 --build 失败契约");
        return;
    };
    let root = crate::tests::scratch("host-build-fail");
    let module_dir = root.join("modules").join("bad");
    std::fs::create_dir_all(&module_dir).expect("建模块目录");
    std::fs::write(
        module_dir.join("module.yaml"),
        format!(
            "id: bad\nbrief: 构建失败用例\nsystem: 用法：失败\nbuild:\n  command: \"{}\"\ntools:\n  hi:\n    command: build/bad\n",
            FAILING_BUILD
        ),
    )
    .expect("写模块清单");
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    let err = builds
        .build(&module_of(&root, "bad"), &Target::Host, false)
        .expect_err("构建失败要如实报失败");
    assert!(err.contains("构建命令失败"), "错误要指名构建失败：{}", err);
    assert!(
        !root
            .join(".vm")
            .join("build-state")
            .join("bad.json")
            .is_file(),
        "失败不落状态（下次重试）"
    );
    // 构建期能力不在场：如实拒绝（点出缺哪个，不猜、不降级）。
    let needy = root.join("modules").join("needy");
    std::fs::create_dir_all(&needy).expect("建模块目录");
    std::fs::write(
        needy.join("module.yaml"),
        "id: needy\nbrief: 构建期能力不在场\nsystem: 用法：构建\nbuild:\n  command: tools/build\n  needs: [no-such-tool-xyz]\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
    let err = builds
        .build(&module_of(&root, "needy"), &Target::Host, false)
        .expect_err("构建期工具不在场要如实拒绝");
    assert!(
        err.contains("构建期能力"),
        "错误要说清缺的是构建期能力：{}",
        err
    );
    // 没声明 build 的模块：明确拒绝（不是静默成功）。
    let plain = root.join("modules").join("plain");
    std::fs::create_dir_all(&plain).expect("建模块目录");
    std::fs::write(
        plain.join("module.yaml"),
        "id: plain\nbrief: 没有构建段\nsystem: 用法：单干\ntools:\n  hi:\n    command: python tools/hi.py\n",
    )
    .expect("写模块清单");
    let err = builds
        .build(&module_of(&root, "plain"), &Target::Host, false)
        .expect_err("没声明 build 的模块要明确拒绝");
    assert!(err.contains("没声明 build 段"), "错误要指到声明：{}", err);
}

/// 目的：真机验收（默认跳过）：虚拟机档构建——在 guest 里编出产物，产物经挂载落回模块目录。
#[cfg(unix)]
#[test]
fn guest_build_makes_the_entry_in_the_guest() {
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
        eprintln!("[探针] 未开启虚拟机档构建的真机验收：设 SOLOMNI_VM_LIVE=1 后重跑");
        return;
    }
    let repo = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let deps = crate::capabilities::provision::api::Provision::status(
        &crate::capabilities::provision::ProvisionService::new(&repo),
    )
    .unwrap_or_default();
    // 引擎与虚拟根不在场就如实跳过（真机验收只在装齐的机器上跑）。
    let has_qemu = deps
        .iter()
        .any(|d| d.present && d.id.starts_with("vm.qemu."));
    let has_guest = repo.join(".vm").join("guest").join("debian").is_dir();
    if !has_qemu || !has_guest {
        eprintln!("[探针] 本机没有可用的 QEMU 或虚拟根 debian，跳过虚拟机档构建验收");
        return;
    }
    // 夹具根借仓库的 `.vm`（虚拟根、QEMU、包库都在那儿）；跑完删链本身，不碰仓库那份。
    let root = crate::tests::scratch("guest-build");
    std::os::unix::fs::symlink(repo.join(".vm"), root.join(".vm")).expect("链 .vm");
    let module_dir = root.join("modules").join("greet");
    std::fs::create_dir_all(&module_dir).expect("建模块目录");
    std::fs::write(
        module_dir.join("module.yaml"),
        "id: greet\nbrief: 虚拟机档构建用例\nsystem: 用法：打招呼\nbuild:\n  command: \"mkdir -p build && printf '#!/bin/sh\\necho GUEST-OK\\n' > build/hello && chmod +x build/hello\"\ntools:\n  hi:\n    command: build/hello\n",
    )
    .expect("写模块清单");
    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过虚拟机档构建验收");
        return;
    };
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    // 同一个虚拟根不能被两台 guest 同时写：真机用例之间串行（见 doubles::live_guest_lock）。
    let _live = crate::tests::doubles::live_guest_lock();
    let built = builds
        .build(
            &module_of(&root, "greet"),
            &Target::Guest("debian".to_string()),
            false,
        )
        .expect("guest 里要能编出产物");
    // 先清理：状态文件写在借来的 `.vm` 里（别留下夹具的痕迹）。
    let _ = std::fs::remove_file(root.join(".vm").join("build-state").join("greet.json"));
    let _ = std::fs::remove_file(root.join(".vm"));
    assert!(!built.skipped);
    let artifact = module_dir.join("build").join("hello");
    assert!(
        artifact.is_file(),
        "产物要经挂载落回模块目录：{}",
        artifact.display()
    );
    use std::os::unix::fs::PermissionsExt;
    let mode = std::fs::metadata(&artifact)
        .expect("读权限")
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "产物要能直接跑（拿到 {:o}）", mode);
}

/// 目的：清单事实（构建用例要拿模块对象）。
/// 参数：`root` = 夹具产品根。
/// 返回：合法模块清单。
fn scanned_modules(root: &std::path::Path) -> Vec<Module> {
    crate::capabilities::modules::detail::FsModules::new(
        root.join("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan()
    .modules
}

#[test]
fn creating_a_session_builds_the_missing_entry_first() {
    use crate::capabilities::conductor::api::{ConductorHandle, SessionOps, WorkMode};
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过创建前补编用例");
        return;
    };
    let root = crate::tests::scratch("create-build");
    let module_dir = root.join("modules").join("hello");
    let artifact = host_build_module(&module_dir);
    assert!(!artifact.is_file(), "夹具起点：入口不在场");
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    let core = crate::tests::doubles::core_with_builds(scanned_modules(&root), Arc::new(builds));
    let handle = ConductorHandle::spawn(core).expect("起核心");
    // 创建：入口不在场、模块声明了 build → 先补编，再建会话。
    let spec = crate::tests::doubles::work("w", WorkMode::Single, &["hello"]);
    let (opened, _) = handle.create_work(spec).expect("补编后要能建会话");
    assert_eq!(opened.agents.len(), 1);
    assert!(
        artifact.is_file(),
        "创建前要编出入口：{}",
        artifact.display()
    );
    assert!(
        root.join(".vm")
            .join("build-state")
            .join("hello.json")
            .is_file(),
        "构建状态要落盘（下次创建不再编）"
    );
}

#[test]
fn creating_a_session_reports_the_build_failure_and_creates_nothing() {
    use crate::capabilities::conductor::api::{ConductorHandle, SessionOps, WorkMode};
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过构建失败用例");
        return;
    };
    let root = crate::tests::scratch("create-build-fail");
    let module_dir = root.join("modules").join("bad");
    std::fs::create_dir_all(&module_dir).expect("建模块目录");
    std::fs::write(
        module_dir.join("module.yaml"),
        format!(
            "id: bad\nbrief: 构建失败用例\nsystem: 用法：失败\nbuild:\n  command: \"{}\"\ntools:\n  hi:\n    command: build/bad\n",
            FAILING_BUILD
        ),
    )
    .expect("写模块清单");
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    let core = crate::tests::doubles::core_with_builds(scanned_modules(&root), Arc::new(builds));
    let handle = ConductorHandle::spawn(core).expect("起核心");
    let spec = crate::tests::doubles::work("w", WorkMode::Single, &["bad"]);
    let err = handle.create_work(spec).expect_err("构建失败要拒绝创建");
    assert!(
        err.contains("构建失败") && err.contains("bad"),
        "错误要指名模块与构建失败：{}",
        err
    );
    assert!(
        !root
            .join(".vm")
            .join("build-state")
            .join("bad.json")
            .is_file(),
        "构建失败不落状态（下次创建还会重试）"
    );
}

/// 目的：入口在场但构建状态属于别的目标时，ensure 必须按当前目标重编。
#[test]
fn ensure_rebuilds_when_the_target_changed() {
    use crate::kernel::detail::process::ProcTools;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;

    let Some(exe) = crate::tests::doubles::built_exe() else {
        eprintln!("[探针] 未找到已构建的可执行文件，跳过目标变化用例");
        return;
    };
    let root = crate::tests::scratch("ensure-target");
    let module_dir = root.join("modules").join("hello");
    let artifact = host_build_module(&module_dir);
    let runner = ProcTools::new(
        exe,
        crate::tests::doubles::test_process_texts(),
        root.join("home"),
        Arc::new(AtomicBool::new(false)),
    );
    let builds = builds_service(&root, runner);
    let module = module_of(&root, "hello");
    builds
        .build(&module, &Target::Host, false)
        .expect("先编出宿主产物");
    assert_eq!(build_runs(&module_dir), 1, "起点只编过一次");
    // 把状态伪造成别的目标：入口仍在，但这份入口不是当前目标的产物。
    std::fs::write(
        root.join(".vm").join("build-state").join("hello.json"),
        r#"{"module":"hello","target":"debian","fingerprint":"stale"}"#,
    )
    .expect("写旧目标状态");
    let again = builds
        .ensure(&module, &Target::Host, false)
        .expect("目标变化后重编");
    assert!(!again.skipped, "目标不匹配不能算缓存命中");
    assert_eq!(build_runs(&module_dir), 2, "构建命令要真重跑一次");
    assert!(artifact.is_file(), "产物仍在：{}", artifact.display());
    let state: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(".vm").join("build-state").join("hello.json"))
            .expect("读新状态"),
    )
    .expect("状态是 JSON");
    assert_eq!(state["target"], "host", "状态要回到当前目标");
    // 随仓自带产物没有 build-state：不能因为缺状态就反复重编。
    std::fs::remove_file(root.join(".vm").join("build-state").join("hello.json")).expect("删状态");
    let prebuilt = builds
        .ensure(&module, &Target::Host, false)
        .expect("自带产物可跳过");
    assert!(prebuilt.skipped, "没有构建状态时按自带产物跳过");
    assert_eq!(build_runs(&module_dir), 2, "跳过时构建命令不再跑");
}
