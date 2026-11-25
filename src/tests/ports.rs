//! 目的：逐端口验收 docs/testing/doubles.md 的替身语义。
//! 管：成功、失败传播、空/边界、交互记录、重复调用；Chat/ChatGateway/Settings/ModelCatalog/ModuleSource/Package/Workspace/WorkStore 等替身。
//! 不管：真实适配器（adapters.rs）与进程/HTTP 真实路径。
//! 联动：docs/testing/doubles.md、tests/doubles、adapters.rs。

use super::builders::{RecordingRunner, SilentRunner};
use super::doubles::{
    FakeCatalog, InMemoryHistory, InMemoryPackages, InMemorySettings, InMemorySysIo,
    InMemoryWorkStore, InMemoryWorkspace, NoFenceHost, RecordingFence, ScriptGateway, SharedScript,
    TestPrompts, VecSource, abs, module_of,
};
use crate::units::catalog::modules::ports::ModuleSource;
use crate::units::catalog::packages::ports::PackageSource;
use crate::units::catalog::registry::api::AgentMeta;
use crate::units::catalog::registry::api::{Provider, Settings};
use crate::units::catalog::registry::ports::SettingsStore;
use crate::units::conversation::session::api::{MEETING, SINGLE, SessionMeta, SessionSpec};
use crate::units::conversation::session::ports::HistoryStore;
use crate::units::llm::api::{Channel, Chat, CompleteOpts, Msg};
use crate::units::llm::ports::{ChatGateway, ModelCatalog};
use crate::units::prompt::ports::PromptSource;
use crate::units::tools::ports::SysIo;
use crate::units::workspace::ports::Workdirs;
use crate::kernel::api::fence::FencePlan;
use crate::units::exec_model::api::ExecSpec;
use crate::kernel::ports::{FenceHost, ProcessRunner};
use crate::kernel::ports::{Log, NoopLog};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn spec() -> FencePlan {
    FencePlan {
        agent: "a".to_string(),
        lease: String::new(),
        home: PathBuf::new(),
        cwd: PathBuf::from("mods").join("m0"),
        net: (false).into(),
        grants: crate::kernel::contracts::fence::grants_of(Vec::new(), Vec::new(), Vec::new()),
    }
}

fn provider() -> Provider {
    Provider {
        base_url: "http://test".to_string(),
        api_key: "k".to_string(),
    }
}

fn meta(name: &str) -> SessionMeta {
    SessionMeta {
        usage_totals: Default::default(),
        context_freshness: false,
        freshness_keep_percent: 80,
        name: name.to_string(),
        spec: SessionSpec::plain(SINGLE, "单机", "solo"),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 7,
        agents: vec![AgentMeta {
            name: "a".to_string(),
            transient: false,
            modules: vec!["a".to_string()],
            model: None,
            chara: String::new(),
        }],
        exec: ExecSpec::default(),
        sandbox: Default::default(),
        shared: Default::default(),
        parent: None,
        node: None,
        delegation: None,
        thinking: None,
        run: crate::units::conversation::session::api::RunState::Active,
        workspace: None,
    }
}

// ---------- Chat / ChatGateway ----------
/// SharedScript 是**明确的非流式替身**：即便 stream = true 也不回调（矩阵里已如实登记）。
/// 它验的是端口的最小语义：按序回放、末条重复兜底、空队列 = 空串。
#[test]
fn chat_double_shared_script_replays_in_order_and_is_explicitly_non_streaming() {
    let mut c = SharedScript {
        q: Arc::new(Mutex::new(vec!["一".to_string(), "二".to_string()])),
        usage: None,
    };
    let mut chunks = 0;
    assert_eq!(
        c.complete(&[Msg::user("x")], CompleteOpts::plain(true), &mut |_| {
            chunks += 1;
            true
        })
        .raw,
        "一"
    );
    assert_eq!(chunks, 0, "非流式替身不回调：不得假装有流式");
    assert_eq!(
        c.complete(&[Msg::user("x")], CompleteOpts::plain(false), &mut |_| true)
            .raw,
        "二"
    );
    assert_eq!(
        c.complete(&[Msg::user("x")], CompleteOpts::plain(false), &mut |_| true)
            .raw,
        "二",
        "末条重复兜底"
    );
    let mut empty = SharedScript {
        q: Arc::new(Mutex::new(Vec::new())),
        usage: None,
    };
    assert_eq!(
        empty
            .complete(&[Msg::user("x")], CompleteOpts::plain(false), &mut |_| true)
            .raw,
        "",
        "空脚本 = 空串"
    );
}

