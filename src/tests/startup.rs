//! 目的：启动门与首次引导——环境检查通过后才能进入 CLI / Web。
//! 管：缺失原因分类、y/n 默认项、本次降级 / 重设 / 退出的决策与写盘结果。
//! 不管：真实权限写入与工具进程（在 kernel）；CLI / Web 的渲染与网络。
//! 联动：启动报告与 `src/main.rs` 的 `FenceReport`；语义见 `docs/kernel/README.md`。

use super::prelude::*;
use crate::capabilities::registry::api::AppSettings;
use crate::capabilities::registry::api::SandboxEnforceLevel::{BestEffort, Require};
use crate::capabilities::registry::detail::yaml_settings::YamlSettingsStore;
use crate::capabilities::registry::ports::SettingsStore;
use crate::kernel::api::SandboxRequirements;
use crate::kernel::detail::confine::Capability;
use crate::{GateOutcome, MissingCause, MissingPart};
use std::collections::BTreeMap;
use std::io::Cursor;

fn cap(fs: bool, net: bool, tree: bool) -> Capability {
    Capability {
        fs,
        net,
        tree,
        note: "测试".to_string(),
    }
}

fn all_required() -> SandboxRequirements {
    SandboxRequirements {
        filesystem: true,
        network: true,
        process_tree: true,
        env_whitelist: true,
    }
}

fn core() -> Conductor {
    core_with(vec![module_of("a")], gw(BTreeMap::new(), Vec::new()))
}

fn ask(input: &str, default_yes: bool) -> (bool, String) {
    let mut reader = Cursor::new(input.as_bytes().to_vec());
    let mut writer = Vec::new();
    let answer = crate::ask_yes_no(&mut reader, &mut writer, "问题", default_yes);
    (answer, String::from_utf8(writer).expect("UTF-8"))
}

#[test]
fn fence_write_env_overrides_settings() {
    assert!(crate::authorize(Some("1"), false), "env=1 必须覆盖设置");
    assert!(!crate::authorize(Some("0"), true), "env=0 必须覆盖设置");
    assert!(crate::authorize(None, true), "没有 env 时按设置");
    assert!(!crate::authorize(None, false));
    assert!(crate::authorize(Some("true"), true), "非 1/0 不覆盖设置");
}

#[test]
fn missing_parts_separate_authorization_from_capability() {
    // 本机能力齐全，但 Windows 未授权：两项都是“未授权”，不是“机制不可用”。
    let parts = crate::missing_parts(&all_required(), &cap(true, true, true), false);
    #[cfg(windows)]
    assert_eq!(
        parts,
        vec![
            MissingPart {
                name: "文件系统围栏",
                cause: MissingCause::Unauthorized
            },
            MissingPart {
                name: "网络围栏",
                cause: MissingCause::Unauthorized
            },
        ],
        "Windows 未授权是授权问题，不是能力问题"
    );
    #[cfg(not(windows))]
    assert!(parts.is_empty(), "Unix 的围栏在守门进程里自足");

    let parts = crate::missing_parts(&all_required(), &cap(false, false, false), true);
    assert_eq!(parts.len(), 3, "fs / net / tree 都要如实报缺");
    assert!(
        parts.iter().all(|p| p.cause == MissingCause::Unsupported),
        "本机能力缺失不能被说成未授权"
    );
}

#[test]
fn ask_yes_no_uses_safe_defaults_and_parses_answers() {
    assert!(ask("\n", true).0, "空行 = 默认项");
    assert!(!ask("\n", false).0, "空行 = 默认项");
    assert!(ask("y\n", false).0);
    assert!(ask("YES\n", false).0);
    assert!(!ask("n\n", true).0);
    assert!(!ask("no\n", true).0);
    assert!(
        ask("随便\n", true).0,
        "无法识别的回答按默认项，不能把不安全路径当答案"
    );
    assert!(ask("", true).1.contains("[Y/n]"), "提示要写明默认项");
}

#[test]
fn write_acl_and_lower_enforce_persist_through_registry() {
    let mut core = core();
    crate::write_acl_setting(core.registry_mut(), true).expect("写授权");
    assert!(
        core.registry()
            .app_settings()
            .sandbox
            .policy
            .windows
            .write_acl,
        "授权要进设置"
    );
    crate::lower_enforce_to_best_effort(core.registry_mut()).expect("降强制等级");
    let app = core.registry().app_settings();
    for (name, level) in [
        ("filesystem", app.sandbox.policy.enforce.filesystem),
        ("network", app.sandbox.policy.enforce.network),
        ("process_tree", app.sandbox.policy.enforce.process_tree),
        ("env_whitelist", app.sandbox.policy.enforce.env_whitelist),
    ] {
        assert_eq!(level, BestEffort, "{name} 应降为 best_effort");
    }
}

