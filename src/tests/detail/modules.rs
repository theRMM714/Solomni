//! 目的：模块与包源码扫描的接受/拒绝契约。
//! 管：非法目录形态逐条给原因、服务与密钥声明、userdata 事实、依赖目录扫描。
//! 不管：模块运行、构建与安装。
//! 联动：隔离根 `tests::scratch`；清单格式见 docs/。

use super::*;

#[test]
fn fs_modules_accepts_valid_folders_and_rejects_each_illegal_form_with_a_reason() {
    let root = scratch("fs-modules");
    let dir = root.join("modules");
    let put = |folder: &str, yaml: &str| {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).expect("建模块目录");
        std::fs::write(d.join("module.yaml"), yaml).expect("写清单");
    };
    put("good", "id: good\nbrief: 简介\nsystem: 用法：good\n");
    std::fs::create_dir_all(dir.join("nomanual")).expect("目录");
    std::fs::write(
        dir.join("nomanual").join("module.yaml"),
        "id: 别的名字\nbrief: b\nsystem: s\n",
    )
    .expect("写");
    std::fs::create_dir_all(dir.join("missyaml")).expect("空目录");
    put(
        "reserved",
        "id: reserved\nbrief: b\nsystem: s\ntools:\n  read:\n    command: python x.py\n",
    );
    put(
        "badcap",
        "id: badcap\nbrief: b\nsystem: s\nruntimes: [Python]\n",
    );
    put("broken", "id: broken\nbrief: [不是字符串\n");

    let roster = FsModules::new(dir, crate::capabilities::tools::api::names()).scan();
    let ids: Vec<&str> = roster
        .modules
        .iter()
        .map(|m| m.manifest.id.as_str())
        .collect();
    assert_eq!(ids, vec!["good"], "只有合法模块进清单，且按 id 排序");
    assert_eq!(
        roster.rejected.len(),
        5,
        "每种非法形式都要留下原因：{:?}",
        roster.rejected
    );
    let all = roster.rejected.join("\n");
    assert!(all.contains("id '别的名字' 与文件夹名不一致"), "{}", all);
    assert!(all.contains("缺少 module.yaml"), "{}", all);
    assert!(all.contains("保留名"), "内置工具名占用要拒收：{}", all);
    assert!(all.contains("不合法"), "运行能力名非法要拒收：{}", all);
    assert!(all.contains("非法"), "坏 yaml 要拒收：{}", all);

    // 目录整个不存在 = 空清单（放入即出现，移出即消失）。
    let gone = FsModules::new(root.join("没有这个目录"), Vec::new()).scan();
    assert!(gone.modules.is_empty() && gone.rejected.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

/// `services:` / `secrets:` 的语法校验：合法进清单；缺 adapter / 缺 env 各自拒收并说明。
#[test]
fn fs_modules_validates_service_and_secret_declarations() {
    let root = scratch("fs-modules-services");
    let dir = root.join("modules");
    let put = |folder: &str, yaml: &str| {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).expect("建模块目录");
        std::fs::write(d.join("module.yaml"), yaml).expect("写清单");
    };
    put(
        "ok",
        "id: ok\nbrief: b\nsystem: s\nservices:\n  fs:\n    adapter: mcp\n    command: npx -y server\nsecrets:\n  token:\n    env: TOKEN\n",
    );
    put(
        "noadapter",
        "id: noadapter\nbrief: b\nsystem: s\nservices:\n  fs:\n    command: x\n",
    );
    put(
        "noenv",
        "id: noenv\nbrief: b\nsystem: s\nsecrets:\n  token:\n    desc: 缺 env\n",
    );

    let roster = FsModules::new(dir, crate::capabilities::tools::api::names()).scan();
    let ids: Vec<&str> = roster
        .modules
        .iter()
        .map(|m| m.manifest.id.as_str())
        .collect();
    assert_eq!(ids, vec!["ok"], "只有合法模块进清单：{:?}", roster.rejected);
    let all = roster.rejected.join("\n");
    assert!(all.contains("adapter"), "缺 adapter 要拒收：{}", all);
    assert!(all.contains("env"), "缺 env 要拒收：{}", all);
    let _ = std::fs::remove_dir_all(&root);
}

/// 载入即确保模块有 `userdata/`：没有就建出来，已存在的不动（幂等）；建不了才置 false。
#[test]
fn fs_modules_reads_the_userdata_fact() {
    let root = scratch("fs-modules-userdata");
    let dir = root.join("modules");
    let put = |folder: &str| {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).expect("建模块目录");
        std::fs::write(
            d.join("module.yaml"),
            format!("id: {}\nbrief: b\nsystem: s\n", folder),
        )
        .expect("写清单");
    };
    put("with");
    put("without");
    std::fs::create_dir_all(dir.join("with").join("userdata")).expect("建 userdata");
    std::fs::write(dir.join("with").join("userdata").join("keep.txt"), "x").expect("放已有的状态");

    let roster = FsModules::new(dir.clone(), crate::capabilities::tools::api::names()).scan();
    let fact = |id: &str| {
        roster
            .modules
            .iter()
            .find(|m| m.manifest.id == id)
            .map(|m| m.has_userdata)
            .expect("模块在清单里")
    };
    assert!(
        fact("with") && fact("without"),
        "载入后每个模块都拿到私有可写落点"
    );
    assert!(
        dir.join("without").join("userdata").is_dir(),
        "原本没有的会被建出来"
    );
    assert!(
        dir.join("with").join("userdata").join("keep.txt").is_file(),
        "已存在的不动（幂等）"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn fs_packages_scans_the_dependency_folder_and_reports_each_rejection() {
    let root = scratch("fs-packages");
    let dir = root.join("runtimes");
    let put = |folder: &str, yaml: Option<&str>| {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).expect("建包目录");
        if let Some(y) = yaml {
            std::fs::write(d.join("package.yaml"), y).expect("写清单");
        }
    };
    put(
        "py",
        Some("id: python\nversion: 3.12.4\nprefix: opt/rt/py\n"),
    );
    put(
        "py2",
        Some("id: python\nversion: 3.12.4\nprefix: opt/rt/py2\n"),
    );
    put("bad", Some("id: Python\nversion: 1\nprefix: opt/p\n"));
    put("noyaml", None);

    let src = FsPackages::new(dir);
    let lib = src.scan();
    assert_eq!(
        lib.versions_of("python").len(),
        1,
        "同 (id, version) 只收一份"
    );
    assert_eq!(lib.rejected.len(), 3, "{:?}", lib.rejected);
    let all = lib.rejected.join("\n");
    assert!(all.contains("只收先出现的那份"), "{}", all);
    assert!(
        all.contains("package.yaml 非法") || all.contains("id"),
        "{}",
        all
    );
    assert!(all.contains("缺少 package.yaml"), "{}", all);
    assert!(
        src.dir().is_absolute() && src.dir().ends_with("runtimes"),
        "dir() 如实报位置"
    );
    assert!(
        FsPackages::new(root.join("没有这个目录"))
            .scan()
            .packages
            .is_empty(),
        "目录不存在 = 空包库"
    );
    let _ = std::fs::remove_dir_all(&root);
}