#[test]
fn chat_gateway_double_scripts_member_and_core_channels_separately() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"甲\"}".to_string()],
    );
    let gw = ScriptGateway::new(member, vec!["[]".to_string()]);
    let channel = Channel {
        base_url: "http://test".to_string(),
        api_key: "k".to_string(),
        model: "m".to_string(),
    };
    let mut c = gw.member_channel(&channel, "a");
    assert!(
        c.complete(&[], CompleteOpts::plain(false), &mut |_| true)
            .raw
            .contains("甲")
    );
    // 未登记的 agent：脚本替身给一条兜底脚本（不是 panic，也不是空串）。
    let mut c2 = gw.member_channel(&channel, "没登记");
    assert!(
        !c2.complete(&[], CompleteOpts::plain(false), &mut |_| true)
            .raw
            .is_empty(),
        "未登记也要能应答"
    );
    // 核心通道与成员通道是两条独立队列，互不污染。
    let mut core = gw.core_channel(&channel);
    assert_eq!(
        core.complete(&[], CompleteOpts::plain(false), &mut |_| true)
            .raw,
        "[]"
    );
    assert!(
        c2.complete(&[], CompleteOpts::plain(false), &mut |_| true)
            .raw
            .contains("（演示）"),
        "成员队列不受核心通道影响"
    );
}

// ---------- SettingsStore ----------
#[test]
fn settings_store_double_roundtrips_and_propagates_failure() {
    let store = InMemorySettings::new();
    let mut s = store.load().expect("预置登记处可读");
    assert!(s.providers.contains_key("p") && s.core.as_deref() == Some("m"));
    s.app.streaming = false;
    store.save(&s).expect("保存成功");
    assert!(
        !store.load().expect("再读").app.streaming,
        "save 后再 load 必须看到新值"
    );

    let bad = InMemorySettings::new().fail_with("登记处损坏");
    assert_eq!(
        bad.load().unwrap_err(),
        "登记处损坏",
        "读取失败必须如实传播"
    );
    assert_eq!(
        bad.save(&Settings::default()).unwrap_err(),
        "登记处损坏",
        "写入失败同样传播"
    );
}

// ---------- ModelCatalog ----------
#[test]
fn model_catalog_double_lists_records_and_propagates_failure() {
    let c = FakeCatalog::new(vec!["m1".to_string(), "m2".to_string()]);
    let p = provider();
    assert_eq!(
        c.list_models(&p.base_url, &p.api_key).unwrap(),
        vec!["m1", "m2"]
    );
    assert_eq!(
        c.list_models(&p.base_url, &p.api_key).unwrap(),
        vec!["m1", "m2"],
        "重复调用结果稳定"
    );
    assert_eq!(
        c.seen.lock().expect("锁").len(),
        2,
        "每次调用都要记录收到的通道"
    );
    assert_eq!(c.seen.lock().expect("锁")[0], "http://test");
    assert!(
        FakeCatalog::new(Vec::new())
            .list_models(&p.base_url, &p.api_key)
            .unwrap()
            .is_empty(),
        "空结果不是错误"
    );
    let bad = FakeCatalog::new(vec!["m1".to_string()]).fail_with("发现失败");
    assert_eq!(
        bad.list_models(&p.base_url, &p.api_key).unwrap_err(),
        "发现失败"
    );
    assert!(
        bad.seen.lock().expect("锁").is_empty(),
        "失败路径不该留下'已发现'的假记录"
    );
}

