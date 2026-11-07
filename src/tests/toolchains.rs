//! 目的：启动期基础工具链解析的单元测试——项目 bin 前置、缺失回退、路径安全与目标匹配。
//! 管：只测 `compose` 的纯逻辑与 `active.json` 形状，不读真实工具链、不改进程环境。
//! 不管：安装器（provision 能力的 `kind: toolchain`）、真实进程围栏与 guest 工具链。
//! 联动：被测实现见 src/entry/toolchains.rs；启动期调用见 src/main.rs。

use crate::capabilities::packages::api::{BaseFacts, HostFallback};
use crate::entry::toolchains::{active_roots, compose, report_lines, target};

type ToolSpec<'a> = (&'a str, &'a str, &'a [&'a str], &'a [(&'a str, &'a str)]);

fn active(target: &str, profile: &[&str], tools: &[ToolSpec<'_>]) -> String {
    let tools = tools
        .iter()
        .map(|(id, version, bins, env)| {
            let bins = bins
                .iter()
                .map(|b| format!("\"{}\"", b))
                .collect::<Vec<_>>()
                .join(",");
            let env = env
                .iter()
                .map(|(k, v)| format!("\"{}\":\"{}\"", k, v))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                "{{\"id\":\"{}\",\"version\":\"{}\",\"bin\":[{}],\"env\":{{{}}}}}",
                id, version, bins, env
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let profile = profile
        .iter()
        .map(|p| format!("\"{}\"", p))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"target\":\"{}\",\"profile\":[{}],\"tools\":[{}]}}",
        target, profile, tools
    )
}

/// 目的：基础工具链装好后的登记与启动期读取是同一份事实：安装（provision 写 active.json）→
///   启动期（entry 读同一份）把项目 bin 前置到 PATH；缺 bin 的工具如实算缺，不假装装上。
#[test]
pub(crate) fn installed_toolchains_are_registered_and_read_back_at_startup() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-register");
    // 两个工具：python 的两个 bin 目录都在场；node 的安装目录不在场（缺件）。
    let py = format!(".tools/toolchains/{}/python/9.9.9", target);
    std::fs::create_dir_all(root.join(&py).join("bin")).unwrap();
    std::fs::create_dir_all(root.join(&py).join("site")).unwrap();
    let deps = [
        toolchain_dep("toolchain.python", target, &py, &["bin", "site"], "python"),
        toolchain_dep(
            "toolchain.node",
            target,
            ".tools/toolchains/x/node/1",
            &["bin"],
            "node",
        ),
    ];
    crate::capabilities::provision::sync_toolchain_active(&root, &deps).expect("登记 active.json");
    let file = root.join(".tools").join("toolchains").join("active.json");
    let raw = std::fs::read_to_string(&file).expect("active.json 要在场");
    let c = compose(&root, &raw, std::ffi::OsStr::new(""), target).expect("启动期要能读");
    assert_eq!(
        c.active,
        vec![("python".to_string(), "9.9.9".to_string())],
        "只登记 bin 目录都在场的工具"
    );
    assert_eq!(c.missing, vec!["node".to_string()], "缺件如实算缺");
    let first = std::env::split_paths(&c.path)
        .next()
        .expect("PATH 有第一段");
    assert_eq!(first, root.join(&py).join("bin"), "项目 bin 要前置到 PATH");
    let _ = std::fs::remove_dir_all(&root);
}

/// 目的：造一条 kind: toolchain 的清单条目（字段形状与 dependencies.yaml 一致）。
fn toolchain_dep(
    id: &str,
    target: &str,
    install: &str,
    bins: &[&str],
    tool: &str,
) -> crate::capabilities::provision::Dep {
    crate::capabilities::provision::Dep {
        id: id.to_string(),
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        install: install.to_string(),
        probe: format!("{}/{}", install, bins.first().copied().unwrap_or("bin")),
        source: "toolchain.python.3.14.9.windows-x86_64".to_string(),
        kind: "toolchain".to_string(),
        guest: None,
        toolchain: Some(crate::capabilities::provision::ToolchainSpec {
            tool: tool.to_string(),
            version: "9.9.9".to_string(),
            target: Some(target.to_string()),
            bins: bins.iter().map(|b| b.to_string()).collect(),
            wrappers: false,
        }),
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    }
}

#[test]
pub(crate) fn report_lines_name_project_missing_host_fallback_and_gap() {
    let mut fallback = std::collections::BTreeMap::new();
    fallback.insert(
        "python".to_string(),
        HostFallback {
            path: std::path::PathBuf::from("C:/host/python.exe"),
            fenced: true,
            why: String::new(),
        },
    );
    fallback.insert(
        "node".to_string(),
        HostFallback {
            path: std::path::PathBuf::from("C:/host/node.exe"),
            fenced: false,
            why: "目录属主不是当前用户".to_string(),
        },
    );
    let base = BaseFacts {
        active: Vec::new(),
        fallback,
    };
    let text = report_lines(&base).join("\n");
    assert!(text.contains("项目内未安装：python、node、cc"), "{}", text);
    assert!(
        text.contains("宿主回退 python") && text.contains("围栏可用"),
        "{}",
        text
    );
    assert!(
        text.contains("宿主回退 node") && text.contains("围栏不可授权"),
        "{}",
        text
    );
    assert!(text.contains("宿主回退 cc：宿主 PATH 也没有"), "{}", text);
    assert!(
        text.contains("node start.js --provision --ensure=toolchain --yes"),
        "{}",
        text
    );
}

#[test]
pub(crate) fn project_bins_come_first_and_missing_profile_entries_are_reported() {
    let Some(target) = target() else {
        eprintln!("[探针] 当前平台没有工具链条目，跳过");
        return;
    };
    let root = crate::tests::scratch("toolchains-compose");
    std::fs::create_dir_all(root.join("py-bin")).unwrap();
    let raw = active(
        target,
        &["python", "node"],
        &[("python", "3.14.9", &["py-bin"], &[("TEST_ROOT", "/x")])],
    );
    let host = root.join("host-bin");
    let current = std::env::join_paths([host.as_path()]).unwrap();
    let c = compose(&root, &raw, &current, target).expect("应能合成");
    let parts: Vec<_> = std::env::split_paths(&c.path).collect();
    assert_eq!(parts.first(), Some(&root.join("py-bin")));
    assert!(parts.iter().any(|p| p == &host));
    assert_eq!(c.active, vec![("python".to_string(), "3.14.9".to_string())]);
    assert_eq!(c.missing, vec!["node".to_string()]);
    assert!(c.env.contains(&("TEST_ROOT".to_string(), "/x".to_string())));
}

#[test]
pub(crate) fn missing_bin_is_reported_and_not_put_on_path() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-missing-bin");
    let raw = active(
        target,
        &["python"],
        &[("python", "3.14.9", &["no-such-bin"], &[])],
    );
    let c = compose(&root, &raw, std::ffi::OsStr::new(""), target).expect("应能合成");
    assert!(c.active.is_empty());
    assert_eq!(c.missing, vec!["python".to_string()]);
    assert!(c.warnings.iter().any(|w| w.contains("不在场")));
}

