//! 目的：容器内真机往返——授权后容器里读得到叶子、读不到父目录其它内容、模块只读、程序能跑。
//! 管：建 AppContainer profile 与写 ACL 的用例，--fence-live 才跑。
//! 不管：ACL 写法的判据（见 `grant.rs`）、视图盘与外部工作区一次性只读（见 `workspace.rs`）。
//! 联动：夹具在 `tests.rs`；机制在 `confine/windows.rs` 与 `windows/container.rs`。

use super::*;

/// 【真机往返探针】授权（叶子读写 + 父目录只读属性）→ 容器里：在自己的边界里写得到也读得回、
/// 从父目录**按名**穿得到叶子里的文件（RIGHTS_STAT 存在的全部理由：存在性判断不会假不存在）、
/// 却读不到父目录里的其它条目——口径端到端成立。
/// 会创建 AppContainer profile（改本机状态），按测试约定只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
///
/// **这里不再用"容器内 `whoami /groups` 里找包 SID 组"判容器是否生效**：包 SID 在现代 Windows 上
/// 不在令牌的组列表里（在 `TokenAppContainerSid` 字段），容器真生效也会因此被判成"环境降级"而跳过
/// （真机令牌转储：`TokenIsAppContainer=1`、`AppContainerSid` = 该 profile 的 SID、`capabilities=0`，
/// 而同一个进程的 `whoami /groups` 里没有任何 `S-1-15-2-`）。现在改由**行为**给结论：
/// 授权落点写得进、父目录内容列不到；做不到就响亮失败（那才是口径破了，不是环境不允许）。
#[test]
fn container_roundtrip_sees_leaf_but_not_parent_content() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_roundtrip_sees_leaf_but_not_parent_content 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-roundtrip-probe-{}", std::process::id()));
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&leaf).expect("建探针目录");
    std::fs::write(leaf.join("data.txt"), "x").expect("写探针文件");
    // 父目录里另放一个条目：容器应当**读不到**它（只读属性只够按名穿过，不够读内容）。
    std::fs::write(base.join("parent-secret.txt"), "PARENT-SECRET").expect("写父目录条目");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-roundtrip".to_string(),
        lease: String::new(),
        private: PathBuf::new(),
        ro_tree: Vec::new(),
        rw: vec![leaf.clone()],
        cwd: leaf.clone(),
        ro: Vec::new(),
        net: false,
    };
    let container = container_name(&spec);
    // 建不出 profile 分两种：环境不允许（如实跳过）与我们的步骤写错（响亮失败），分开处理。
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let home = base.join("ledger");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container).expect("派生容器 SID");

    // 1) 数据边界里写得到、读得回（cwd 相对路径，与真实工具的形态一致）：写不进授权落点等于授权没生效，
    //    这一条同时也是"容器进程真拿到了 rw 写权"的正向对照。
    let code = run_in_container(
        sid,
        &spec,
        "echo data > out.txt & type out.txt > read.txt 2>&1",
    )
    .expect("容器进程应当能启动");
    let read = std::fs::read_to_string(leaf.join("read.txt")).unwrap_or_default();
    if code != 0 || !read.contains("data") {
        panic!(
            "容器里写自己的边界并读回来应当成功（exit={}，拿到 {:?}）：容器内诊断——{}",
            code,
            read.trim(),
            container_diag(sid, &spec, &leaf)
        );
    }

    // 2) 父目录能被**按名穿过**（这正是「父目录只拿只读属性」那条口径）：从父目录按名字读到叶子里的文件。
    //    不用 if exist / attrib / dir 当尺子——前两个要走父目录的列举权；dir 与 Get-ChildItem 在容器里会被拒。
    let code = run_in_container(sid, &spec, "type ..\\leaf\\data.txt > reach.txt 2>&1")
        .expect("容器进程应当能启动");
    let reach = std::fs::read_to_string(leaf.join("reach.txt")).unwrap_or_default();
    if code != 0 || !reach.contains('x') {
        panic!(
            "父目录要能按名穿过（exit={}，拿到 {:?} = 穿不过去，存在性判断会假不存在）：容器内诊断——{}",
            code,
            reach.trim(),
            container_diag(sid, &spec, &leaf)
        );
    }

    // 3) 父目录里的其它条目读不到：只读属性只够按名穿过，不等于能读父目录的内容。
    let code = run_in_container(sid, &spec, "type ..\\parent-secret.txt > psecret.txt 2>&1")
        .expect("容器进程应当能启动");
    let got = std::fs::read_to_string(leaf.join("psecret.txt")).unwrap_or_default();
    assert!(
        code != 0 && !got.contains("PARENT-SECRET"),
        "父目录里的其它条目不该读得到：{:?}（exit={}）",
        got,
        code
    );

    // 4) 列自己的产物：容器要在自己的边界里看得见自己的东西。判据用 cmd 的 `for` 枚举（FindFirstFile）——
    //    **不用 `dir`**：同一叶子上 `dir` 与 PowerShell 的 `Get-ChildItem` 会被拒，`for` 与 python 的 `os.listdir` 正常。
    let code = run_in_container(sid, &spec, "(for %f in (*) do @echo %f) > listing.txt 2>&1")
        .expect("容器进程应当能启动");
    let listing = std::fs::read_to_string(leaf.join("listing.txt")).unwrap_or_default();
    assert!(
        code == 0 && listing.contains("data.txt"),
        "容器里要能列出自己的产物（exit={}，拿到 {:?}）",
        code,
        listing
    );
    // `dir` 只记录、不断言：它是这个环境的已知异常，环境变了要能在日志里看见。
    let dir_code =
        run_in_container(sid, &spec, "dir /b > dir-listing.txt 2>&1").expect("容器进程应当能启动");
    eprintln!(
        "[诊断] 同一叶子上的 cmd dir：exit={} 输出={:?}",
        dir_code,
        std::fs::read_to_string(leaf.join("dir-listing.txt"))
            .unwrap_or_default()
            .trim()
    );

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    // 探针建的容器 profile 也要带走：测试不在本机留痕。
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 【真机往返探针·模块形态】**垂直分工的权限方位**端到端验收：`ro_tree` = 模块根（递归只读）、
/// `rw` = `<模块>/userdata`（可写）、`cwd` = 模块根，而另一席的沙箱不在任何授权里。容器里要同时成立：
/// ① 模块脚本读得到、`userdata/` 写得进；② 模块目录里写新文件被机制拒（写权只来自 rw）；
/// ③ 另一席沙箱里的明文拿不到（跨 agent 不可达）。
/// 会创建 AppContainer profile（改本机状态），只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_roundtrip_keeps_module_read_only_and_peer_unreachable() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_roundtrip_keeps_module_read_only_and_peer_unreachable 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：模块形态往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-module-probe-{}", std::process::id()));
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    let peer = base.join("session").join("peer");
    std::fs::create_dir_all(&userdata).expect("建模块 userdata");
    std::fs::create_dir_all(&peer).expect("建另一席的沙箱");
    std::fs::write(module.join("script.py"), "print(1)\n").expect("写模块脚本");
    std::fs::write(peer.join("secret.txt"), "PEER-SECRET").expect("写另一席的明文");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-module".to_string(),
        lease: String::new(),
        private: userdata.clone(),
        ro_tree: vec![module.clone()],
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        cwd: module.clone(),
        net: false,
    };
    let container = container_name(&spec);
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let home = base.join("ledger");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container).expect("派生容器 SID");

    // ① 模块目录读得到、userdata 写得进：一条命令验两件事（脚本内容经重定向落进 userdata）。
    let code = run_in_container(sid, &spec, "type script.py > userdata\\read.txt 2>&1")
        .expect("容器进程应当能启动");
    let read = std::fs::read_to_string(userdata.join("read.txt")).unwrap_or_default();
    assert_eq!(code, 0, "模块目录要读得到、userdata 要写得进：{}", read);
    assert!(read.contains("print(1)"), "模块脚本内容要读得到：{}", read);

    // ② 模块目录对 agent 只读：模块根里写不进新文件（写权只来自 rw）。
    let forbidden = module.join("should-not-exist.txt");
    let code =
        run_in_container(sid, &spec, "echo x > should-not-exist.txt").expect("容器进程应当能启动");
    assert!(
        !forbidden.exists() && code != 0,
        "模块目录对 agent 只读：模块根里不该写得进新文件（exit={}）：{:?}",
        code,
        std::fs::read_dir(&module).map(|d| d.flatten().map(|e| e.file_name()).collect::<Vec<_>>())
    );

    // ③ 另一席的沙箱不在任何授权里：明文拿不到（跨 agent 不可达）。
    let peer_file = peer.join("secret.txt");
    let code = run_in_container(
        sid,
        &spec,
        &format!("type \"{}\" > userdata\\peer.txt 2>&1", peer_file.display()),
    )
    .expect("容器进程应当能启动");
    let got = std::fs::read_to_string(userdata.join("peer.txt")).unwrap_or_default();
    assert!(
        !got.contains("PEER-SECRET") && code != 0,
        "另一席的沙箱不可达：拿到 {:?}（exit={}）",
        got,
        code
    );

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 【真机往返探针·node 形态】容器里 `node <文件>` 必须跑得起来：node 的 `fs.realpathSync` 会先 lstat 盘卷根、
/// 再逐级 lstat 祖先前缀，而这两类落点按设计都不在可达范围——没有解释器基线（`NODE_OPTIONS` 跳过 realpath），
/// 进程在脚本执行前就 EPERM 死。所以分两段：带运行期环境跑通一次完整往返（模块脚本 + require 进来的依赖 +
/// 产物落进 userdata），再用同一份环境**关掉开关**复现失败现场（根因钉死，不是"容器坏了"）。
/// 夹具的模块根带一份 package.json：主模块格式判定会逐级向上找作用域配置，容器里够不到的祖先会让它报
/// `ERR_INVALID_PACKAGE_CONFIG` 判死；模块要自带这份作用域（见 MODULE_SPEC 的「模块目录就是发现边界」）。
/// 会创建 AppContainer profile（改本机状态），只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑；本机没有 node 时如实跳过。
#[test]
fn container_runs_a_node_module_tool_with_realpath_skipped() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_runs_a_node_module_tool_with_realpath_skipped 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：node 形态往返探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    if !node_runs() {
        eprintln!("[探针] 本机没有能跑的 node：node 形态往返探针跳过（不静默当作通过）");
        return;
    }
    let base = std::env::temp_dir().join(format!("solomni-node-probe-{}", std::process::id()));
    let module = base.join("modules").join("m0");
    let userdata = module.join("userdata");
    std::fs::create_dir_all(module.join("tools")).expect("建模块 tools");
    std::fs::create_dir_all(&userdata).expect("建模块 userdata");
    // 主脚本 require 同目录的依赖：依赖那一路也要 realpath，光有主模块那个开关不够（这正是两个开关的理由）。
    // 模块根放一份 package.json：主模块格式判定会逐级向上找作用域配置，够不到的祖先会直接把它判死。
    std::fs::write(module.join("package.json"), "{ \"type\": \"commonjs\" }")
        .expect("写模块根 package.json");
    std::fs::write(
        module.join("tools").join("helper.js"),
        "module.exports = { marker: 'node-tool-ok' };",
    )
    .expect("写依赖脚本");
    std::fs::write(
        module.join("tools").join("report.js"),
        "const helper = require('./helper.js'); const fs = require('fs'); fs.writeFileSync(process.argv[2], helper.marker);",
    )
    .expect("写主脚本");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-node".to_string(),
        lease: String::new(),
        rw: vec![userdata.clone()],
        ro: Vec::new(),
        ro_tree: vec![module.clone()],
        private: userdata.clone(),
        cwd: module.clone(),
        net: false,
    };
    let container = container_name(&spec);
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let home = base.join("ledger");
    let command = "node tools/report.js userdata/report.txt";
    let prep = prepare_fence(&spec, command, &home);
    if let Some(blocked) = &prep.blocked {
        // 解释器目录（PATH 里那个 node 的安装处）授不上权限时，容器里读不到解释器，这条链路本机做不了：
        // 环境结论，如实跳过。授权代码真坏了会被同族的往返探针响亮抓住（它们用的是系统里的 cmd）。
        eprintln!(
            "[探针] 本机给授权落点写不了权限（{}）：node 形态往返探针跳过（不静默当作通过）",
            blocked.line()
        );
        if let Err(e) = release_fence(&spec, &home) {
            eprintln!("[诊断] 撤权未完成（{}）：要收尾请跑 --fence-clean", e);
        }
        clean(&home).ok();
        delete_profile(&container);
        discard(&base);
        return;
    }
    let sid = container_sid(&container).expect("派生容器 SID");

    // ① 运行期白名单这一层就要带上两个开关：机制不在这儿就位，容器里再补已经晚了。
    let env = crate::kernel::detail::confine::fence_env(&spec, command);
    let opts = env
        .iter()
        .find(|(k, _)| k == "NODE_OPTIONS")
        .map(|(_, v)| v.to_string_lossy().to_string())
        .unwrap_or_default();
    assert!(
        opts.contains("--preserve-symlinks") && opts.contains("--preserve-symlinks-main"),
        "运行期环境要带两个开关（缺哪个都会在另一半上照样 realpath）：{:?}",
        opts
    );

    // ② 容器里跑得通：产物落在 userdata（rw 里），内容来自 require 进来的依赖。
    let code = run_in_container(sid, &spec, command).expect("容器进程应当能启动");
    let got = std::fs::read_to_string(userdata.join("report.txt")).unwrap_or_default();
    assert!(
        code == 0 && got.contains("node-tool-ok"),
        "容器里 node 工具应当跑通（exit={}，产物 {:?}）——容器里 stdout/stderr 已透传到本进程输出",
        code,
        got.trim()
    );

    // ③ 对照：命令行把两个开关关掉（命令行优先于 NODE_OPTIONS）→ 复现"脚本执行前就倒"的失败现场。
    let off = "node --no-preserve-symlinks --no-preserve-symlinks-main tools/report.js userdata/off.txt 2> userdata/off-err.txt";
    let off_code = run_in_container(sid, &spec, off).expect("容器进程应当能启动");
    let off_err = std::fs::read_to_string(userdata.join("off-err.txt")).unwrap_or_default();
    if off_code == 0 {
        eprintln!(
            "[诊断] 关掉开关也跑通了（本机这条路径不再需要它）：{:?}",
            off_err.trim()
        );
    } else {
        assert!(
            off_err.contains("lstat"),
            "关掉开关的失败现场应当是 realpath 的 lstat 落点（exit={}，原话 {:?}）",
            off_code,
            off_err.trim()
        );
    }

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 【真机】容器里**绝对路径**的程序也要能跑：执行前把程序名改写成相对 cwd 的路径。
///   现象：同一文件用 `D:\…\x.exe` 报 Access denied，用 `..\x.exe` 正常（相对路径走 cwd 的目录句柄，
///   不遍历卷根；非系统盘的卷根 DACL 要管理员才写得动）。容器执行因此在 `run_in_container` 里做这次改写。
/// 会创建 AppContainer profile 与写目录 ACL，只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_runs_a_program_given_by_absolute_path() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_runs_a_program_given_by_absolute_path 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：绝对路径执行探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let base = repo.join("target").join("probe-exec");
    let c_leaf = base.join("c-leaf");
    let d_leaf = base.join("d-leaf");
    std::fs::create_dir_all(&c_leaf).expect("建 cwd 夹具");
    std::fs::create_dir_all(&d_leaf).expect("建相邻夹具");
    let cmd_exe = PathBuf::from(std::env::var("SystemRoot").unwrap_or_default())
        .join("System32")
        .join("cmd.exe");
    std::fs::copy(&cmd_exe, c_leaf.join("cmd2.exe")).expect("拷到 cwd");
    std::fs::copy(&cmd_exe, d_leaf.join("cmd2.exe")).expect("拷到相邻目录");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-exec".to_string(),
        lease: String::new(),
        rw: vec![c_leaf.clone(), d_leaf.clone()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: c_leaf.clone(),
        cwd: c_leaf.clone(),
        net: false,
    };
    let container = container_name(&spec);
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let home = base.join("ledger");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let sid = container_sid(&container).expect("派生 SID");
    // 相对名（基线对照）与绝对路径（相邻目录，必须被改写成相对路径才跑得起来）。
    let rel = run_in_container(sid, &spec, "cmd2.exe /c echo REL > rel.txt").expect("起容器");
    let abs = run_in_container(
        sid,
        &spec,
        &format!(
            "\"{}\" /c echo ABS > abs.txt",
            d_leaf.join("cmd2.exe").display()
        ),
    )
    .expect("起容器");
    assert!(
        rel == 0 && c_leaf.join("rel.txt").is_file(),
        "相对名要能跑（exit={}）",
        rel
    );
    assert!(
        abs == 0 && c_leaf.join("abs.txt").is_file(),
        "绝对路径要能跑（执行前应改写成相对 cwd 的路径；exit={}）",
        abs
    );
    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 目的：容器临时目录按 AppContainer 的 `GetTempPath2` 布局给——`<私有沙箱>/Packages/<容器名>/AC/Temp`。