// ---------- ModuleSource ----------
#[test]
fn module_source_double_scans_a_pure_function_of_the_given_list() {
    let src = VecSource(vec![module_of("a"), module_of("b")]);
    let first = src.scan();
    assert_eq!(first.modules.len(), 2);
    assert!(first.rejected.is_empty());
    assert_eq!(
        src.scan().modules[0].manifest.id,
        "a",
        "重扫顺序稳定（清单即事实）"
    );
    assert!(
        VecSource(Vec::new()).scan().modules.is_empty(),
        "空清单不是错误"
    );
}

// ---------- PackageSource ----------
#[test]
fn package_source_double_builds_a_library_and_reports_its_dir() {
    let empty = InMemoryPackages::empty();
    assert!(empty.scan().packages.is_empty());
    let lib = InMemoryPackages::with(&["id: python\nversion: 3.12.4\nprefix: opt/rt/py"]).scan();
    assert_eq!(lib.versions_of("python").len(), 1);
    assert!(lib.rejected.is_empty(), "{:?}", lib.rejected);
    // 非法清单：拒收原因必须随包库一起给出（不静默丢）。
    let bad = InMemoryPackages::with(&["id: Python\nversion: 1\nprefix: opt/p"]).scan();
    assert!(!bad.rejected.is_empty(), "非法清单要留下原因");
    assert!(
        empty.dir().to_string_lossy().ends_with("runtimes"),
        "dir() 要如实报包库位置"
    );
}

// ---------- Workspace ----------
#[test]
fn workspace_double_prepares_writes_lists_and_propagates_failure() {
    let ws = InMemoryWorkspace::new();
    ws.prepare("w", &["a".to_string()]).expect("准备成功");
    let roots = ws.roots("w", &["a".to_string()]).expect("取根");
    assert!(
        roots.shared.is_absolute(),
        "根一律绝对路径（相对路径会被围栏判为越界）"
    );
    assert!(roots.agents.contains_key("a"));
    assert!(!ws.work_has("w", "note.txt"));
    ws.write_work("w", "note.txt", b"hi").expect("写入成功");
    assert!(ws.work_has("w", "note.txt"), "写入后必须可见");
    assert_eq!(
        ws.list("w", &["a".to_string()]).expect("列文件").work,
        vec!["note.txt".to_string()]
    );

    let bad = InMemoryWorkspace::new().fail_with("工作区不可用");
    assert_eq!(bad.prepare("w", &[]).unwrap_err(), "工作区不可用");
    assert_eq!(bad.roots("w", &[]).unwrap_err(), "工作区不可用");
    assert_eq!(bad.write_work("w", "x", b"").unwrap_err(), "工作区不可用");
    assert_eq!(bad.list("w", &[]).unwrap_err(), "工作区不可用");
    assert!(
        !bad.work_has("w", "x"),
        "布尔查询没有错误通道，不受失败注入影响"
    );
}

