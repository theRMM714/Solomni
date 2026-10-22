//! 目的：启动期基础工具链解析的单元测试——项目 bin 前置、缺失回退、路径安全与目标匹配。
//! 管：只测 `compose` 的纯逻辑与 `active.json` 形状，不读真实工具链、不改进程环境。
//! 不管：安装器（setup-toolchains.js）、真实进程围栏与 guest 工具链。
//! 联动：被测实现见 src/entry/toolchains.rs；启动期调用见 src/main.rs。

use crate::entry::toolchains::{active_roots, compose, target};

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
