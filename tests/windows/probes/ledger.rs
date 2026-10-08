//! 目的：按条处置围栏台账的命令行入口探针。
//! 管：列清单（JSON / 可读两版）与按条处置的用法错、失败退出的如实性；只读台账与落点，不写任何 ACL。
//! 不管：真机的 ACL 写与撤（L1 的台账用例在 temp 目录里跑真 ACL）；缺口账（tests/gaps.yaml）。
//! 联动：命令行协议见 src/guard/mod.rs；台账形状与 ACE 读法见 src/capabilities/tools/detail/confine/。

use crate::probe::{bin, scratch};
use std::process::Command;

/// 跑一次产品入口，拿（退出码, stdout, stderr）——判定归这里，产品只报事实。
fn run(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(bin()).args(args).output().expect("跑产品入口");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// 手写一份台账（三条：一条根内快照、一条路径已不在的根外授权、一个不存在的 profile）：
/// 本探针验的是**清单与按条处置的命令行契约**，不在这里写 ACL。
#[test]
fn ledger_listing_and_per_item_commands_are_machine_readable_and_honest() {
    let root = scratch("ledger-cli");
    let snap = root.join("inside");
    std::fs::create_dir_all(&snap).expect("建快照落点");
    let gone = root.join("gone");
    let home = root.join(".home");
    std::fs::create_dir_all(&home).expect("建私有区");
    let esc = |p: &std::path::Path| p.to_string_lossy().replace('\\', "/");
    let ledger = format!(
        "{{\n  \"profiles\": [{{\"name\": \"Solomni.Agent.ProbeLedger\", \"at\": 11}}],\n  \"grants\": [{{\"sid\": \"S-1-15-2-1\", \"path\": \"{}\", \"rights\": 18, \"at\": 12}}],\n  \"snapshots\": [{{\"path\": \"{}\", \"at\": 13, \"bytes\": []}}]\n}}\n",
        esc(&gone),
        esc(&snap),
    );
    std::fs::write(home.join("fence-grants.json"), ledger).expect("写台账");
    let root_s = root.to_string_lossy().into_owned();
    // 台账里存的就是写进去的那个字符串（产品原样回显，不另做规范化），断言按同一份写法比。
    let snap_s = esc(&snap);
    let gone_s = esc(&gone);

    // ① 列清单默认 JSON（机器可读优先）：三条都在，路径不在了的如实带 note。
    let (code, out, err) = run(&["--root", &root_s, "--fence-ledger"]);
    assert_eq!(code, 0, "列清单恒退出 0（判定归调用方）：{}", err);
    let view: serde_json::Value = serde_json::from_str(&out).expect("清单要是 JSON");
    let entries = view["entries"].as_array().expect("entries 要是数组");
    assert_eq!(entries.len(), 3, "台账三条都要列出来：{}", out);
    assert!(
        entries.iter().any(|e| e["kind"] == "snapshot"
            && e["path"] == snap_s.as_str()
            && e["present"] == true),
        "根内快照要列出来并如实标成还在：{}",
        out
    );
    assert!(
        entries.iter().any(|e| {
            e["kind"] == "grant"
                && e["sid"] == "S-1-15-2-1"
                && e["path"] == gone_s.as_str()
                && e["present"] == false
        }),
        "根外授权要列出来并如实标成路径已不在：{}",
        out
    );
    assert!(
        entries
            .iter()
            .any(|e| e["kind"] == "profile" && e["present"] == false),
        "profile 要列出来并如实标成不在：{}",
        out
    );

    // ② 可读版：一行一条，三类都认得出。
    let (code, out, err) = run(&["--root", &root_s, "--fence-ledger", "--human"]);
    assert_eq!(code, 0, "可读版也恒退出 0：{}", err);
    for want in ["[快照]", "[授权]", "[profile]", "现在不在了"] {
        assert!(out.contains(want), "可读版要写出 {}：{}", want, out);
    }

    // ③ 用法错：缺参数的按条处置如实报用法并退出 2（与其它隐藏模式一致）。
    for flag in ["--fence-restore", "--fence-profile-rm"] {
        let (code, _, err) = run(&["--root", &root_s, flag]);
        assert_eq!(code, 2, "{} 缺参数要退出 2：{}", flag, err);
        assert!(err.contains("用法"), "{} 缺参数要说用法：{}", flag, err);
    }
    let (code, _, err) = run(&["--root", &root_s, "--fence-revoke", "S-1-15-2-1"]);
    assert_eq!(code, 2, "撤权缺路径要退出 2：{}", err);

    // ④ 失败如实报、非零退出：SID 不合法 / 台账里没有这条快照。
    let (code, _, err) = run(&["--root", &root_s, "--fence-revoke", "not-a-sid", &snap_s]);
    assert_eq!(code, 1, "SID 不合法要退出 1：{}", err);
    assert!(err.contains("SID 不合法"), "{}", err);
    let (code, _, err) = run(&["--root", &root_s, "--fence-restore", &gone_s]);
    assert_eq!(code, 1, "台账里没有这条快照要退出 1：{}", err);
    assert!(err.contains("台账里没有"), "{}", err);

    // ⑤ 本来就不在的 profile：如实说"本来就不在"，不当成失败。
    let (code, out, err) = run(&[
        "--root",
        &root_s,
        "--fence-profile-rm",
        "Solomni.Agent.NotThere",
    ]);
    assert_eq!(code, 0, "本来就不在不算失败：{} / {}", out, err);
    assert!(out.contains("本来就不在"), "{}", out);

    let _ = std::fs::remove_dir_all(&root);
}