/// 目的：内存版本库替身与真实适配器同一份资产流式契约：扫描、写入校验、复制、超限与缺失。
#[test]
fn work_store_double_streams_assets_and_rejects_oversize_or_mismatch() {
    use crate::units::workspace::api::content_hash;
    use crate::units::workspace::ports::WorkStore;
    let s = InMemoryWorkStore::new();
    let root = abs(&["w", "a"]);
    let other = abs(&["w", "b"]);
    let store = abs(&["w", ".work"]);
    let data: Vec<u8> = (0..10_000u32).map(|i| (i % 251) as u8).collect();
    s.write_under(&root, "clip.bin", &data).unwrap();

    assert_eq!(s.stat_under(&root, "clip.bin").unwrap(), Some(10_000));
    assert_eq!(s.scan_under(&root, "nope", None).unwrap(), None);
    let scanned = s.scan_under(&root, "clip.bin", None).unwrap().unwrap();
    assert_eq!(scanned.hash, content_hash(&data));
    assert!(scanned.text.is_none());
    let hash = scanned.hash.clone();

    assert_eq!(
        s.ingest_under(&store, &root, "clip.bin", &hash, 20_000)
            .unwrap(),
        10_000
    );
    assert_eq!(s.stat_object(&store, &hash).unwrap(), 10_000);
    assert_eq!(s.scan_object(&store, &hash, None).unwrap().hash, hash);
    s.copy_object_to_under(&store, &hash, &other, "sub/clip.bin")
        .unwrap();
    assert_eq!(
        s.scan_under(&other, "sub/clip.bin", None)
            .unwrap()
            .unwrap()
            .hash,
        hash
    );

    // 跨根复制（备份 / 恢复原语）：源指纹校验，目标根按传入路径隔离。
    let lost = abs(&["w", ".work", "lost", "a"]);
    assert_eq!(
        s.copy_under_to_under(&root, "clip.bin", &lost, "clip.bin", &hash)
            .unwrap(),
        10_000
    );
    assert_eq!(
        s.scan_under(&lost, "clip.bin", None).unwrap().unwrap().hash,
        hash
    );
    let bad_cross = content_hash(b"different");
    assert!(
        s.copy_under_to_under(&root, "clip.bin", &lost, "x", &bad_cross)
            .is_err()
    );
    assert!(s.scan_under(&lost, "x", None).unwrap().is_none());

    let bad = content_hash(b"different");
    let err = s
        .ingest_under(&store, &root, "clip.bin", &bad, 20_000)
        .unwrap_err();
    assert!(err.contains("改动"), "{}", err);
    assert!(s.stat_object(&store, &bad).is_err());

    s.write_under(&root, "big.bin", &[0u8; 32]).unwrap();
    let big = content_hash(&[0u8; 32]);
    let err = s
        .ingest_under(&store, &root, "big.bin", &big, 16)
        .unwrap_err();
    assert!(err.contains("上限"), "{}", err);
    assert!(s.stat_object(&store, &big).is_err());
    assert!(s.scan_object(&store, "nope", None).is_err());
    assert!(s.copy_object_to_under(&store, "nope", &other, "x").is_err());

    let bad = InMemoryWorkStore::new().fail_with("版本库不可用");
    assert_eq!(
        bad.scan_under(&root, "x", None).unwrap_err(),
        "版本库不可用"
    );
    assert_eq!(bad.stat_object(&store, "h").unwrap_err(), "版本库不可用");
}

/// 目的：版本库替身与真实适配器同一份过滤契约：`list_visible` 按 `.workignore` 剪枝，`read_text` 缺文件给 None。
#[test]
fn work_store_double_lists_visible_files_and_reads_small_text() {
    use crate::units::workspace::ports::WorkStore;
    let s = InMemoryWorkStore::new();
    let root = abs(&["w", "work"]);
    for rel in ["README.md", "src/main.rs", "node_modules/pkg/index.js"] {
        s.write_under(&root, rel, rel.as_bytes()).unwrap();
    }

    assert_eq!(s.list(&root).unwrap().len(), 3, "不过滤时列全");
    assert_eq!(
        s.list_visible(&root, Some("node_modules\n")).unwrap(),
        vec!["README.md".to_string(), "src/main.rs".to_string()],
        "按规则剪掉被忽略的目录"
    );
    assert_eq!(
        s.list_visible(&root, None).unwrap().len(),
        3,
        "None 与不过滤等价"
    );
    assert!(
        !s.list_visible(&root, Some("[z-a]\n"))
            .unwrap_err()
            .is_empty(),
        "非法规则要如实报错，不静默忽略"
    );

    let rules = abs(&["w", ".work", ".workignore"]);
    assert_eq!(s.read_text(&rules).unwrap(), None, "缺文件给 None");
    s.write_under(&abs(&["w", ".work"]), ".workignore", b"target\n")
        .unwrap();
    assert_eq!(s.read_text(&rules).unwrap().as_deref(), Some("target\n"));
    // write_text 覆盖整份小文本，父目录不存在也能写（规则文件的唯一写入口）。
    s.write_text(&rules, "target\nvenv\n").unwrap();
    assert_eq!(
        s.read_text(&rules).unwrap().as_deref(),
        Some("target\nvenv\n"),
        "整份覆盖，不追加"
    );
    let nested = abs(&["w2", ".work", "nested", ".workignore"]);
    s.write_text(&nested, "x\n").unwrap();
    assert_eq!(s.read_text(&nested).unwrap().as_deref(), Some("x\n"));
}

