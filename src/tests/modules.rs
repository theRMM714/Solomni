//! 目的：模块能力的测试——清单、契约校验与启停有效值。
//! 管：模块清单与启停的用例。
//! 不管：清单来源机制（FsModules）的契约（在 `detail.rs`）；常驻服务的运行态（在 `residents.rs`）。
//! 联动：替身见 `doubles.rs`；被测实现在 `src/capabilities/modules/`。
use super::prelude::*;

#[test]
pub(crate) fn build_decl_is_validated_and_survives_the_scan() {
    use crate::capabilities::modules::api::{check_build, BuildDecl};
    let mut m = module_of("idx").manifest;
    assert!(check_build(&m).is_ok(), "没写 build 段是合法的");
    m.build = Some(BuildDecl {
        command: String::new(),
        needs: Vec::new(),
        net: false,
    });
    assert!(check_build(&m).is_err(), "有 build 段但没写 command = 拒收");
    m.build = Some(BuildDecl {
        command: "make".to_string(),
        needs: vec!["cc".to_string(), "cc".to_string()],
        net: false,
    });
    assert!(check_build(&m).is_err(), "needs 重复 = 拒收");
    m.build = Some(BuildDecl {
        command: "make".to_string(),
        needs: vec!["C++".to_string()],
        net: false,
    });
    assert!(check_build(&m).is_err(), "能力名不合法 = 拒收");
    m.build = Some(BuildDecl {
        command: "make all".to_string(),
        needs: vec!["cc".to_string()],
        net: true,
    });
    assert!(check_build(&m).is_ok(), "{}", "正常的构建声明要过");
}

#[test]
pub(crate) fn shipped_indexer_declares_its_build() {
    // 随仓的 indexer 是编译型模块：`build:` 段必须被扫出来（产品据此能替它编）。
    let roster = crate::capabilities::modules::detail::FsModules::new(
        PathBuf::from("modules"),
        crate::capabilities::tools::api::names(),
    )
    .scan();
    let idx = roster
        .modules
        .iter()
        .find(|m| m.manifest.id == "indexer")
        .expect("随仓有 indexer");
    let b = idx.manifest.build.as_ref().expect("indexer 要声明 build");
    assert!(b.command.contains("indexer.cpp"), "{}", b.command);
    assert!(b.command.contains("build/indexer"), "{}", b.command);
    assert_eq!(b.needs, vec!["cc".to_string()]);
    assert!(!b.net, "构建不需要出网");
}
#[test]
pub(crate) fn local_entry_only_accepts_relative_paths() {
    use crate::capabilities::modules::api::local_entry;
    assert_eq!(local_entry("build/indexer build"), Some("build/indexer"));
    assert_eq!(local_entry("./tools/run.sh"), Some("./tools/run.sh"));
    // Windows 写法也认（相对路径用反斜杠）。
    assert_eq!(
        local_entry("build\\indexer.exe build"),
        Some("build\\indexer.exe")
    );
    assert_eq!(
        local_entry("python tools/scan.py"),
        None,
        "裸名走 PATH / 运行能力"
    );
    assert_eq!(local_entry("indexer build"), None);
    assert_eq!(
        local_entry("/usr/bin/x build"),
        None,
        "绝对路径不在模块目录里找"
    );
    assert_eq!(local_entry("C:\\tools\\x.exe"), None);
    assert_eq!(local_entry("   "), None);
}
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
    let roster = crate::capabilities::modules::detail::FsModules::new(
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
pub(crate) fn module_runtimes_are_validated() {
    let mut m = module_of("a");
    m.manifest.runtimes = vec!["python".to_string(), "cc".to_string()];
    assert!(crate::capabilities::modules::api::check_runtimes(&m.manifest).is_ok());
    m.manifest.runtimes = vec!["Python".to_string()];
    assert!(
        crate::capabilities::modules::api::check_runtimes(&m.manifest).is_err(),
        "大写不合法"
    );
    m.manifest.runtimes = vec!["python".to_string(), "python".to_string()];
    assert!(
        crate::capabilities::modules::api::check_runtimes(&m.manifest)
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
        crate::capabilities::modules::api::check_tools(
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
        let why = crate::capabilities::modules::api::check_tools(
            &m.manifest,
            &crate::capabilities::tools::api::names(),
        )
        .unwrap_err();
        assert!(why.contains("保留名"), "内置工具名要拒收：{}", why);
        m.manifest.tools.remove(name);
    }
}
#[test]
pub(crate) fn effective_enablement_is_override_then_author_default() {
    let mut m = module_of("a");
    m.manifest.services.insert(
        "s".to_string(),
        crate::capabilities::modules::api::ServiceDecl {
            adapter: "fake".to_string(),
            command: "run".to_string(),
            desc: String::new(),
            enabled: true,
            options: BTreeMap::new(),
        },
    );
    let modules = test_modules_store(
        Arc::new(VecSource(vec![m])),
        Arc::new(InMemoryModuleState::new()),
    );
    assert!(modules.module_enabled("a"), "作者默认启用");
    assert!(modules.service_enabled("a", "s"), "服务作者默认启用");
    modules.set_module_enabled("a", false).expect("停用模块");
    assert!(!modules.module_enabled("a"));
    assert!(
        !modules.service_enabled("a", "s"),
        "模块停用 ⇒ 服务有效停用"
    );
    modules.set_module_enabled("a", true).expect("启用模块");
    modules
        .set_service_enabled("a", "s", false)
        .expect("停用服务");
    assert!(modules.module_enabled("a"));
    assert!(!modules.service_enabled("a", "s"));
    assert!(
        modules.set_module_enabled("nope", true).is_err(),
        "未知模块要报错"
    );
    assert!(
        modules.set_service_enabled("a", "nope", true).is_err(),
        "未知服务要报错"
    );
}

#[test]
pub(crate) fn author_can_ship_a_module_disabled() {
    let mut m = module_of("a");
    m.manifest.enabled = false;
    let modules = test_modules(Arc::new(VecSource(vec![m])));
    assert!(!modules.module_enabled("a"), "作者默认停用");
}

#[test]
pub(crate) fn overrides_survive_a_reload() {
    let store = Arc::new(InMemoryModuleState::new());
    let source = Arc::new(VecSource(vec![module_of("a")]));
    let first = test_modules_store(
        Arc::clone(&source) as Arc<dyn ModuleSource + Send + Sync>,
        Arc::clone(&store)
            as Arc<dyn crate::capabilities::modules::ports::ModuleStateStore + Send + Sync>,
    );
    first.set_module_enabled("a", false).expect("停用");
    let second = test_modules_store(
        source,
        store as Arc<dyn crate::capabilities::modules::ports::ModuleStateStore + Send + Sync>,
    );
    assert!(!second.module_enabled("a"), "覆盖要落盘并在下次装配时载入");
}

#[test]
pub(crate) fn ops_exposes_enablement_reads() {
    let (_handle, ops) = crate::tests::ops_with(vec![module_of("a")], vec!["[]"]);
    assert!(ops.modules.module_enabled("a").expect("读模块启用"));
    assert!(!ops
        .modules
        .service_enabled("a", "nope")
        .expect("读服务启用"));
}