/// 约束：私有沙箱为空时退回工作目录；两者都空就没有落点（返回 `None`，不猜）。
#[test]
fn container_temp_dir_follows_gettemp2_layout() {
    let _fence = crate::tests::fence_acl_guard();
    let mut spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-temp".to_string(),
        lease: String::new(),
        rw: Vec::new(),
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: PathBuf::from("sandbox"),
        cwd: PathBuf::from("module"),
        net: false,
    };
    let dir = container_temp_dir(&spec).expect("私有沙箱在场");
    assert_eq!(
        dir,
        PathBuf::from("sandbox")
            .join("Packages")
            .join(container_name(&spec))
            .join("AC")
            .join("Temp")
    );
    spec.private = PathBuf::new();
    assert!(
        container_temp_dir(&spec)
            .expect("退回工作目录")
            .starts_with("module"),
        "私有沙箱为空时要退回工作目录"
    );
    spec.cwd = PathBuf::new();
    assert!(container_temp_dir(&spec).is_none(), "没有落点就不猜");
}

/// 【真机】容器里的系统临时目录由 `prepare_fence` 建好且可写：进程能在里面落临时文件。
///   不建它的现场：程序写 `%TEMP%` 失败（编译器会报 `Cannot create temporary file in …\AC\Temp\` 并 abort）。
/// 会创建 AppContainer profile 与写目录 ACL，只在 --fence-live（SOLOMNI_FENCE_LIVE=1）下跑。
#[test]
fn container_has_a_writable_temp_dir() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：container_has_a_writable_temp_dir 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：容器临时目录探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    // 夹具放在仓库卷上：跨卷时程序名无法相对化，而这条用例验的是容器能写自己的临时目录。
    let base = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join(format!("temp-probe-{}", std::process::id()));
    let sandbox = base.join("sandbox");
    let leaf = base.join("leaf");
    std::fs::create_dir_all(&sandbox).expect("建私有沙箱");
    std::fs::create_dir_all(&leaf).expect("建工作目录");
    let spec = FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-temp".to_string(),
        lease: String::new(),
        rw: vec![sandbox.clone(), leaf.clone()],
        ro: Vec::new(),
        ro_tree: Vec::new(),
        private: sandbox.clone(),
        cwd: leaf.clone(),
        net: false,
    };
    let container = container_name(&spec);
    if let Err(e) = ensure_profile(&container) {
        if e.contains(PROFILE_ENV_BLOCKED_MARK) {
            eprintln!(
                "[探针] 本环境不允许建 AppContainer profile（环境结论，如实跳过）：{}",
                e
            );
            discard(&base);
            return;
        }
        panic!("建容器 profile：{}", e);
    }
    let home = base.join("ledger");
    expect_granted(&prepare_fence(&spec, "cmd", &home), "授权应当成功");
    let temp = container_temp_dir(&spec).expect("私有沙箱在场");
    assert!(
        temp.is_dir(),
        "prepare 要先建好容器临时目录：{}",
        temp.display()
    );
    let sid = container_sid(&container).expect("派生容器 SID");

    // ① 容器里的 TEMP 就是它（GetTempPath2 的落点），且可写（授权靠 rw 的递归 ACE，不单列一条）。
    let probe = temp.join("probe.txt");
    let seen = leaf.join("temp-path.txt");
    let code = run_in_container(
        sid,
        &spec,
        &format!(
            "echo %TEMP%> \"{}\" & echo ok > \"{}\"",
            seen.display(),
            probe.display()
        ),
    )
    .expect("容器进程应当能启动");
    assert!(
        code == 0 && probe.is_file(),
        "容器要能写临时目录（exit={}，路径 {}）",
        code,
        probe.display()
    );
    let got = std::fs::read_to_string(&seen).unwrap_or_default();
    assert_eq!(
        got.trim().to_lowercase(),
        temp.display().to_string().to_lowercase(),
        "容器里的 TEMP 应当就是 prepare 建的落点"
    );

    free_sid(sid);
    release_fence(&spec, &home).expect("撤权应当成功");
    clean(&home).expect("台账回收应当成功");
    assert!(delete_profile(&container), "探针的容器 profile 应当删得掉");
    discard(&base);
}