// ---------- SysIo ----------
#[test]
fn sys_io_double_reads_writes_and_propagates_failure() {
    let io = InMemorySysIo::new();
    let f = abs(&["w", "note.txt"]);
    io.seed(&["w", "note.txt"], "内容");
    let r = io.read(&f).expect("读成功");
    assert_eq!(r.text, "内容");
    assert_eq!(r.bytes, "内容".len(), "bytes 是原始字节数，不是字符数");
    assert!(!r.lossy && !r.cut, "替身不做解码猜测，也不截断");
    io.mutate(
        &f,
        &crate::units::tools::ports::Expect::Version(r.hash.clone()),
        crate::units::tools::ports::Mutation::Replace("改写".to_string()),
    )
    .expect("写成功");
    assert_eq!(
        io.get(&["w", "note.txt"]).as_deref(),
        Some("改写"),
        "写后读必须看到新内容"
    );
    assert!(
        io.read(&abs(&["w", "nope.txt"]))
            .err()
            .expect("应当失败")
            .contains("不存在"),
        "缺文件要如实报错"
    );

    let bad = InMemorySysIo::new().fail_with("磁盘故障");
    assert_eq!(bad.read(&f).err().expect("应当失败"), "磁盘故障");
    let fault = bad
        .mutate(
            &f,
            &crate::units::tools::ports::Expect::Absent,
            crate::units::tools::ports::Mutation::Replace("x".to_string()),
        )
        .unwrap_err();
    assert_eq!(fault.code, "FS_IO");
    assert!(fault.message.contains("磁盘故障"), "{}", fault.message);
}

#[test]
fn sys_io_double_walks_by_rules_scans_lines_and_reads_small() {
    use crate::units::tools::ports::WalkOpts;
    let io = InMemorySysIo::new();
    io.seed(&["w", "a.txt"], "one\ntwo\n");
    io.seed(&["w", "sub", "b.txt"], "x\n");
    io.seed(&["w", "vendor", "skip.txt"], "y\n");
    io.seed(&["w", ".hidden.txt"], "z\n");
    let root = abs(&["w"]);
    let walked = io
        .walk(
            &root,
            Some("vendor/\n"),
            WalkOpts {
                hidden: false,
                ignore: true,
            },
        )
        .expect("遍历成功");
    let rels: Vec<&str> = walked.iter().map(|e| e.rel.as_str()).collect();
    assert_eq!(
        rels,
        vec!["a.txt", "sub/b.txt"],
        "规则与隐藏文件都剪掉：{:?}",
        rels
    );
    let all = io
        .walk(
            &root,
            None,
            WalkOpts {
                hidden: true,
                ignore: false,
            },
        )
        .expect("遍历成功");
    assert_eq!(all.len(), 4, "关规则并放行隐藏：{}", all.len());

    let mut seen: Vec<(u32, String)> = Vec::new();
    let summary = io
        .scan_lines(&abs(&["w", "a.txt"]), &mut |n, line| {
            seen.push((n, line.to_string()));
        })
        .expect("扫描成功");
    assert_eq!(summary.lines, 2, "行号从 1 数到 2");
    assert!(!summary.binary && !summary.lossy);
    assert_eq!(seen, vec![(1, "one".to_string()), (2, "two".to_string())]);

    io.seed(&["w", "bin.dat"], "x\0y\n");
    let mut called = 0;
    let summary = io
        .scan_lines(&abs(&["w", "bin.dat"]), &mut |_, _| {
            called += 1;
        })
        .expect("扫描成功");
    assert!(summary.binary, "含 NUL 判二进制");
    assert_eq!(called, 0, "二进制不回调任何行");

    io.seed(&["w", ".workignore"], "vendor/\n");
    assert_eq!(
        io.read_small(&abs(&["w", ".workignore"]))
            .expect("读成功")
            .as_deref(),
        Some("vendor/\n")
    );
    assert!(
        io.read_small(&abs(&["w", "nope.txt"]))
            .expect("读成功")
            .is_none(),
        "缺文件 = None，不是错误"
    );
}

