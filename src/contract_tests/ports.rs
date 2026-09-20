//! 端口替身契约（docs/testing/port-matrix.md 的逐端口验收）。
//! 每个端口都验同一组语义：成功、失败传播、空/边界、交互记录、重复调用；
//! 真实适配器的对应边界在 adapters.rs，进程与 HTTP 的真实路径在 tests/cross-platform/。
//! 替身统一复用 src/tests.rs 的 `InMemory*` / `Fake*` / `Recording*`——契约测试不另造一份。

use crate::core::exec::ExecSpec;
use crate::core::fence::FenceSpec;
use crate::core::history::{AgentMeta, SessionMeta};
use crate::core::ports::{
    Chat, ChatGateway, CompleteOpts, FenceHost, HistoryStore, Log, ModelCatalog, ModuleSource, Msg,
    NoopLog, PackageSource, PromptSource, SettingsStore, SysIo, ToolRunner, Workspace,
};
use crate::core::providers::{Provider, Settings};
use crate::tests::{
    abs, module_of, FakeCatalog, InMemoryHistory, InMemoryPackages, InMemorySettings,
    InMemorySysIo, InMemoryWorkspace, NoFenceHost, RecordingFence, RecordingRunner, ScriptGateway,
    SharedScript, SilentRunner, TestPrompts, VecSource,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

fn spec() -> FenceSpec {
    FenceSpec {
        agent: "a".to_string(),
        rw: Vec::new(),
        cwd: PathBuf::from("mods").join("m0"),
        net: false,
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
        name: name.to_string(),
        mode: "direct".to_string(),
        delegate: false,
        modules: vec!["a".to_string()],
        task: None,
        ts: 7,
        agents: vec![AgentMeta {
            name: "a".to_string(),
            transient: false,
            modules: vec!["a".to_string()],
            model: None,
        }],
        exec: ExecSpec::default(),
    }
}

// ---------- Chat / ChatGateway ----------

/// SharedScript 是**明确的非流式替身**：即便 stream = true 也不回调（矩阵里已如实登记）。
/// 它验的是端口的最小语义：按序回放、末条重复兜底、空队列 = 空串。
#[test]
fn chat_double_shared_script_replays_in_order_and_is_explicitly_non_streaming() {
    let mut c = SharedScript {
        q: Arc::new(Mutex::new(vec!["一".to_string(), "二".to_string()])),
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
    let (mut c, notice) = gw.member_channel(None, "a");
    assert!(notice.is_none(), "脚本替身不做回落，就不该编造通知");
    assert!(c
        .complete(&[], CompleteOpts::plain(false), &mut |_| true)
        .raw
        .contains("甲"));
    // 未登记的 agent：回落一条演示发言（不是 panic，也不是空串）。
    let (mut c2, _) = gw.member_channel(None, "没登记");
    assert!(
        !c2.complete(&[], CompleteOpts::plain(false), &mut |_| true)
            .raw
            .is_empty(),
        "未登记也要能应答"
    );
    // 核心通道与成员通道是两条独立队列，互不污染。
    let (mut core, demo) = gw.core_channel(None);
    assert!(!demo, "脚本替身不是演示通道");
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
    assert_eq!(c.list_models(&p).unwrap(), vec!["m1", "m2"]);
    assert_eq!(
        c.list_models(&p).unwrap(),
        vec!["m1", "m2"],
        "重复调用结果稳定"
    );
    assert_eq!(
        c.seen.lock().expect("锁").len(),
        2,
        "每次调用都要记录收到的通道"
    );
    assert_eq!(c.seen.lock().expect("锁")[0].base_url, "http://test");
    assert!(
        FakeCatalog::new(Vec::new())
            .list_models(&p)
            .unwrap()
            .is_empty(),
        "空结果不是错误"
    );
    let bad = FakeCatalog::new(vec!["m1".to_string()]).fail_with("发现失败");
    assert_eq!(bad.list_models(&p).unwrap_err(), "发现失败");
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
    io.write(&f, "改写").expect("写成功");
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
    assert_eq!(bad.write(&f, "x").unwrap_err(), "磁盘故障");
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
    let listed = h.list().expect("列会话");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].done, "ended 之后 done = true");
    let mut renamed = meta("w1");
    renamed.mode = "collab".to_string();
    h.save_meta(&renamed).expect("写回元信息");
    assert_eq!(
        h.load("w1").expect("读回").0.mode,
        "collab",
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
    assert_eq!(bad.load("x").unwrap_err(), "历史不可用");
    assert_eq!(bad.delete("x").unwrap_err(), "历史不可用");
}

// ---------- PromptSource ----------

#[test]
fn prompt_source_double_loads_the_builtin_book_and_propagates_failure() {
    let p = TestPrompts::ok().load().expect("内置提示词册必须合法");
    assert!(!p.core.no_agents.is_empty(), "册子里的话术不能是空的");
    assert_eq!(
        TestPrompts::ok()
            .load()
            .expect("重复加载稳定")
            .core
            .no_agents,
        p.core.no_agents
    );
    assert_eq!(
        TestPrompts::ok().fail_with("册子缺失").load().unwrap_err(),
        "册子缺失"
    );
}

// ---------- ToolRunner ----------

#[test]
fn tool_runner_double_records_the_call_site_and_reports_failure_honestly() {
    let fence = spec();
    let ok = RecordingRunner::new("输出", true);
    let out = ok.run(&fence, "python tools/x.py", "{\"k\":1}");
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
    let out = bad.run(&fence, "x", "{}");
    assert!(
        !out.ok && out.output == "失败原因",
        "失败必须如实回执（ok = false + 原因）"
    );
}

#[test]
#[should_panic(expected = "不应调用工具")]
fn silent_runner_double_fails_loudly_on_any_call() {
    let _ = SilentRunner.run(&spec(), "x", "{}");
}

// ---------- FenceHost ----------

#[test]
fn fence_host_doubles_release_record_and_propagate_failure() {
    let s = spec();
    NoFenceHost
        .release(&s)
        .expect("本平台没有该机制时，空操作实现必须成功");
    let rec = RecordingFence::new();
    rec.release(&s).expect("记录型实现成功");
    rec.release(&s).expect("重复撤销不报错（幂等）");
    assert_eq!(rec.released.lock().expect("锁").as_slice(), ["a", "a"]);

    let bad = RecordingFence::new().fail_with("撤权失败");
    assert_eq!(
        bad.release(&s).unwrap_err(),
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
