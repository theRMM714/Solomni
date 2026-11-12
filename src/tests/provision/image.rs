//! 目的：guest 镜像构造、文件系统权限位还原与工具链安装的用例。
//! 管：detail 的纯函数与落盘行为（解包、选包、替换、权限位、rootfs 流水线）。
//! 不管：清单与编排（见 `service.rs`）、端口实现（见 `ports.rs`）。
//! 联动：被测实现是 `capabilities::provision::detail::{guest_image,install,fetch}`。

use super::service::run;
use crate::capabilities::provision::detail::fetch::{
    choose_fastest, download_with_resume, mirror_of, part_path, part_source_marker,
};
use crate::capabilities::provision::detail::guest_image::*;
use crate::capabilities::provision::detail::ledger::record_installed;
use crate::capabilities::provision::domain::*;
use crate::capabilities::provision::service::*;
use std::collections::BTreeMap;
use std::fs;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
/// 目的：缓存文件名取 URL 末段，去掉查询串。
#[test]
fn asset_name_takes_last_path_segment() {
    assert_eq!(asset_name("https://x/a/b.tar.zst?q=1"), "b.tar.zst");
    assert_eq!(asset_name("https://x/y.zip#frag"), "y.zip");
}

/// 目的：打出来的 guest 磁盘镜像必须是 4 KiB 块——位置参数按块大小解释，不给 -b 时 mke2fs 会
///   按小体积选 1 KiB 块的 small 布局，镜像缩成约四分之一而装不下量出来的内容（Linux 真机上炸过）。
/// 约束：只在 Linux、且 unshare -r 与 mke2fs（e2fsprogs）可用时真跑；否则如实跳过（不假装验过）。
#[test]
fn packed_guest_image_uses_4k_blocks() {
    if !cfg!(target_os = "linux") {
        eprintln!("[探针] 打包镜像的块大小只在 Linux 上成立：本平台跳过");
        return;
    }
    use std::io::Read;
    use std::process::Stdio;
    let tools = std::process::Command::new("unshare")
        .args(["-r", "mke2fs", "-V"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !tools {
        eprintln!(
            "[探针] 没有可用的 unshare -r / mke2fs（要 e2fsprogs 与非特权用户命名空间）：跳过"
        );
        return;
    }
    let root = crate::tests::scratch("provision-pack-4k");
    let base = root.join("base");
    let cache = root.join("cache");
    fs::create_dir_all(&base).expect("建 base");
    fs::create_dir_all(&cache).expect("建 cache");
    fs::write(base.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
    fs::write(base.join("hello.txt"), b"hello").expect("写文件");
    pack_guest_base(&base, &cache, &mut |_| {}).expect("打包要成功");
    // ext4 超级块从镜像 1024 偏移开始：magic 在 +0x38，log_block_size 在 +0x18；块大小 = 1024 << log。
    let mut head = [0u8; 2048];
    fs::File::open(base.join("rootfs.ext4"))
        .expect("开镜像")
        .read_exact(&mut head)
        .expect("读镜像头");
    let sb = &head[1024..2048];
    let magic = u16::from_le_bytes([sb[0x38], sb[0x39]]);
    let log_block = u32::from_le_bytes([sb[0x18], sb[0x19], sb[0x1a], sb[0x1b]]);
    assert_eq!(magic, 0xEF53, "得是 ext4 超级块");
    assert_eq!(
        1024u64 << log_block,
        4096,
        "块大小必须是 4 KiB（1 KiB 的真机上装不下）"
    );
    let _ = fs::remove_dir_all(&root);
}

/// 目的：Linux 上真跑 ext4 工具链的前置（unshare -r + mke2fs + debugfs）；缺一个就不是"验过"。
#[cfg(target_os = "linux")]
fn guest_image_tools() -> bool {
    use std::process::Stdio;
    std::process::Command::new("unshare")
        .args(["-r", "mke2fs", "-V"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
        && std::process::Command::new("debugfs")
            .arg("-V")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
}

/// 目的：造一个带指定 mode 的普通文件（宿主属主；镜像里的属主由 mke2fs 的假 root 决定）。
#[cfg(target_os = "linux")]
fn write_mode(path: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    fs::write(path, b"#!/bin/sh\n").expect("写文件");
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("设权限");
}

/// 目的：读宿主目录树上某个条目的权限位（只比低 12 位；解包后这里应当**丢了**特权位）。
#[cfg(target_os = "linux")]
fn mode_of(path: &Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    fs::metadata(path).expect("读属性").permissions().mode() & 0o7777
}

/// 目的：rdump / mke2fs 都会丢**特权位与属主、属组**：按源镜像回填之后，sudo 的 setuid、
///   ssh-agent 的 setgid + 组、/tmp 的粘滞位、/var/mail 的 setgid + 组、/etc/shadow 的 shadow 组
///   都要和源镜像一致。
/// 约束：只在本机有 unshare -r / mke2fs / debugfs 时真跑；否则如实跳过（不假装验过）。
#[cfg(target_os = "linux")]
#[test]
fn guest_rootfs_restores_privilege_bits_and_ownership() {
    use std::os::unix::fs::PermissionsExt;
    if !guest_image_tools() {
        eprintln!(
            "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
        );
        return;
    }
    let root = crate::tests::scratch("provision-guest-attrs");
    let src = root.join("src");
    let tree = root.join("tree");
    let cache = root.join("cache");
    fs::create_dir_all(&cache).expect("建缓存目录");
    for d in [
        src.join("usr").join("bin"),
        src.join("etc"),
        src.join("tmp"),
        src.join("var").join("mail"),
    ] {
        fs::create_dir_all(&d).expect("建源目录");
    }
    write_mode(&src.join("usr").join("bin").join("sudo"), 0o4755);
    write_mode(&src.join("usr").join("bin").join("ssh-agent"), 0o2755);
    write_mode(&src.join("etc").join("shadow"), 0o640);
    fs::set_permissions(src.join("tmp"), fs::Permissions::from_mode(0o1777)).expect("设 /tmp");
    fs::set_permissions(
        src.join("var").join("mail"),
        fs::Permissions::from_mode(0o2775),
    )
    .expect("设 /var/mail");
    // 源镜像：mke2fs 的假 root 把属主写成 0；组用 sif 写成 cloud 镜像里的事实（宿主上 chown 不了）。
    let source = root.join("source.ext4");
    let st = std::process::Command::new("unshare")
        .args(["-r", "mke2fs", "-q", "-b", "4096", "-t", "ext4", "-F", "-d"])
        .arg(&src)
        .arg(&source)
        .arg("4096")
        .status()
        .expect("跑 mke2fs");
    assert!(st.success(), "造源镜像");
    let mut cmds = String::new();
    for (path, gid) in [
        ("/usr/bin/ssh-agent", 103u32),
        ("/etc/shadow", 42),
        ("/var/mail", 8),
    ] {
        cmds.push_str(&format!(
            "sif {} gid {}\n",
            debugfs_quote(path).expect("路径可写进命令表"),
            gid
        ));
    }
    run_debugfs(&source, &cmds, true).expect("给源镜像写属组");
    // 解包 → 清单 → 打包 → 回填。
    fs::create_dir_all(&tree).expect("建解包目录");
    extract_guest_rootfs(&source, &tree).expect("解包");
    assert_eq!(
        mode_of(&tree.join("usr").join("bin").join("sudo")),
        0o755,
        "rdump 本来就会丢 setuid——这正是回填要补的"
    );
    let attrs = guest_attrs_to_restore(&source, &tree).expect("读源属性");
    for want in [
        "/usr/bin/sudo",
        "/usr/bin/ssh-agent",
        "/etc/shadow",
        "/tmp",
        "/var/mail",
    ] {
        assert!(
            attrs.iter().any(|a| a.path == want),
            "清单里要有 {}：{:?}",
            want,
            attrs.iter().map(|a| a.path.clone()).collect::<Vec<_>>()
        );
    }
    fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
    pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
    let image = tree.join("rootfs.ext4");
    restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
    let parents: Vec<String> = ["/", "/usr/bin", "/etc", "/var"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    let got = image_attrs(&image, &parents).expect("读最终镜像");
    for want in [
        "/usr/bin/sudo",
        "/usr/bin/ssh-agent",
        "/etc/shadow",
        "/tmp",
        "/var/mail",
    ] {
        let a = attrs.iter().find(|a| a.path == want).expect("清单条目");
        let g = got
            .iter()
            .find(|g| g.path == want)
            .unwrap_or_else(|| panic!("最终镜像里没有 {}", want));
        assert_eq!(
            (g.mode, g.uid, g.gid),
            (a.mode, a.uid, a.gid),
            "{} 要回填成源镜像的属性",
            want
        );
    }
    let _ = fs::remove_dir_all(&root);
}

/// 目的：真机验收（opt-in）：拿项目内缓存的 Debian rootfs 分区跑同一条构造链，最终镜像里
///   **源镜像所有特权位 / 非 root 属主的条目**都与源一致（不碰 `.vm/guest/debian`，只写 target/test-scratch）。
/// 约束：只在 SOLOMNI_VM_LIVE=1、本机有工具链、且 `.vm/cache/rootfs.ext4` 在场时真跑；否则 [探针] 如实跳过。
#[cfg(target_os = "linux")]
#[test]
fn guest_rootfs_pipeline_keeps_source_attrs_on_the_real_image() {
    if !guest_image_tools() {
        eprintln!(
            "[探针] 没有可用的 unshare -r / mke2fs / debugfs（要 e2fsprogs 与非特权用户命名空间）：跳过"
        );
        return;
    }
    if std::env::var("SOLOMNI_VM_LIVE").as_deref() != Ok("1") {
        eprintln!(
            "[探针] 未开启虚拟根真机验收：设 SOLOMNI_VM_LIVE=1 后重跑（用 .vm/cache/rootfs.ext4）"
        );
        return;
    }
    let source = crate::tests::root_dir()
        .join(".vm")
        .join("cache")
        .join("rootfs.ext4");
    if !source.is_file() {
        eprintln!(
            "[探针] 本机没有源 rootfs 分区（先 solomni --provision --ensure=vm.guest-rootfs.linux --yes）：跳过"
        );
        return;
    }
    let root = crate::tests::scratch("provision-guest-attrs-live");
    let tree = root.join("tree");
    let cache = root.join("cache");
    fs::create_dir_all(&tree).expect("建解包目录");
    fs::create_dir_all(&cache).expect("建缓存目录");
    extract_guest_rootfs(&source, &tree).expect("解包真源镜像");
    let attrs = guest_attrs_to_restore(&source, &tree).expect("读真源镜像属性");
    let sudo = attrs
        .iter()
        .find(|a| a.path == "/usr/bin/sudo")
        .expect("真源镜像里要有 setuid 的 sudo");
    assert!(
        sudo.mode & 0o4000 != 0,
        "sudo 的 setuid 要读出来：{:o}",
        sudo.mode
    );
    assert!(
        attrs.iter().any(|a| a.gid != 0),
        "清单里要包含非 0 属组（例如 shadow）"
    );
    fs::write(tree.join("guest.yaml"), "id: t\nos: linux\narch: amd64\n").expect("写画像");
    pack_guest_base(&tree, &cache, &mut |_| {}).expect("打包");
    let image = tree.join("rootfs.ext4");
    restore_guest_image_attrs(&image, &attrs).expect("回填并核对");
    // 抽查 sudo 的最终 mode（回填已经在内部核对过全部条目）。
    let got = image_attrs(&image, &["/usr/bin".to_string()]).expect("读最终镜像");
    let sudo = got
        .iter()
        .find(|g| g.path == "/usr/bin/sudo")
        .expect("最终镜像里要有 sudo");
    assert_eq!(sudo.mode & 0o7777, 0o4755, "最终镜像里的 sudo 要带 setuid");
    let _ = fs::remove_dir_all(&root);
}

/// 目的：kind: toolchain 走同一条补齐内核——从缓存解归档到安装目录，装完把工具登记进 active.json
///   （启动期 entry::toolchains 只读这一份）；全程不碰网络。
#[test]
fn toolchain_install_extracts_and_registers_the_tool() {
    let root = crate::tests::scratch("provision-toolchain-install");
    let target = host_target();
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&cache).expect("建缓存");
    let install = format!(".tools/toolchains/{}/python/9.9.9", target);
    let tar_path = cache.join("py.tar");
    {
        let f = fs::File::create(&tar_path).expect("建 tar");
        let mut b = tar::Builder::new(f);
        let mut h = tar::Header::new_gnu();
        h.set_size(2);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, "bin/python3", &b"py"[..])
            .expect("写文件");
        b.finish().expect("收尾");
    }
    let dep = Dep {
        id: "toolchain.python.test".to_string(),
        os: "any".to_string(),
        arch: "any".to_string(),
        install: install.clone(),
        probe: format!("{}/bin", install),
        source: "release:py.tar".to_string(),
        kind: "toolchain".to_string(),
        guest: None,
        toolchain: Some(ToolchainSpec {
            tool: "python".to_string(),
            version: "9.9.9".to_string(),
            target: Some(target.clone()),
            bins: vec!["bin".to_string()],
            wrappers: false,
        }),
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    };
    let mut links = BTreeMap::new();
    links.insert(
        "release.base".to_string(),
        "https://example.invalid".to_string(),
    );
    let mut lines: Vec<String> = Vec::new();
    run(
        &root,
        &[dep],
        &links,
        &[],
        &AtomicBool::new(false),
        &mut |l| lines.push(l.to_string()),
        None,
    )
    .expect("从缓存装工具链");
    assert!(under(&root, &install).join("bin").is_dir(), "{:?}", lines);
    let active = root.join(".tools").join("toolchains").join("active.json");
    let raw = fs::read_to_string(&active).expect("要写 active.json");
    let doc: serde_json::Value = serde_json::from_str(&raw).expect("active.json 是 JSON");
    assert_eq!(doc["target"], serde_json::json!(target));
    assert_eq!(doc["profile"], serde_json::json!(["python"]));
    assert_eq!(doc["tools"][0]["id"], serde_json::json!("python"));
    assert_eq!(
        doc["tools"][0]["bin"][0],
        serde_json::json!(format!("{}/bin", install))
    );
    let _ = fs::remove_dir_all(&root);
}

/// 目的：Zig 包解出来只有 zig 本体；装完补 cc / gcc / g++ / clang 包装（相对路径，产品根搬移后仍可用）。
#[test]
fn zig_toolchain_gets_compiler_wrappers() {
    let root = crate::tests::scratch("provision-toolchain-zig");
    let target = host_target();
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&cache).expect("建缓存");
    let install = format!(".tools/toolchains/{}/cc/9.9.9", target);
    let tar_path = cache.join("zig.tar");
    {
        let f = fs::File::create(&tar_path).expect("建 tar");
        let mut b = tar::Builder::new(f);
        let mut h = tar::Header::new_gnu();
        h.set_size(3);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, "zig-demo/zig", &b"zig"[..])
            .expect("写 zig");
        b.finish().expect("收尾");
    }
    let dep = Dep {
        id: "toolchain.cc.test".to_string(),
        os: "any".to_string(),
        arch: "any".to_string(),
        install: install.clone(),
        probe: format!("{}/bin", install),
        source: "release:zig.tar".to_string(),
        kind: "toolchain".to_string(),
        guest: None,
        toolchain: Some(ToolchainSpec {
            tool: "cc".to_string(),
            version: "9.9.9".to_string(),
            target: Some(target),
            bins: vec!["bin".to_string()],
            wrappers: true,
        }),
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    };
    let mut links = BTreeMap::new();
    links.insert(
        "release.base".to_string(),
        "https://example.invalid".to_string(),
    );
    run(
        &root,
        &[dep],
        &links,
        &[],
        &AtomicBool::new(false),
        &mut |_| {},
        None,
    )
    .expect("装 Zig 工具链");
    let bin = under(&root, &install).join("bin");
    let zig = fs::read_to_string(bin.join("zig")).expect("zig 包装");
    assert!(
        zig.contains("../zig-demo/zig"),
        "zig 包装要指向包内本体：{}",
        zig
    );
    assert!(
        zig.contains("ZIG_GLOBAL_CACHE_DIR") && zig.contains("cache/global"),
        "zig 包装要把缓存钉在安装目录内：{}",
        zig
    );
    let gcc = fs::read_to_string(bin.join("gcc")).expect("gcc 包装");
    assert!(gcc.contains("zig\" cc"), "gcc 包装要调 zig cc：{}", gcc);
    let gxx = fs::read_to_string(bin.join("g++")).expect("g++ 包装");
    assert!(gxx.contains("zig\" c++"), "g++ 包装要调 zig c++：{}", gxx);
    assert!(
        gcc.contains("--target=*unknown-*") && gcc.contains("--target=\\1-"),
        "gcc 包装要把 Rust 三元组的 unknown- 去掉且保留 --target= 前缀：{}",
        gcc
    );
    for name in ["cc", "gcc", "g++", "c++", "clang", "clang++"] {
        assert!(bin.join(name).is_file(), "缺包装：{}", name);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(bin.join("gcc"))
            .expect("看模式")
            .permissions()
            .mode();
        assert!(mode & 0o111 != 0, "包装要有执行位：{:o}", mode);
    }
    let _ = fs::remove_dir_all(&root);
}

/// 目的：全量补齐时已在场的基础工具链不重装（260 MB 的 MinGW 不该每次重解）；显式点名才强制重装。
#[test]
fn present_toolchain_is_skipped_on_full_ensure() {
    let root = crate::tests::scratch("provision-toolchain-skip");
    let target = host_target();
    let install = format!(".tools/toolchains/{}/python/9.9.9", target);
    fs::create_dir_all(under(&root, &install).join("bin")).expect("铺已装好的工具");
    let dep = Dep {
        id: "toolchain.python.test".to_string(),
        os: "any".to_string(),
        arch: "any".to_string(),
        install: install.clone(),
        probe: format!("{}/bin", install),
        source: "release:missing.tar".to_string(),
        kind: "toolchain".to_string(),
        guest: None,
        toolchain: Some(ToolchainSpec {
            tool: "python".to_string(),
            version: "9.9.9".to_string(),
            target: Some(target),
            bins: vec!["bin".to_string()],
            wrappers: false,
        }),
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: String::new(),
        note: String::new(),
    };
    record_installed(&root, &dep).expect("台账");
    let mut links = BTreeMap::new();
    links.insert("release.base".to_string(), "http://127.0.0.1:1".to_string());
    let mut lines: Vec<String> = Vec::new();
    run(
        &root,
        std::slice::from_ref(&dep),
        &links,
        &[],
        &AtomicBool::new(false),
        &mut |l| lines.push(l.to_string()),
        None,
    )
    .expect("在位就跳过，不碰网络");
    assert!(
        lines.iter().any(|l| l.contains("已就位（跳过）")),
        "{:?}",
        lines
    );
    // 显式点名 = 你要的就是它：缓存里没有、源不可达时如实失败，不假装成功。
    let err = run(
        &root,
        &[dep],
        &links,
        &["toolchain.python.test".to_string()],
        &AtomicBool::new(false),
        &mut |_| {},
        None,
    )
    .expect_err("点名就必须真装");
    assert!(err.contains("下载失败"), "{}", err);
    let _ = fs::remove_dir_all(&root);
}

/// 目的：缓存与 pin 不符时换一份重下，而不是把旧缓存当证据；换不到就如实失败、安装目录一个字节都不写。
#[test]
fn cache_pin_mismatch_is_replaced_not_trusted() {
    let root = crate::tests::scratch("provision-toolchain-pin");
    let target = host_target();
    let cache = root.join(".vm").join("cache");
    fs::create_dir_all(&cache).expect("建缓存");
    fs::write(cache.join("py.tar"), b"abc").expect("写缓存");
    let install = format!(".tools/toolchains/{}/python/9.9.9", target);
    let dep = Dep {
        id: "toolchain.python.test".to_string(),
        os: "any".to_string(),
        arch: "any".to_string(),
        install: install.clone(),
        probe: format!("{}/bin", install),
        source: "release:py.tar".to_string(),
        kind: "toolchain".to_string(),
        guest: None,
        toolchain: Some(ToolchainSpec {
            tool: "python".to_string(),
            version: "9.9.9".to_string(),
            target: Some(target),
            bins: vec!["bin".to_string()],
            wrappers: false,
        }),
        family: None,
        strip: 0,
        size_mb: 0,
        sha256: "0".repeat(64),
        note: String::new(),
    };
    let mut links = BTreeMap::new();
    links.insert(
        "release.base".to_string(),
        "https://example.invalid".to_string(),
    );
    let err = run(
        &root,
        &[dep],
        &links,
        &[],
        &AtomicBool::new(false),
        &mut |_| {},
        None,
    )
    .expect_err("pin 不符不能装");
    assert!(err.contains("下载失败"), "换不到源要如实失败：{}", err);
    assert!(!under(&root, &install).exists(), "装不上不许落盘");
    assert!(
        !cache.join("py.tar").is_file(),
        "不符的旧缓存要被换掉，不能当证据留着"
    );
    let _ = fs::remove_dir_all(&root);
}

/// 目的：镜像只是候选之一（上游在前），GitHub 资产才加；选源排除不可达的候选，全都不可达时说清状态码。
#[test]
fn mirror_and_source_choice() {
    assert_eq!(
        mirror_of("https://github.com/a/b/c.zip", "https://gh.example/"),
        Some("https://gh.example/github.com/a/b/c.zip".to_string())
    );
    assert_eq!(
        mirror_of("https://example.test/a.zip", "https://gh.example"),
        None
    );
    assert_eq!(mirror_of("https://github.com/a/b", ""), None);
    let sample = |url: &str, code: u16, kbps: f64| (url.to_string(), code, kbps);
    let picked = choose_fastest(&[
        sample("https://a", 200, 10.0),
        sample("https://b", 200, 900.0),
        sample("https://c", 404, 0.0),
    ])
    .expect("要选出最快的可达源");
    assert_eq!(picked, "https://b");
    let err = choose_fastest(&[sample("https://a", 0, 0.0)]).unwrap_err();
    assert!(err.contains("没有可用的下载源"), "{}", err);
}

/// 目的：断点续传——分片与来源标记一致时带 Range 续上；来源不明就从头下（不把两份字节接起来）。
#[test]
fn resume_uses_range_only_for_a_known_source() {
    use std::sync::Arc;
    use std::time::Duration;
    let body: &'static [u8] = b"0123456789";
    let server = Arc::new(tiny_http::Server::http("127.0.0.1:0").expect("起本地源"));
    let addr = server.server_addr().to_ip().expect("TCP 地址");
    let url = format!("http://127.0.0.1:{}/a.bin", addr.port());
    let stop = Arc::new(AtomicBool::new(false));
    let seen = Arc::new(Mutex::new(Vec::<Option<String>>::new()));
    let (srv, st, sn) = (Arc::clone(&server), Arc::clone(&stop), Arc::clone(&seen));
    let handle = std::thread::spawn(move || {
        while !st.load(Ordering::Relaxed) {
            let Ok(Some(req)) = srv.recv_timeout(Duration::from_millis(50)) else {
                continue;
            };
            let range = req
                .headers()
                .iter()
                .find(|h| h.field.equiv("Range"))
                .map(|h| h.value.as_str().to_string());
            sn.lock().expect("锁").push(range.clone());
            let resp = match range {
                Some(r) => {
                    let start: usize = r
                        .trim_start_matches("bytes=")
                        .trim_end_matches('-')
                        .parse()
                        .unwrap_or(0);
                    tiny_http::Response::from_data(body[start..].to_vec()).with_status_code(206)
                }
                None => tiny_http::Response::from_data(body.to_vec()),
            };
            let _ = req.respond(resp);
        }
    });
    let root = crate::tests::scratch("provision-resume");
    let dest = root.join("a.bin");
    let part = part_path(&dest);
    // 分片 + 来源标记一致：续传（Range: bytes=<断点>-）。
    fs::write(&part, &body[..4]).expect("写分片");
    fs::write(part_source_marker(&part), &url).expect("写来源标记");
    let agent = crate::kernel::detail::outbound::agent(5, 30);
    download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("续传");
    assert_eq!(
        fs::read(&dest).expect("读下载结果"),
        body,
        "续传后内容要完整"
    );
    assert!(!part.exists(), "成功后分片要改名掉");
    // 来历不明的分片：从头下（不发 Range）。
    fs::write(&part, b"XYZ").expect("写来历不明的分片");
    download_with_resume(&agent, &url, &dest, &mut |_| {}).expect("从头下");
    assert_eq!(fs::read(&dest).expect("读下载结果"), body);
    let ranges = seen.lock().expect("锁").clone();
    assert_eq!(ranges.len(), 2, "{:?}", ranges);
    assert_eq!(ranges[0].as_deref(), Some("bytes=4-"), "首个请求要带断点");
    assert_eq!(ranges[1], None, "来源不明必须从头下");
    stop.store(true, Ordering::Relaxed);
    handle.join().expect("收本地源线程");
    let _ = fs::remove_dir_all(&root);
}