#[test]
fn sys_io_double_mutate_enforces_expectations_and_transact_is_all_or_nothing() {
    use crate::units::tools::api::LineRange;
    use crate::units::tools::ports::{Expect, Mutation, WriteOp};

    let io = InMemorySysIo::new();
    let f = abs(&["w", "note.txt"]);
    io.seed(&["w", "note.txt"], "一\n二\n");
    let before = io.read(&f).expect("读成功");
    let receipt = io
        .mutate(
            &f,
            &Expect::Version(before.hash.clone()),
            Mutation::Edit {
                old: "二".to_string(),
                new: "贰".to_string(),
                all: false,
            },
        )
        .expect("编辑成功");
    assert_eq!(io.get(&["w", "note.txt"]).as_deref(), Some("一\n贰\n"));
    assert_eq!(receipt.changed, vec![LineRange::new(2, 2)]);
    assert_eq!(receipt.replaced, 1);

    // 版本已变时再写：锁内版本比较失败，内容保持不动。
    let stale = io
        .mutate(
            &f,
            &Expect::Version(before.hash),
            Mutation::Replace("覆盖".to_string()),
        )
        .unwrap_err();
    assert_eq!(stale.code, "FS_STALE_VERSION");
    assert_eq!(io.get(&["w", "note.txt"]).as_deref(), Some("一\n贰\n"));

    // 新建语义：已存在就不再覆盖。
    let exists = io
        .mutate(&f, &Expect::Absent, Mutation::Replace("覆盖".to_string()))
        .unwrap_err();
    assert_eq!(exists.code, "FS_ALREADY_EXISTS");

    // 事务：第 2 个路径失败时，第 1 个也不能留下。
    let a = abs(&["w", "a.txt"]);
    let b = abs(&["w", "b.txt"]);
    io.fail_on(&["w", "b.txt"]);
    let err = io
        .transact(vec![
            WriteOp {
                path: a.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("甲".to_string()),
            },
            WriteOp {
                path: b.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("乙".to_string()),
            },
        ])
        .unwrap_err();
    assert_eq!(err.code, "FS_IO");
    assert_eq!(io.get(&["w", "a.txt"]), None, "事务失败不能留下半个结果");
    assert_eq!(io.get(&["w", "b.txt"]), None);
}

#[test]
fn sys_io_double_transact_rejects_the_same_path_twice() {
    use crate::units::tools::ports::{Expect, Mutation, WriteOp};
    let io = InMemorySysIo::new();
    let f = abs(&["w", "same.txt"]);
    let err = io
        .transact(vec![
            WriteOp {
                path: f.clone(),
                expect: Expect::Absent,
                mutation: Mutation::Replace("一".to_string()),
            },
            WriteOp {
                path: f,
                expect: Expect::Absent,
                mutation: Mutation::Replace("二".to_string()),
            },
        ])
        .unwrap_err();
    assert_eq!(err.code, "FS_DUPLICATE_PATH");
}