/// 【真机并发】同一 agent 的两个不同 scope 调用**重叠**时：各自独立容器身份，窄调用读不到宽调用临时加的根；
/// 释放一次不影响另一次；收尾不留孤儿 ACE / profile / 临时目录。
/// 约束：只在 --fence-live（SOLOMNI_FENCE_LIVE=1）且本机能做 ACL 往返时跑；会创建 AppContainer profile，
///   收尾按台账撤权、删 profile 与临时目录。
#[test]
fn concurrent_calls_of_one_agent_are_isolated_and_released_independently() {
    let _fence = crate::tests::fence_acl_guard();
    if std::env::var("SOLOMNI_FENCE_LIVE")
        .map(|v| v != "1")
        .unwrap_or(true)
    {
        eprintln!(
            "[探针] 未开启真机围栏测试：concurrent_calls_of_one_agent_are_isolated_and_released_independently 会创建 AppContainer profile（改本机状态），已跳过；要真跑加 --fence-live"
        );
        return;
    }
    if !capability().fs {
        eprintln!(
            "[探针] 本机不允许改目录 ACL（{}）：并发隔离探针跳过（不静默当作通过）",
            capability().note
        );
        return;
    }
    use crate::kernel::ports::FenceHost;
    let base = std::env::temp_dir().join(format!("solomni-overlap-{}", std::process::id()));
    let wide = base.join("wide");
    let narrow = base.join("narrow");
    std::fs::create_dir_all(&wide).expect("建宽根");
    std::fs::create_dir_all(&narrow).expect("建窄根");
    std::fs::write(wide.join("secret.txt"), "WIDE-SECRET").expect("写宽根文件");
    std::fs::write(narrow.join("ok.txt"), "NARROW-OK").expect("写窄根文件");
    if !acl_round_trip(&base, "overlap") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：并发隔离探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_of = |lease: &str, root: &Path, private: &Path| FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-overlap".to_string(),
        lease: lease.to_string(),
        private: private.to_path_buf(),
        ro_tree: Vec::new(),
        rw: vec![root.to_path_buf()],
        cwd: root.to_path_buf(),
        ro: Vec::new(),
        net: false,
    };
    // 同一会话的两个不同 reach（宽/窄）+ 另一个会话（对照：撤销互不影响）。
    let wide_spec = spec_of("session-x", &wide, &base.join("private-wide"));
    let narrow_spec = spec_of("session-x", &narrow, &base.join("private-narrow"));
    let other_root = base.join("other");
    std::fs::create_dir_all(&other_root).expect("建另一会话的根");
    let other_spec = spec_of("session-y", &other_root, &base.join("private-other"));
    for spec in [&wide_spec, &narrow_spec, &other_spec] {
        expect_granted(&prepare_fence(spec, "cmd", &home), "授权应当成功");
        assert!(
            ensure_profile(&container_name(spec)).is_ok(),
            "建容器 profile"
        );
    }
    let wide_sid = container_sid(&container_name(&wide_spec)).expect("派生宽调用 SID");
    let narrow_sid = container_sid(&container_name(&narrow_spec)).expect("派生窄调用 SID");
    let other_sid = container_sid(&container_name(&other_spec)).expect("派生另一会话 SID");
    assert_ne!(
        sid_to_string(wide_sid),
        sid_to_string(narrow_sid),
        "同一会话的不同根集合必须是不同容器身份"
    );

    // 宽调用在容器里活着（约 5 秒），同时窄调用尝试读宽根。
    let wide_addr = wide_sid as usize;
    let wide_spec_bg = wide_spec.clone();
    let holding = std::thread::spawn(move || {
        // PSID 只在本次探针内有效；线程结束前 main 不会 free。
        run_in_container(
            wide_addr as PSID,
            &wide_spec_bg,
            "ping -n 6 127.0.0.1 > nul",
        )
    });
    std::thread::sleep(std::time::Duration::from_millis(800));

    // 窄调用：自己的文件读得到。
    let own = run_in_container(narrow_sid, &narrow_spec, "type ok.txt > own.txt 2>&1")
        .expect("窄调用要能启动");
    let own_text = std::fs::read_to_string(narrow.join("own.txt")).unwrap_or_default();
    assert!(
        own == 0 && own_text.contains("NARROW-OK"),
        "窄调用自己的根要读得到（exit={}，拿到 {:?}）",
        own,
        own_text
    );
    // 窄调用：宽根的密文读不到（宽调用此刻还活着）。
    let leak_cmd = format!(
        "type \"{}\" > \"{}\" 2>&1",
        wide.join("secret.txt").display(),
        narrow.join("leak.txt").display()
    );
    let leak = run_in_container(narrow_sid, &narrow_spec, &leak_cmd).expect("窄调用要能启动");
    let leak_text = std::fs::read_to_string(narrow.join("leak.txt")).unwrap_or_default();
    assert!(
        leak != 0 && !leak_text.contains("WIDE-SECRET"),
        "窄调用不该读到宽调用的额外根（exit={}，拿到 {:?}）",
        leak,
        leak_text
    );

    let wide_exit = holding.join().expect("宽调用线程");
    assert!(
        wide_exit.is_ok(),
        "宽调用容器进程应当正常结束：{:?}",
        wide_exit
    );

    // 会话级撤销：session-x 的宽/窄都撤掉，另一个会话不受影响。
    let host = crate::kernel::detail::confine::FenceHostAdapter::new(home.clone());
    host.release_by_lease("session-x")
        .expect("按会话释放应当成功");
    assert!(
        !has_any_ace_for(wide_sid, &wide) && !has_any_ace_for(narrow_sid, &narrow),
        "会话的两种 reach 都要撤掉"
    );
    assert!(
        has_ace_for(other_sid, &other_root, RIGHTS_RW),
        "别的会话的授权不能跟着撤"
    );
    assert!(
        !profile_exists(&container_name(&wide_spec))
            && !profile_exists(&container_name(&narrow_spec)),
        "本会话的 profile 要删掉"
    );
    assert!(
        profile_exists(&container_name(&other_spec)),
        "别的会话 profile 要留着"
    );

    // 再释放另一个会话：收尾不留孤儿。
    host.release_by_lease("session-y")
        .expect("第二个会话释放应当成功");
    assert!(!has_any_ace_for(other_sid, &other_root));
    assert!(!profile_exists(&container_name(&other_spec)));
    assert!(
        orphan_package_aces(&narrow).unwrap_or_default().is_empty()
            && orphan_package_aces(&wide).unwrap_or_default().is_empty()
            && orphan_package_aces(&other_root)
                .unwrap_or_default()
                .is_empty(),
        "收尾后不该留台账外的包 SID ACE"
    );
    free_sid(wide_sid);
    free_sid(narrow_sid);
    free_sid(other_sid);
    let _ = clean(&home);
    discard(&base);
}