#[cfg(windows)]
#[test]
fn first_run_authorization_writes_once_and_restarts() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    let mut reader = Cursor::new(b"y\n".to_vec());
    let mut writer = Vec::new();
    let wrote = crate::maybe_onboard_windows(
        &missing,
        false,
        true,
        None,
        None,
        &cap(true, true, true),
        core.registry_mut(),
        &mut reader,
        &mut writer,
    )
    .expect("写授权");
    assert!(wrote, "首次选授权必须写盘并重启");
    assert!(
        core.registry()
            .app_settings()
            .sandbox
            .policy
            .windows
            .write_acl,
        "授权要落进设置"
    );
}

#[test]
fn first_run_decline_does_not_write_best_effort() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    let mut reader = Cursor::new(b"n\n".to_vec());
    let mut writer = Vec::new();
    let wrote = crate::maybe_onboard_windows(
        &missing,
        false,
        true,
        None,
        None,
        &cap(true, true, true),
        core.registry_mut(),
        &mut reader,
        &mut writer,
    )
    .expect("不写盘不是错误");
    assert!(!wrote, "首次选不启用不能自动写 best_effort");
    let app = core.registry().app_settings();
    assert!(!app.sandbox.policy.windows.write_acl);
    assert_eq!(
        app.sandbox.policy.enforce.filesystem,
        crate::capabilities::registry::api::SandboxEnforceLevel::Require,
        "选不启用不能偷偷把强制等级降掉"
    );
}

#[test]
fn gate_without_frontend_exits_instead_of_starting() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    let mut reader = Cursor::new(Vec::new());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        true,
        false,
        None,
        None,
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(
        matches!(out, GateOutcome::Exit(1)),
        "没有可回答的前端必须退出，不能进入会话"
    );
}

#[test]
fn gate_degrade_is_per_process_and_does_not_write_settings() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    let mut reader = Cursor::new(b"y\n".to_vec());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        true,
        true,
        None,
        None,
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(matches!(out, GateOutcome::Proceed));
    let app = core.registry().app_settings();
    assert!(!app.sandbox.policy.windows.write_acl, "本次降级不许落盘");
    assert_eq!(
        app.sandbox.policy.enforce.filesystem,
        crate::capabilities::registry::api::SandboxEnforceLevel::Require,
        "本次降级不改设置里的强制等级"
    );
}

#[test]
fn gate_reconfigure_can_lower_enforce_when_capability_is_missing() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unsupported,
    }];
    // 不降级 -> 重设 -> 同意降 best_effort。
    let mut reader = Cursor::new(b"n\ny\ny\n".to_vec());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        true,
        true,
        None,
        None,
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(matches!(out, GateOutcome::Restart));
    assert_eq!(
        core.registry()
            .app_settings()
            .sandbox
            .policy
            .enforce
            .filesystem,
        BestEffort
    );
}

#[test]
fn gate_reconfigure_refuses_to_write_when_env_overrides_authorization() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    // 不降级 -> 重设；授权被 SOLOMNI_FENCE_WRITE 覆盖，不能写无效配置。
    let mut reader = Cursor::new(b"n\ny\n".to_vec());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        true,
        true,
        None,
        Some("0"),
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(matches!(out, GateOutcome::Exit(0)));
    assert!(
        !core
            .registry()
            .app_settings()
            .sandbox
            .policy
            .windows
            .write_acl,
        "env 覆盖时写设置不会生效，不能假装修好了"
    );
}

#[test]
fn gate_refuse_skips_degrade_but_allows_reconfigure() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    // on_unavailable=refuse：不提供降级，直接问重设；重设后再确认授权。
    let mut reader = Cursor::new(b"y\ny\n".to_vec());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        false,
        true,
        None,
        None,
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(matches!(out, GateOutcome::Restart));
    assert!(
        core.registry()
            .app_settings()
            .sandbox
            .policy
            .windows
            .write_acl,
        "refuse 只禁止本次降级，不禁止重设"
    );
}

#[test]
fn gate_forced_strict_exits_without_prompting() {
    let mut core = core();
    let missing = vec![MissingPart {
        name: "文件系统围栏",
        cause: MissingCause::Unauthorized,
    }];
    let mut reader = Cursor::new(Vec::new());
    let mut writer = Vec::new();
    let out = crate::run_gate(
        &missing,
        true,
        true,
        Some("0"),
        None,
        core.registry_mut(),
        &mut reader,
        &mut writer,
    );
    assert!(
        matches!(out, GateOutcome::Exit(1)),
        "测试/CI 注入严格模式时不能偷偷启动"
    );
}