// ---------- HistoryStore ----------
#[test]
fn history_store_double_roundtrips_lists_deletes_and_propagates_failure() {
    let h = InMemoryHistory::new();
    h.create(&meta("w1")).expect("创建成功");
    assert!(h.load("w1").expect("读回").1.is_empty(), "新会话没有流水");
    h.append("w1", &[serde_json::json!({"type": "say", "text": "hi"})])
        .expect("追加成功");
    h.append("w1", &[serde_json::json!({"type": "ended"})])
        .expect("追加成功");
    let (m, events) = h.load("w1").expect("读回");
    assert_eq!(m.name, "w1");
    assert_eq!(events.len(), 2, "流水只追加，不丢行");
    // 只读元信息：与 load 是同一份真相，但不回放流水。
    assert_eq!(h.meta("w1").expect("读元信息").name, "w1");
    let listed = h.list().expect("列会话");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].done, "ended 之后 done = true");
    let mut renamed = meta("w1");
    renamed.spec = SessionSpec::plain(MEETING, "会议", "discussant");
    h.save_meta(&renamed).expect("写回元信息");
    assert_eq!(
        h.load("w1").expect("读回").0.spec.kind,
        MEETING,
        "meta 是会话身份的唯一真相"
    );
    assert!(h.delete("w1").expect("删除成功"), "删除已存在的会话 = true");
    assert!(!h.delete("w1").expect("再删"), "重复删除 = false，不是错误");
    assert!(
        h.load("w1").unwrap_err().contains("无此会话"),
        "删掉之后读不到要如实报错"
    );

    let bad = InMemoryHistory::new().fail_with("历史不可用");
    assert_eq!(bad.create(&meta("x")).unwrap_err(), "历史不可用");
    assert_eq!(bad.save_meta(&meta("x")).unwrap_err(), "历史不可用");
    assert_eq!(bad.append("x", &[]).unwrap_err(), "历史不可用");
    assert_eq!(bad.list().unwrap_err(), "历史不可用");
    assert_eq!(bad.meta("x").unwrap_err(), "历史不可用");
    assert_eq!(bad.load("x").unwrap_err(), "历史不可用");
    assert_eq!(bad.delete("x").unwrap_err(), "历史不可用");
}

// ---------- PromptSource ----------
#[test]
fn prompt_source_double_loads_the_builtin_book_and_propagates_failure() {
    let p = TestPrompts::ok().load().expect("内置提示词册必须合法");
    assert!(!p.core.chat_protocol.is_empty(), "册子里的段落不能是空的");
    assert_eq!(
        TestPrompts::ok()
            .load()
            .expect("重复加载稳定")
            .core
            .chat_protocol,
        p.core.chat_protocol
    );
    assert_eq!(
        TestPrompts::ok().fail_with("册子缺失").load().unwrap_err(),
        "册子缺失"
    );
}

// ---------- ProcessRunner ----------
#[test]
fn tool_runner_double_records_the_call_site_and_reports_failure_honestly() {
    let fence = spec();
    let ok = RecordingRunner::new("输出", true);
    let out = ok.run(
        &fence,
        "python tools/x.py",
        "{\"k\":1}",
        &[],
        None,
        None,
        None,
    );
    assert!(out.ok && out.output == "输出");
    let calls = ok.calls.lock().expect("锁");
    assert_eq!(calls.len(), 1, "每次调用都要留现场");
    assert_eq!(
        calls[0].0, fence.cwd,
        "记录的是工具进程的工作目录（= 该模块的根）"
    );
    assert_eq!(calls[0].1, "python tools/x.py");
    assert_eq!(calls[0].2, "{\"k\":1}");
    drop(calls);

    let bad = RecordingRunner::new("失败原因", false);
    let out = bad.run(&fence, "x", "{}", &[], None, None, None);
    assert!(
        !out.ok && out.output == "失败原因",
        "失败必须如实回执（ok = false + 原因）"
    );
}

#[test]
#[should_panic(expected = "不应调用工具")]
fn silent_runner_double_fails_loudly_on_any_call() {
    let _ = SilentRunner.run(&spec(), "x", "{}", &[], None, None, None);
}