#[test]
pub(crate) fn unsafe_bin_paths_are_refused() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-unsafe");
    let raw = active(
        target,
        &["python"],
        &[("python", "3.14.9", &["../escape"], &[])],
    );
    let c = compose(&root, &raw, std::ffi::OsStr::new(""), target).expect("应能合成");
    assert!(c.active.is_empty());
    assert!(c.warnings.iter().any(|w| w.contains("不安全")));
}

#[test]
pub(crate) fn target_mismatch_is_an_error() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-target");
    let raw = active("not-a-target", &[], &[]);
    let err = compose(&root, &raw, std::ffi::OsStr::new(""), target).unwrap_err();
    assert!(err.contains("目标不符"), "{}", err);
}

#[test]
pub(crate) fn malformed_active_json_is_an_error() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-json");
    let err = compose(&root, "{not json", std::ffi::OsStr::new(""), target).unwrap_err();
    assert!(err.contains("active.json 非法"), "{}", err);
}

#[test]
pub(crate) fn compose_does_not_touch_the_process_environment() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-pure");
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let raw = active(
        target,
        &["python"],
        &[("python", "1", &["bin"], &[("X", "Y")])],
    );
    let before = std::env::var_os("PATH");
    let _ = compose(&root, &raw, std::ffi::OsStr::new(""), target).unwrap();
    assert_eq!(std::env::var_os("PATH"), before);
}

#[test]
pub(crate) fn active_roots_return_the_install_root_not_just_bin() {
    let Some(target) = target() else { return };
    let root = crate::tests::scratch("toolchains-roots");
    let bin = root
        .join(".tools")
        .join("toolchains")
        .join("py")
        .join("bin");
    std::fs::create_dir_all(&bin).unwrap();
    let active_path = root.join(".tools").join("toolchains").join("active.json");
    std::fs::create_dir_all(active_path.parent().unwrap()).unwrap();
    std::fs::write(
        &active_path,
        active(
            target,
            &["python"],
            &[("python", "1", &[".tools/toolchains/py/bin"], &[])],
        ),
    )
    .unwrap();
    assert_eq!(
        active_roots(&root),
        vec![root.join(".tools").join("toolchains").join("py")]
    );
}
