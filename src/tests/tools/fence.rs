//! 目的：围栏落点的**必要性判据**与"必要落点授不上"这条路的纯逻辑（真实 ACL 由平台探针验）。
//! 管：`FencePart` 的必要 / 可选判据与文案、`fence_ask` 的选项集与消息、`FencePrep` 的分流。
//! 不管：真实 ACL 的写与撤（Windows 的 confine 探针）、围栏在真机上的行为（`--fence-live` 的 ProcTools 用例）。
//! 联动：判据写进 docs/tools/README.md 五；卡片契约见 docs/session/session-model.md 的「请用户裁决」。

use crate::kernel::detail::confine::FencePrep;
use crate::kernel::domain::fence::{
    fence_ask, FenceBlocked, FencePart, OPT_FENCE_ABORT, OPT_FENCE_UNFENCED,
};
use std::path::PathBuf;

/// 一个必要落点授不上的现场。
fn blocked(part: FencePart, path: &str) -> FenceBlocked {
    FenceBlocked {
        part,
        path: PathBuf::from(path),
        why: "写 DACL 失败（错误码 5）".to_string(),
    }
}

/// 必要 / 可选的**判据只有一处**：解释器目录、模块目录、工作目录、私有沙箱、其余数据边界、
/// 容器身份与授权台账都是必要落点；用户显式授权的只读根与父目录的只读属性是可选落点。
#[test]
fn only_the_authorized_roots_and_parent_stat_are_optional() {
    for part in [
        FencePart::Interpreter,
        FencePart::Module,
        FencePart::Cwd,
        FencePart::Sandbox,
        FencePart::DataBoundary,
        FencePart::ContainerIdentity,
        FencePart::Ledger,
    ] {
        assert!(part.necessary(), "{:?} 缺了这次命令就跑不起来", part);
        assert!(!part.label().is_empty() && !part.fix().is_empty());
    }
    for part in [FencePart::AuthorizedRead, FencePart::Parent] {
        assert!(
            !part.necessary(),
            "{:?} 授不上只记事实，不牵动整次执行",
            part
        );
    }
}

/// 必要落点授不上时的选项集：两条都**真能执行**（本轮无围栏跑一次 / 放弃这次调用），
/// 且消息写清哪一环、哪个目录、缺什么前提、怎么补。
#[test]
fn fence_ask_offers_two_real_options_with_an_actionable_message() {
    let fact = blocked(FencePart::Interpreter, "C:/nvm4w/nodejs");
    let ask = fence_ask(&fact, "a", true).expect("无围栏能跑就给两条选项");
    assert_eq!(ask.role, "tools", "谁在问：工具层");
    assert_eq!(ask.name, "a");
    let ids: Vec<String> = ask.options.iter().map(|(id, _)| id.clone()).collect();
    assert_eq!(
        ids,
        vec![OPT_FENCE_UNFENCED.to_string(), OPT_FENCE_ABORT.to_string()],
        "选项 id 是契约"
    );
    assert!(ask.options.iter().all(|(_, label)| !label.is_empty()));
    for want in ["解释器安装目录", "C:/nvm4w/nodejs", "错误码 5", "怎么补"] {
        assert!(
            ask.detail.contains(want),
            "详情要写清 {}：{}",
            want,
            ask.detail
        );
    }
    assert!(
        ask.body.contains("不许悄悄按无围栏执行"),
        "正文要说清为什么问：{}",
        ask.body
    );
}

/// **构不出可用选项**（除"放弃"外没有一条真能执行的）时**不发起裁决**：
/// 返回 `None`——调用方据此停掉会话 + 落一条警告（契约禁止置灰）。
#[test]
fn no_real_option_means_no_question() {
    let fact = blocked(FencePart::Cwd, "C:/gone/module");
    assert!(
        fence_ask(&fact, "a", false).is_none(),
        "无围栏也跑不起来时连问题都给不出"
    );
}

/// 停会话那条警告与回执都读这一句：哪一环（角色名）、哪个目录、缺什么前提、怎么补。
#[test]
fn blocked_line_names_the_part_the_dir_why_and_the_fix() {
    let fact = blocked(FencePart::Module, "C:/mods/m0");
    let line = fact.line();
    assert!(line.contains("模块目录"), "{}", line);
    assert!(line.contains("C:/mods/m0"), "{}", line);
    assert!(line.contains("错误码 5"), "{}", line);
    assert!(
        line.contains(FencePart::Module.fix()),
        "要给出怎么补：{}",
        line
    );
    // 没有具体目录的一环（容器身份 / 台账）如实说没有，不编一个路径出来。
    let sidless = blocked(FencePart::ContainerIdentity, "");
    assert!(sidless.line().contains("容器身份"), "{}", sidless.line());
}

/// 台账清单面在**没有台账**时也如实给一句话（列清单恒退出 0 的判据：判定归调用方）。
#[test]
fn ledger_listing_says_there_is_no_ledger_instead_of_pretending() {
    let root = crate::tests::scratch("fence-ledger-empty");
    let home = root.join(".home");
    let view = crate::kernel::detail::confine::ledger(&home);
    assert!(view.entries.is_empty(), "没写过权限项就没有条目");
    assert!(
        !view.note.is_empty(),
        "没有台账时也要如实说一句，不能给一份空清单让人以为一切正常"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// 其它平台不留权限项：按条处置如实拒绝（不是"存在但空转"）。
#[cfg(not(windows))]
#[test]
fn per_item_disposal_is_honestly_unavailable_off_windows() {
    use crate::kernel::detail::confine;
    let root = crate::tests::scratch("fence-ledger-offplatform");
    let home = root.join(".home");
    for done in [
        confine::restore_one(&home, &root),
        confine::revoke_grant(&home, "S-1-15-2-1", &root),
        confine::remove_profile_one(&home, "Solomni.Agent.Probe"),
    ] {
        let err = done.expect_err("本平台没有台账，按条处置要如实拒绝");
        assert!(err.contains("本平台"), "{}", err);
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// 授权结论的分流：**必要**落点进 `blocked`（第一次为准——那就是用户要补的那一环），
/// 可选落点与后续必要落点进 `notes`（只记事实）。
#[test]
fn prep_routes_necessary_failures_to_blocked_and_optional_ones_to_notes() {
    let mut prep = FencePrep::default();
    assert!(prep.ok(), "初始 = 围栏成立");
    prep.fail(
        FencePart::AuthorizedRead,
        PathBuf::from("C:/read-only"),
        "用户授权的只读根授不上".to_string(),
    );
    assert!(prep.ok(), "可选落点授不上不牵动整次执行");
    assert_eq!(prep.notes.len(), 1);
    prep.fail(
        FencePart::Module,
        PathBuf::from("C:/mods/m0"),
        "写 DACL 失败（错误码 5）".to_string(),
    );
    assert!(!prep.ok());
    assert_eq!(
        prep.blocked.as_ref().map(|b| b.part),
        Some(FencePart::Module),
        "第一个必要落点就是用户要补的那一环"
    );
    // 再来一个必要落点也不顶掉第一个：用户要补的是**最先**授不上的那一环。
    prep.fail(
        FencePart::Interpreter,
        PathBuf::from("C:/nvm4w/nodejs"),
        "写 DACL 失败（错误码 5）".to_string(),
    );
    assert_eq!(
        prep.blocked.as_ref().map(|b| b.part),
        Some(FencePart::Module)
    );
    assert_eq!(prep.notes.len(), 2, "后续失败如实记着，不漏");
}