// ---------- FenceHost ----------
#[test]
fn fence_host_doubles_release_record_and_propagate_failure() {
    let s = spec();
    NoFenceHost
        .release_call(&s)
        .expect("本平台没有该机制时，空操作实现必须成功");
    let rec = RecordingFence::new();
    rec.release_call(&s).expect("记录型实现成功");
    rec.release_call(&s).expect("重复撤销不报错（幂等）");
    assert_eq!(rec.released.lock().expect("锁").as_slice(), ["a", "a"]);

    let bad = RecordingFence::new().fail_with("撤权失败");
    assert_eq!(
        bad.release_call(&s).unwrap_err(),
        "撤权失败",
        "撤权失败不得被当成已撤销"
    );
    assert!(
        bad.released.lock().expect("锁").is_empty(),
        "失败时不该留下'已撤销'的假记录"
    );
}

// ---------- Log ----------
#[test]
fn noop_log_is_silent_and_shareable_across_threads() {
    NoopLog.info("at", "m");
    NoopLog.warn("at", "m");
    NoopLog.error("at", "m");
    let shared: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    let handle = std::thread::spawn(move || shared.error("线程", "也要能记"));
    handle
        .join()
        .expect("跨线程可用（核心与 Web 泵线程共用同一个日志端口）");
}

// ---------- HostProbe ----------
/// 真实适配器：路径事实按真实文件系统回答（scratch 里真建一个目录与一个文件）。
#[test]
fn host_probe_adapter_reports_real_path_facts_and_absent_exes() {
    use crate::kernel::ports::HostProbe;
    let real = crate::kernel::detail::HostProbeAdapter;
    let dir = crate::tests::scratch("host-probe");
    let file = dir.join("a.txt");
    std::fs::write(&file, b"x").unwrap();
    assert!(real.is_dir(&dir), "真目录要认得");
    assert!(real.is_file(&file), "真文件要认得");
    assert!(!real.is_file(&dir), "目录不是文件");
    assert!(!real.is_dir(&file), "文件不是目录");
    assert!(!real.is_file(&dir.join("nope")), "不存在的路径一律不算");
    assert!(
        !real.has_exe("definitely-not-an-exe-solomni"),
        "PATH 上没有的可执行文件要如实说没有"
    );
}

/// 替身：**只按给定答案回答**，不读真实环境（用例要确定性）。
#[test]
fn fixed_probe_answers_only_what_was_declared() {
    use crate::kernel::ports::HostProbe;
    use crate::tests::doubles::FixedProbe;
    let dir = std::path::PathBuf::from("some-dir");
    let file = std::path::PathBuf::from("some-file");
    let p = FixedProbe {
        files: vec![file.clone()],
        dirs: vec![dir.clone()],
        exes: vec!["qemu-system-x86_64".to_string()],
        hypervisor: true,
        guests: Vec::new(),
        not_executable: Vec::new(),
    };
    assert!(p.is_dir(&dir) && !p.is_dir(&file), "声明之外的目录一律不算");
    assert!(
        p.is_file(&file) && !p.is_file(&dir),
        "声明之外的文件一律不算"
    );
    assert!(p.has_exe("qemu-system-x86_64") && !p.has_exe("other"));
    assert!(p.hypervisor_available(), "虚拟化按声明回答");
    assert!(!FixedProbe::default().hypervisor_available(), "缺省一律否");
}

/// 目的：真实宿主探针的"能直接跑"（`is_executable`）按权限位判——只在场不算，这与"在场"是两件事。
#[cfg(unix)]
#[test]
fn real_probe_is_executable_follows_the_mode_bit() {
    use crate::kernel::ports::HostProbe;
    use std::os::unix::fs::PermissionsExt;
    let root = crate::tests::scratch("probe-exec");
    let script = root.join("run.sh");
    std::fs::write(&script, "#!/bin/sh\n").expect("写脚本");
    let probe = crate::kernel::detail::host_probe::HostProbeAdapter;
    assert!(!probe.is_executable(&script), "没有执行位就不算能跑");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("给执行位");
    assert!(probe.is_executable(&script), "有执行位才算能跑");
    assert!(
        !probe.is_executable(&root.join("nowhere")),
        "不在场当然不算能跑"
    );
}