/// 【一次性调用】构建 / 独立调用这类没有会话的调用：各自独立身份，释放一次不影响另一次；
/// 收尾要把自己的 ACE、profile 与按次临时目录都带走，不留孤儿。
#[test]
fn one_shot_release_only_removes_its_own_grant_and_profile() {
    let _fence = crate::tests::fence_acl_guard();
    use crate::kernel::ports::FenceHost;
    let base = std::env::temp_dir().join(format!("solomni-release-owner-{}", std::process::id()));
    let target = base.join("target");
    let private = base.join("private");
    std::fs::create_dir_all(&target).expect("建探针目录");
    if !acl_round_trip(&base, "release-owner") {
        eprintln!(
            "[探针] 本机做不了 ACL 完整往返（写→读回→撤）：归属释放探针跳过（不静默当作通过）"
        );
        discard(&base);
        return;
    }
    let home = base.join(".home");
    let spec_of = |lease: &str| FenceSpec {
        machine_ro_tree: Vec::new(),
        agent: "probe-per-call".to_string(),
        lease: lease.to_string(),
        private: private.clone(),
        ro_tree: Vec::new(),
        rw: vec![target.clone()],
        cwd: target.clone(),
        ro: Vec::new(),
        net: false,
    };
    let s1 = spec_of("call-1");
    let s2 = spec_of("call-2");
    // 同一 agent 的两次调用：各自授权、各自 profile。
    expect_granted(&prepare_fence(&s1, "cmd", &home), "调用 1 授权应当成功");
    expect_granted(&prepare_fence(&s2, "cmd", &home), "调用 2 授权应当成功");
    assert!(
        ensure_profile(&container_name(&s1)).is_ok()
            && ensure_profile(&container_name(&s2)).is_ok(),
        "探针要建出两个按次 profile"
    );
    let sid1 = container_sid(&container_name(&s1)).expect("派生调用 1 的 SID");
    let sid2 = container_sid(&container_name(&s2)).expect("派生调用 2 的 SID");
    assert_ne!(
        sid_to_string(sid1),
        sid_to_string(sid2),
        "同一 agent 的两次调用必须是不同容器身份"
    );
    assert!(
        has_ace_for(sid1, &target, RIGHTS_RW) && has_ace_for(sid2, &target, RIGHTS_RW),
        "两次调用各自的 ACE 都要在"
    );
    let host = crate::kernel::detail::confine::FenceHostAdapter::new(home.clone());
    // 释放调用 1：只带走它自己的 ACE / profile / 临时目录。
    host.release_call(&s1).expect("调用 1 的按次释放应当成功");
    assert!(!has_any_ace_for(sid1, &target), "调用 1 的 ACE 要撤掉");
    assert!(
        has_ace_for(sid2, &target, RIGHTS_RW),
        "调用 2 还在跑，它的 ACE 必须留着"
    );
    assert!(
        !profile_exists(&container_name(&s1)),
        "调用 1 的 profile 要删掉"
    );
    assert!(
        profile_exists(&container_name(&s2)),
        "调用 2 的 profile 要留着"
    );
    if let Some(dir) = container_package_dir(&s1) {
        assert!(
            !dir.exists(),
            "调用 1 的按次临时目录要删掉：{}",
            dir.display()
        );
    }
    // 释放调用 2：收尾不留孤儿。
    host.release_call(&s2).expect("调用 2 的按次释放应当成功");
    assert!(!has_any_ace_for(sid1, &target) && !has_any_ace_for(sid2, &target));
    assert!(
        !profile_exists(&container_name(&s2)),
        "调用 2 的 profile 要删掉"
    );
    assert!(
        orphan_package_aces(&target).unwrap_or_default().is_empty(),
        "按次收尾后不该留台账外的包 SID ACE"
    );
    free_sid(sid1);
    free_sid(sid2);
    let _ = clean(&home);
    discard(&base);
}