#[test]
fn require_without_write_acl_is_rejected_on_windows() {
    let mut app = AppSettings::default();
    app.sandbox.policy.enforce.filesystem = Require;
    app.sandbox.policy.enforce.network = Require;
    app.sandbox.policy.windows.write_acl = false;
    #[cfg(windows)]
    assert!(
        crate::capabilities::registry::domain::providers::validate_sandbox_authorization(
            &app.sandbox.policy
        )
        .is_err(),
        "Windows 上 require + 不授权写 ACL 是不可能满足的配置"
    );
    #[cfg(not(windows))]
    assert!(
        crate::capabilities::registry::domain::providers::validate_sandbox_authorization(
            &app.sandbox.policy
        )
        .is_ok(),
        "Unix 的围栏自足，write_acl 不参与"
    );

    // best_effort + 不授权：合法（用户显式选择无围栏）。
    app.sandbox.policy.enforce.filesystem = BestEffort;
    app.sandbox.policy.enforce.network = BestEffort;
    assert!(
        crate::capabilities::registry::domain::providers::validate_sandbox_authorization(
            &app.sandbox.policy
        )
        .is_ok()
    );
    // require + 授权：合法。
    app.sandbox.policy.enforce.filesystem = Require;
    app.sandbox.policy.windows.write_acl = true;
    assert!(
        crate::capabilities::registry::domain::providers::validate_sandbox_authorization(
            &app.sandbox.policy
        )
        .is_ok()
    );
    // 全局网络开关打开后，网络围栏不再 require。
    app.sandbox.policy.enforce.filesystem = BestEffort;
    app.sandbox.policy.enforce.network = Require;
    app.sandbox.policy.network = true;
    app.sandbox.policy.windows.write_acl = false;
    assert!(
        crate::capabilities::registry::domain::providers::validate_sandbox_authorization(
            &app.sandbox.policy
        )
        .is_ok()
    );
}

#[cfg(windows)]
#[test]
fn set_app_settings_rejects_require_without_write_acl() {
    let mut core = core();
    let mut app = core.registry().app_settings();
    app.sandbox.policy.enforce.filesystem = Require;
    app.sandbox.policy.enforce.network = Require;
    app.sandbox.policy.windows.write_acl = false;
    let err = core.registry_mut().set_app_settings(app).unwrap_err();
    assert!(err.contains("write_acl"), "{err}");
}

#[test]
fn set_app_settings_accepts_best_effort_without_write_acl() {
    let mut core = core();
    let mut app = core.registry().app_settings();
    app.sandbox.policy.enforce.filesystem = BestEffort;
    app.sandbox.policy.enforce.network = BestEffort;
    app.sandbox.policy.windows.write_acl = false;
    core.registry_mut()
        .set_app_settings(app)
        .expect("best_effort + 不授权是合法组合");
}

#[test]
fn existing_settings_file_must_be_self_consistent() {
    let dir = crate::tests::scratch("settings-authorization");
    let store = YamlSettingsStore::new(
        dir.join("providers.yaml"),
        dir.join("models.yaml"),
        dir.join("settings.yaml"),
        dir.join("agents.yaml"),
    );
    std::fs::write(
        dir.join("settings.yaml"),
        "sandbox:\n  policy:\n    enforce:\n      filesystem: require\n      network: require\n    windows:\n      write_acl: false\n",
    )
    .expect("写 settings.yaml");
    let loaded = store.load();
    #[cfg(windows)]
    {
        let err = loaded.expect_err("Windows 上应拒绝矛盾配置");
        assert!(err.contains("settings.yaml 配置非法"), "{err}");
        assert!(err.contains("write_acl"), "{err}");
    }
    #[cfg(not(windows))]
    assert!(loaded.is_ok(), "Unix 上 write_acl 不参与");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn missing_settings_file_still_loads_defaults_for_first_run() {
    let dir = crate::tests::scratch("settings-first-run");
    let store = YamlSettingsStore::new(
        dir.join("providers.yaml"),
        dir.join("models.yaml"),
        dir.join("settings.yaml"),
        dir.join("agents.yaml"),
    );
    let loaded = store.load().expect("首启没有 settings.yaml 不能拒绝");
    assert!(!loaded.app.sandbox.policy.windows.write_acl);
    let _ = std::fs::remove_dir_all(&dir);
}
