//! 真实适配器边界（docs/testing/port-matrix.md 的「真实适配器」列）。
//! 一律用隔离根 `target/test-scratch/contract/<name>`（target/ 不入库），跑完删除；
//! 不碰真实 `.home/` 与 `session/`。HTTP 适配器只对本机环回假供应商说话（无外网、无真实密钥）。

use crate::adapters::endpoint::memo_new;
use crate::adapters::fs_history::FsHistory;
use crate::adapters::fs_modules::FsModules;
use crate::adapters::fs_packages::FsPackages;
use crate::adapters::fs_workspace::FsWorkspace;
use crate::adapters::http_chat::HttpGateway;
use crate::adapters::log::FileLog;
use crate::adapters::model_catalog::HttpModelCatalog;
use crate::adapters::sys_io::FsSysIo;
use crate::adapters::yaml_prompts::YamlPrompts;
use crate::adapters::yaml_settings::YamlSettingsStore;
use crate::contract_tests::scratch;
use crate::core::exec::ExecSpec;
use crate::core::history::{AgentMeta, SessionMeta};
use crate::core::ports::{
    ChatGateway, Chunk, CompleteOpts, HistoryStore, Log, ModelCatalog, ModuleSource, Msg, NoopLog,
    PackageSource, ProbeOutcome, PromptSource, SettingsStore, SysIo, Workspace,
};
use crate::core::providers::{Channel, Provider, Settings};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

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

// ---------- FsWorkspace ----------

#[test]
fn fs_workspace_builds_the_real_layout_and_lists_only_files() {
    let root = scratch("fs-workspace");
    let sessions = root.join("session");
    let ws = FsWorkspace::new(sessions.clone());
    ws.prepare("w", &["a".to_string(), "b".to_string()])
        .expect("建目录成功");
    assert!(
        sessions.join("w").join("work").is_dir(),
        "共享区要真的建出来"
    );
    assert!(
        sessions.join("w").join("a").is_dir() && sessions.join("w").join("b").is_dir(),
        "每个 agent 一个私有沙箱"
    );

    assert!(!ws.work_has("w", "note.txt"));
    ws.write_work("w", "note.txt", "你好".as_bytes())
        .expect("写入成功");
    assert!(ws.work_has("w", "note.txt"));
    assert_eq!(
        std::fs::read_to_string(sessions.join("w").join("work").join("note.txt")).expect("读回"),
        "你好"
    );

    std::fs::create_dir_all(sessions.join("w").join("a").join("sub")).expect("造子目录");
    std::fs::write(
        sessions.join("w").join("a").join("sub").join("deep.txt"),
        "x",
    )
    .expect("造私有文件");
    let files = ws.list("w", &["a".to_string()]).expect("列文件");
    assert_eq!(
        files.work,
        vec!["note.txt".to_string()],
        "相对路径、/ 分隔、排序稳定"
    );
    assert_eq!(
        files.agents.get("a").map(|v| v.as_slice()),
        Some(&["sub/deep.txt".to_string()][..]),
        "递归列出，只列文件"
    );

    let roots = ws.roots("w", &["a".to_string()]).expect("取根");
    assert!(
        roots.shared.is_absolute() && roots.agents.contains_key("a"),
        "根一律绝对路径"
    );
    // 列清单是查询不是创建：会话不存在 = 空清单，不报错也不造目录。
    let none = ws.list("没这个会话", &[]).expect("列不存在的会话不报错");
    assert!(
        none.work.is_empty() && !sessions.join("没这个会话").exists(),
        "查询不该有副作用"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- FsSysIo ----------

#[test]
fn fs_sys_io_reads_utf8_flags_lossy_and_truncation_and_creates_parents() {
    let root = scratch("fs-sysio");
    let f = root.join("deep").join("note.txt");
    let io = FsSysIo::default();
    io.write(&f, "你好，世界").expect("写要自动建父目录");
    let r = io.read(&f).expect("读回");
    assert_eq!(r.text, "你好，世界");
    assert_eq!(r.bytes, "你好，世界".len(), "bytes 是文件原始字节数");
    assert!(!r.lossy && !r.cut);

    let bad = root.join("bad.bin");
    std::fs::write(&bad, [0x41, 0xff, 0xfe, 0x42]).expect("造非法 UTF-8");
    let r = io.read(&bad).expect("读回");
    assert!(r.lossy, "含非法字节必须如实标注 lossy（本程序不猜编码）");
    assert!(r.text.starts_with('A'), "{}", r.text);

    let small = FsSysIo { max_read_bytes: 3 };
    let cut = small.read(&f).expect("截断读取成功");
    assert!(cut.cut, "超出上限要如实标注 cut");
    assert_eq!(cut.bytes, "你好，世界".len(), "bytes 仍是整个文件的字节数");
    assert!(cut.text.chars().count() < 6, "只读了开头：{}", cut.text);

    assert!(
        io.read(&root.join("nope.txt"))
            .err()
            .expect("应当失败")
            .contains("读取失败"),
        "缺文件要如实报错"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- FsHistory ----------

#[test]
fn fs_history_roundtrips_lists_deletes_and_rejects_broken_meta() {
    let root = scratch("fs-history");
    let h = FsHistory::new(root.clone());
    // 目录还不存在：列会话 = 空清单，不是错误。
    assert!(h.list().expect("列会话").is_empty());

    h.create(&meta("w1")).expect("创建成功");
    assert!(
        root.join("w1").join("meta.yaml").is_file(),
        "meta.yaml 落盘"
    );
    h.append("w1", &[serde_json::json!({"type": "say", "text": "hi"})])
        .expect("追加成功");
    h.append("w1", &[serde_json::json!({"type": "ended"})])
        .expect("追加成功");
    let (m, events) = h.load("w1").expect("读回");
    assert_eq!(m.name, "w1");
    assert_eq!(events.len(), 2, "jsonl 一行一条，只追加不丢");
    let listed = h.list().expect("列会话");
    assert_eq!(listed.len(), 1);
    assert!(listed[0].done, "ended 之后 done = true");

    let mut renamed = meta("w1");
    renamed.mode = "collab".to_string();
    h.save_meta(&renamed).expect("写回元信息");
    assert_eq!(
        h.load("w1").expect("读回").0.mode,
        "collab",
        "meta.yaml 是会话身份的唯一真相"
    );

    std::fs::create_dir_all(root.join("w2")).expect("造一个坏会话");
    std::fs::write(root.join("w2").join("meta.yaml"), "mode: [不是字符串").expect("造坏 meta");
    assert!(
        h.load("w2").unwrap_err().contains("非法"),
        "坏 meta 要如实报错，不静默跳过"
    );
    assert_eq!(h.list().expect("列会话").len(), 1, "坏会话不进清单");

    assert!(h.delete("w1").expect("删除成功"), "删除已存在的会话 = true");
    assert!(!h.delete("w1").expect("再删"), "重复删除 = false，不是错误");
    assert!(h.load("w1").unwrap_err().contains("无此会话"));
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- YamlSettingsStore ----------

#[test]
fn yaml_settings_store_defaults_saves_and_reports_malformed_files() {
    let root = scratch("yaml-settings");
    let home = root.join(".home");
    let paths = || {
        (
            home.join("providers.yaml"),
            home.join("models.yaml"),
            home.join("settings.yaml"),
            home.join("agents.yaml"),
        )
    };
    let (p, m, s, a) = paths();
    let store = YamlSettingsStore::new(p, m, s, a);

    // 四份文件都不存在 = 空登记处 + 默认设置（不是错误）。
    let empty = store.load().expect("缺文件必须取默认，不报错");
    assert!(empty.providers.is_empty() && empty.models.is_empty() && empty.core.is_none());
    assert!(empty.app.streaming && !empty.app.fence_write);
    assert!(empty.agents.is_empty());

    let mut want = Settings::default();
    want.providers.insert(
        "p".to_string(),
        Provider {
            base_url: "http://x".to_string(),
            api_key: "sk-secret".to_string(),
        },
    );
    want.models.insert(
        "m".to_string(),
        crate::core::providers::ModelEntry {
            name: "M".to_string(),
            api_model: "m".to_string(),
            provider: "p".to_string(),
            note: String::new(),
            tools: crate::core::providers::ToolMode::Envelope,
        },
    );
    want.core = Some("m".to_string());
    want.app.streaming = false;
    want.agents.insert(
        "甲".to_string(),
        crate::core::agents::Agent {
            modules: vec!["a".to_string()],
            model: None,
            note: "n".to_string(),
        },
    );
    store.save(&want).expect("保存成功");
    let got = store.load().expect("读回");
    assert_eq!(
        got.providers.get("p").map(|p| p.base_url.as_str()),
        Some("http://x")
    );
    assert_eq!(
        got.providers.get("p").map(|p| p.api_key.as_str()),
        Some("sk-secret"),
        "密钥落 providers.yaml"
    );
    assert_eq!(got.core.as_deref(), Some("m"));
    assert!(!got.app.streaming && got.agents.contains_key("甲"));

    // unix：密钥文件 0600。
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(home.join("providers.yaml"))
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "密钥文件必须是 0600");
    }

    // 非法 yaml：哪一份坏了要说清是哪一份（不静默兜底）。
    std::fs::write(home.join("models.yaml"), "models: [不是映射").expect("造坏文件");
    let err = store.load().unwrap_err();
    assert!(err.contains("models.yaml 非法"), "{}", err);

    // 工具调用形态是**封闭枚举**：envelope / native 之外的写法一律拒收（不猜、不静默降级）
    std::fs::write(
        home.join("models.yaml"),
        "models:\n  m:\n    name: M\n    api_model: m\n    provider: p\n    tools: auto\n",
    )
    .expect("造非法形态");
    let err = store.load().unwrap_err();
    assert!(
        err.contains("models.yaml 非法"),
        "非法形态要拒收并说明是哪个文件：{}",
        err
    );

    // 合法形态能落盘也能读回（envelope 是缺省，native 要显式写）
    std::fs::write(
        home.join("models.yaml"),
        "models:\n  m:\n    name: M\n    api_model: m\n    provider: p\n    tools: native\n",
    )
    .expect("造合法形态");
    let back = store.load().expect("合法形态必须能读回");
    assert_eq!(
        back.models.get("m").map(|m| m.tools),
        Some(crate::core::providers::ToolMode::Native),
        "注册表里的形态要原样读回"
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- FsModules ----------

#[test]
fn fs_modules_accepts_valid_folders_and_rejects_each_illegal_form_with_a_reason() {
    let root = scratch("fs-modules");
    let dir = root.join("modules");
    let put = |folder: &str, yaml: &str| {
        let d = dir.join(folder);
        std::fs::create_dir_all(&d).expect("建模块目录");
        std::fs::write(d.join("module.yaml"), yaml).expect("写清单");
    };
    put("good", "id: good\nbrief: 简介\nsystem: 你是 good\n");
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

    let roster = FsModules::new(dir).scan();
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
    let gone = FsModules::new(root.join("没有这个目录")).scan();
    assert!(gone.modules.is_empty() && gone.rejected.is_empty());
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- FsPackages ----------

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

// ---------- YamlPrompts ----------

#[test]
fn yaml_prompts_loads_the_shipped_book_and_reports_missing_or_broken_files() {
    let root = scratch("yaml-prompts");
    let real = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("prompts.yaml");
    let ok = YamlPrompts::new(real)
        .load()
        .expect("产品自带提示词册必须合法");
    assert!(!ok.core.no_agents.is_empty());

    let missing = YamlPrompts::new(root.join("nope.yaml")).load().unwrap_err();
    assert!(missing.contains("提示词册缺失"), "{}", missing);
    let broken = root.join("broken.yaml");
    std::fs::write(&broken, "core: [不是映射").expect("造坏册子");
    assert!(YamlPrompts::new(broken)
        .load()
        .unwrap_err()
        .contains("prompts.yaml 非法"));
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- FileLog ----------

#[test]
fn file_log_writes_every_level_into_a_timestamped_file_under_logs() {
    let root = scratch("file-log");
    {
        let log = FileLog::new(&root, "契约测试").expect("建日志文件成功");
        log.info("模块", "正常信息");
        log.warn("模块", "警示信息");
        log.error("模块", "错误信息");
    }
    let dir = root.join("logs");
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .expect("日志目录要建出来")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 1, "一次运行一个文件：{:?}", names);
    assert!(names[0].ends_with(".log"));
    let text = std::fs::read_to_string(dir.join(&names[0])).expect("读日志");
    assert!(
        text.contains("契约测试")
            && text.contains("正常信息")
            && text.contains("警示信息")
            && text.contains("错误信息")
    );
    let _ = std::fs::remove_dir_all(&root);
}

// ---------- 本机环回假供应商（HTTP 适配器的真实边界） ----------

/// 本机假供应商：按脚本逐条应答，跑完/丢弃时停线程。只绑 127.0.0.1。
struct Mock {
    base: String,
    stop: Arc<AtomicBool>,
    hits: Arc<Mutex<Vec<String>>>,
    /// 收到的请求体（与 hits 同序）：探针类用例要断言"到底发出去了什么形状"。
    bodies: Arc<Mutex<Vec<String>>>,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl Mock {
    fn start(replies: Vec<(u16, &'static str, String)>) -> Mock {
        let server = tiny_http::Server::http("127.0.0.1:0").expect("起本机假供应商");
        let addr = server.server_addr().to_ip().expect("TCP 地址");
        let base = format!("http://127.0.0.1:{}", addr.port());
        let stop = Arc::new(AtomicBool::new(false));
        let hits = Arc::new(Mutex::new(Vec::new()));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (st, hi, bo) = (Arc::clone(&stop), Arc::clone(&hits), Arc::clone(&bodies));
        let handle = std::thread::spawn(move || {
            let mut served = 0usize;
            while !st.load(Ordering::Relaxed) {
                let mut req = match server.recv_timeout(Duration::from_millis(50)) {
                    Ok(Some(r)) => r,
                    _ => continue,
                };
                hi.lock()
                    .expect("锁")
                    .push(format!("{} {}", req.method(), req.url()));
                let mut body_text = String::new();
                let _ = req.as_reader().read_to_string(&mut body_text);
                bo.lock().expect("锁").push(body_text);
                let (code, ctype, body) = match replies.get(served) {
                    Some(r) => (r.0, r.1, r.2.clone()),
                    None => (200, "application/json", "{}".to_string()),
                };
                served += 1;
                let header = tiny_http::Header::from_bytes(&b"Content-Type"[..], ctype.as_bytes())
                    .expect("响应头");
                let _ = req.respond(
                    tiny_http::Response::from_string(body)
                        .with_status_code(code)
                        .with_header(header),
                );
            }
        });
        Mock {
            base,
            stop,
            hits,
            bodies,
            handle: Some(handle),
        }
    }

    fn channel(&self, key: &str) -> Channel {
        Channel {
            provider: Provider {
                base_url: self.base.clone(),
                api_key: key.to_string(),
            },
            model: "m".to_string(),
        }
    }

    fn requests(&self) -> Vec<String> {
        self.hits.lock().expect("锁").clone()
    }

    fn bodies(&self) -> Vec<String> {
        self.bodies.lock().expect("锁").clone()
    }
}

impl Drop for Mock {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

fn completion_body(content: &str) -> String {
    serde_json::json!({"choices": [{"message": {"content": content}}]}).to_string()
}

fn sse_delta(content: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({"choices": [{"delta": {"content": content}}]})
    )
}

/// 收尾分片：只带 finish_reason（供应商通常这样收尾）。
fn sse_finish(reason: &str) -> String {
    format!(
        "data: {}\n\n",
        serde_json::json!({"choices": [{"delta": {}, "finish_reason": reason}]})
    )
}

fn gateway() -> HttpGateway {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    HttpGateway::with_log(log, memo_new())
}

#[test]
fn http_gateway_talks_to_a_real_endpoint_on_loopback() {
    let mock = Mock::start(vec![(
        200,
        "application/json",
        completion_body("来自本机假供应商"),
    )]);
    let (mut chat, notice) = gateway().member_channel(Some(&mock.channel("k")), "a");
    assert!(
        notice.is_none(),
        "有通道时不得回落演示，也不得编造回落通知：{:?}",
        notice
    );
    let out = chat
        .complete(
            &[Msg::user("你好")],
            CompleteOpts::plain(false),
            &mut |_| true,
        )
        .raw;
    assert_eq!(out, "来自本机假供应商");
    let hits = mock.requests();
    assert_eq!(hits.len(), 1, "端点一次命中就不再探测候选：{:?}", hits);
    assert!(hits[0].contains("/chat/completions"), "{:?}", hits);
}

#[test]
fn http_chat_streams_chunks_and_stops_when_the_caller_aborts() {
    let body = format!("{}{}data: [DONE]\n\n", sse_delta("你"), sse_delta("好"));
    let mock = Mock::start(vec![(200, "text/event-stream", body)]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel("k")), "a");
    let mut seen: Vec<String> = Vec::new();
    let out = chat
        .complete(
            &[Msg::user("讲两句")],
            CompleteOpts::plain(true),
            &mut |c| {
                seen.push(match c {
                    Chunk::Start => "start".to_string(),
                    Chunk::Text(t) => format!("text:{}", t),
                    Chunk::Reasoning(r) => format!("reasoning:{}", r),
                });
                true
            },
        )
        .raw;
    assert_eq!(out, "你好", "流式返回完整正文");
    assert_eq!(
        seen,
        vec!["start", "text:你", "text:好"],
        "先 Start，再逐片 Text"
    );

    // 中止：Start 放行、第一片 Text 后要求停止 → 立即停下并保留已产出的正文。
    let body2 = format!("{}{}data: [DONE]\n\n", sse_delta("你"), sse_delta("好"));
    let mock2 = Mock::start(vec![(200, "text/event-stream", body2)]);
    let (mut chat2, _) = gateway().member_channel(Some(&mock2.channel("k")), "a");
    let mut seen2: Vec<String> = Vec::new();
    let out2 = chat2
        .complete(&[Msg::user("停")], CompleteOpts::plain(true), &mut |c| {
            let is_text = matches!(c, Chunk::Text(_));
            seen2.push(format!("{:?}", c));
            !is_text
        })
        .raw;
    assert_eq!(out2, "你", "中止后返回已产出的正文，且不重开候选");
    assert_eq!(seen2.len(), 2, "中止后不再回调：{:?}", seen2);
    assert_eq!(mock2.requests().len(), 1, "中止不得把流重新拉起来");
}

#[test]
fn finish_reason_is_carried_back_from_both_paths() {
    // 非流式：结束原因在 choices[0].finish_reason
    let body = serde_json::json!({
        "choices": [{"message": {"content": "写完"}, "finish_reason": "stop"}]
    })
    .to_string();
    let mock = Mock::start(vec![(200, "application/json", body)]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel("k")), "a");
    let out = chat.complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
        true
    });
    assert_eq!(out.raw, "写完");
    assert_eq!(out.finish, "stop", "结束原因要如实带回");
    assert!(!out.truncated(), "stop 不是截断");

    // 流式：收尾分片带 finish_reason = length（核心据此判定"被截断"，而不是"模型写错"）
    let sse = format!(
        "{}{}data: [DONE]\n\n",
        sse_delta("半句"),
        sse_finish("length")
    );
    let mock2 = Mock::start(vec![(200, "text/event-stream", sse)]);
    let (mut chat2, _) = gateway().member_channel(Some(&mock2.channel("k")), "a");
    let out2 = chat2.complete(&[Msg::user("hi")], CompleteOpts::plain(true), &mut |_| true);
    assert_eq!(out2.raw, "半句", "已产出的正文照常返回");
    assert_eq!(out2.finish, "length");
    assert!(out2.truncated(), "各家的截断取值都归到一处判定");
}

/// 一次原生工具调用的非流式响应（content 为 null：纯工具调用轮的常见形状）。
fn tool_call_body(name: &str, args: &str) -> String {
    serde_json::json!({
        "choices": [{
            "message": {
                "content": null,
                "tool_calls": [{ "id": "call_1", "type": "function", "function": { "name": name, "arguments": args } }]
            },
            "finish_reason": "tool_calls"
        }]
    })
    .to_string()
}

/// 流式里的一段工具调用分片（按 index 归位；id/name 一般只在第一片，arguments 逐片拼）。
fn sse_tool(index: u64, id: Option<&str>, name: Option<&str>, args: Option<&str>) -> String {
    let mut f = serde_json::json!({ "index": index });
    if let Some(id) = id {
        f["id"] = serde_json::json!(id);
    }
    let mut func = serde_json::Map::new();
    if let Some(n) = name {
        func.insert("name".to_string(), serde_json::json!(n));
    }
    if let Some(a) = args {
        func.insert("arguments".to_string(), serde_json::json!(a));
    }
    f["function"] = serde_json::Value::Object(func);
    format!(
        "data: {}\n\n",
        serde_json::json!({ "choices": [{ "delta": { "tool_calls": [f] } }] })
    )
}

#[test]
fn http_chat_reads_native_tool_calls() {
    // content 是 null（不是缺字段）：按空串处理，并把 tool_calls 原样带回来
    let mock = Mock::start(vec![(
        200,
        "application/json",
        tool_call_body("read", "{\"path\":\"/x\"}"),
    )]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel("k")), "a");
    let out = chat.complete(
        &[Msg::user("读一下")],
        CompleteOpts::plain(false),
        &mut |_| true,
    );
    assert_eq!(out.raw, "", "纯工具调用轮没有正文");
    assert_eq!(out.finish, "tool_calls");
    assert_eq!(
        out.calls,
        vec![crate::core::ports::ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            args_json: "{\"path\":\"/x\"}".to_string(),
        }]
    );
}

#[test]
fn http_chat_assembles_streaming_tool_calls_by_index() {
    let sse = format!(
        "{}{}{}{}{}data: [DONE]\n\n",
        sse_tool(0, Some("call_a"), Some("read"), Some("{\"pa")),
        sse_tool(0, None, None, Some("th\":\"/x\"}")),
        sse_tool(1, Some("call_b"), Some("search"), Some("{}")),
        sse_finish("tool_calls"),
        "",
    );
    let mock = Mock::start(vec![(200, "text/event-stream", sse)]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel("k")), "a");
    let out = chat.complete(
        &[Msg::user("两件事")],
        CompleteOpts::plain(true),
        &mut |_| true,
    );
    assert_eq!(out.finish, "tool_calls");
    assert_eq!(
        out.raw, "",
        "纯工具调用轮没有正文，也不能因此判成「没内容」去换候选"
    );
    assert_eq!(
        out.calls.len(),
        2,
        "两次调用按 index 各就各位：{:?}",
        out.calls
    );
    assert_eq!(out.calls[0].id, "call_a");
    assert_eq!(out.calls[0].name, "read");
    assert_eq!(
        out.calls[0].args_json, "{\"path\":\"/x\"}",
        "分片的 arguments 要按序拼起来"
    );
    assert_eq!(out.calls[1].name, "search");
    assert_eq!(out.calls[1].args_json, "{}");
}

/// 历史里带原生调用时，请求体必须发成协议形状：assistant(正文 + tool_calls) + 每条结果一条 role=tool。
/// 这就是回放（重启/回档重建）要走的那套形状，所以它必须真的发得出去，不能只在内存里对。
#[test]
fn native_history_goes_out_in_protocol_shape() {
    let mock = Mock::start(vec![(200, "application/json", completion_body("ok"))]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel("k")), "a");
    let history = [
        Msg::user("读一下"),
        Msg::assistant_calls(
            "我看看这个文件。",
            vec![crate::core::ports::ToolCall {
                id: "call_1".to_string(),
                name: "read".to_string(),
                args_json: "{\"path\":\"/x\"}".to_string(),
            }],
        ),
        Msg::tool("call_1", "[工具结果] read\n第一行"),
    ];
    let _ = chat.complete(&history, CompleteOpts::plain(false), &mut |_| true);
    let body = &mock.bodies()[0];
    assert!(
        body.contains("\"tool_calls\""),
        "助手消息要带 tool_calls：{}",
        body
    );
    assert!(
        body.contains("\"id\":\"call_1\"") && body.contains("\"name\":\"read\""),
        "调用的 id 与名字原样带上：{}",
        body
    );
    assert!(
        body.contains("\"arguments\""),
        "参数走结构化槽位（arguments）而不是正文：{}",
        body
    );
    assert!(
        body.contains("\"content\":\"我看看这个文件。\""),
        "助手消息的正文要真的发出去：{}",
        body
    );
    assert!(
        body.contains("\"role\":\"tool\"") && body.contains("\"tool_call_id\":\"call_1\""),
        "结果消息用 role=tool + tool_call_id 回应它：{}",
        body
    );
}

#[test]
fn tool_probe_tells_supported_unsupported_and_unknown_apart() {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    let gateway = || HttpGateway::with_log(Arc::clone(&log), memo_new());

    // ① 支持：不带 tools 能通，带 tools 返回工具调用
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (
            200,
            "application/json",
            tool_call_body("solomni_ping", "{}"),
        ),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Supported { detail }) => {
            assert!(detail.contains("solomni_ping"), "{}", detail)
        }
        other => panic!("应判为支持：{:?}", other),
    }

    // ② 明确不支持：同样的请求，带上 tools 被供应商拒（附供应商原话）
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (
            400,
            "application/json",
            serde_json::json!({ "error": { "message": "Unknown parameter: tools" } }).to_string(),
        ),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Unsupported { detail }) => assert!(detail.contains("tools"), "{}", detail),
        other => panic!("应判为明确不支持：{:?}", other),
    }

    // ③ 无法判定：带 tools 也通，但这次没发起调用——如实说"没法定论"，不替用户拍板
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("好")),
        (200, "application/json", completion_body("我不调用工具")),
    ]);
    match gateway().probe_tools(&mock.channel("k")) {
        Ok(ProbeOutcome::Unknown { detail }) => {
            assert!(detail.contains("没有发起调用"), "{}", detail)
        }
        other => panic!("应判为无法判定：{:?}", other),
    }

    // ④ 通道本身就不通：如实报"测不了"，绝不把 401 之类的失败当成"不支持工具调用"
    let mock = Mock::start(vec![(
        401,
        "application/json",
        "{\"error\":{\"message\":\"bad key\"}}".to_string(),
    )]);
    let err = gateway()
        .probe_tools(&mock.channel("bad"))
        .expect_err("通道不通就是 Err");
    assert!(err.contains("通道本身就没打通"), "{}", err);
}

/// 回放形状探测：逐项如实回报哪种写法被接受（不合并、不替用户拍板），
/// 且发出去的确实是那个形状；基线不通时如实报"测不了"，绝不把 401 说成"这个形状被拒"。
#[test]
fn replay_shape_probe_reports_which_writings_the_supplier_accepts() {
    let log: Arc<dyn Log + Send + Sync> = Arc::new(NoopLog);
    let rejected = serde_json::json!({
        "error": { "message": "content is required when tool_calls is present" }
    })
    .to_string();
    // 编号只放在工具结果里：回答里带回它才算"真的读到了"（收了 != 看懂了）。
    // 而且模型常常把前缀省掉（真机实测就只回后半段）——那不算没读懂，判据要容得下。
    let nonce = "solomni-7f3a91c2";
    let mock = Mock::start(vec![
        (200, "application/json", completion_body("7f3a91c2")),
        (
            200,
            "application/json",
            completion_body("那个编号是 7f3a91c2"),
        ),
        (400, "application/json", rejected),
        (200, "application/json", completion_body("我看不到任何编号")),
        (200, "application/json", completion_body("solomni-7f3a91c2")),
    ]);
    let report = crate::adapters::http_probe::probe_replay_with(&mock.channel("k"), &log, nonce)
        .expect("基线通过就该拿到报告");
    let got: Vec<(&str, bool, bool)> = report
        .shapes
        .iter()
        .map(|s| (s.name.as_str(), s.accepted, s.understood))
        .collect();
    assert_eq!(
        got,
        vec![
            ("baseline-text", true, true),
            ("content-empty", true, true),
            ("content-null", false, false),
            ("content-prose", true, false),
            ("tool-with-name", true, true),
        ],
        "每个形状的接受 / 被拒 / 有没有真被读懂都要逐项报出来（省掉前缀也算读懂）"
    );
    let no = report
        .shapes
        .iter()
        .find(|s| !s.accepted)
        .expect("有一条被拒");
    assert!(
        no.detail.contains("content is required"),
        "被拒要带供应商原话：{}",
        no.detail
    );
    // 发出去的确实是那个形状：助手回合带 tool_calls，结果消息用同一个 tool_call_id 对应。
    let bodies = mock.bodies();
    assert_eq!(bodies.len(), 5, "每个形状各发一次");
    assert!(
        !bodies[0].contains("tool_calls"),
        "基线是文本回放，不带协议字段：{}",
        bodies[0]
    );
    assert!(
        bodies[0].contains("\"tools\""),
        "每个形状都要带工具声明：{}",
        bodies[0]
    );
    assert!(
        bodies[1].contains("\"tool_calls\"")
            && bodies[1].contains("\"tool_call_id\":\"call_solomni_probe\""),
        "协议形状要真的发出去：{}",
        bodies[1]
    );
    assert!(
        bodies[1].contains(nonce),
        "本次编号要真的在工具结果里：{}",
        bodies[1]
    );

    let bad = Mock::start(vec![(
        401,
        "application/json",
        "{\"error\":{\"message\":\"bad key\"}}".to_string(),
    )]);
    let err = crate::adapters::http_probe::probe_replay(&bad.channel("bad"), &log)
        .expect_err("通道不通就是 Err");
    assert!(err.contains("通道本身就没打通"), "{}", err);
}

#[test]
fn http_errors_come_back_honestly_and_never_carry_the_api_key() {
    let secret = "sk-super-secret-xyz";
    let body =
        serde_json::json!({"error": {"message": format!("invalid key {}", secret)}}).to_string();
    let mock = Mock::start(vec![
        (401, "application/json", body),
        (401, "application/json", "{}".to_string()),
    ]);
    let (mut chat, _) = gateway().member_channel(Some(&mock.channel(secret)), "a");
    let out = chat
        .complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
            true
        })
        .raw;
    assert!(out.contains("模型调用失败"), "失败必须如实回执：{}", out);
    assert!(!out.contains(secret), "出站错误里的密钥必须先脱敏：{}", out);
    assert!(
        out.contains("***") || out.contains("401"),
        "要留下可定位的事实：{}",
        out
    );
}

#[test]
fn http_gateway_falls_back_to_demo_only_when_there_is_no_channel() {
    let (mut chat, notice) = gateway().member_channel(None, "reviewer");
    assert!(notice.is_some(), "没有通道时必须如实告知回落：{:?}", notice);
    let out = chat
        .complete(&[Msg::user("hi")], CompleteOpts::plain(false), &mut |_| {
            true
        })
        .raw;
    assert!(out.contains("（演示）"), "{}", out);
    let (_, core_demo) = gateway().core_channel(None);
    assert!(core_demo, "核心通道同样如实标记");
}

#[test]
fn http_model_catalog_lists_models_and_rejects_broken_shapes() {
    let ok = Mock::start(vec![(
        200,
        "application/json",
        serde_json::json!({"data": [{"id": "m-a"}, {"id": "m-b"}]}).to_string(),
    )]);
    let p = ok.channel("k").provider;
    let got = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&p)
        .expect("发现成功");
    assert_eq!(got, vec!["m-a", "m-b"]);
    assert!(ok.requests()[0].contains("/models"), "{:?}", ok.requests());

    // 2xx 但不是模型清单形状：不猜测兜底，如实报错（并试过回落候选）。
    let bad = Mock::start(vec![
        (200, "application/json", "{\"foo\": 1}".to_string()),
        (200, "application/json", "{\"foo\": 1}".to_string()),
    ]);
    let p2 = bad.channel("k").provider;
    let err = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&p2)
        .unwrap_err();
    assert!(!err.is_empty(), "形状不符必须报错：{}", err);

    // 4xx：如实报错，不静默给空清单。
    let boom = Mock::start(vec![
        (400, "application/json", "no".to_string()),
        (400, "application/json", "no".to_string()),
    ]);
    let p3 = boom.channel("k").provider;
    let err2 = HttpModelCatalog::with_log(Arc::new(NoopLog), memo_new())
        .list_models(&p3)
        .unwrap_err();
    assert!(err2.contains("400"), "{}", err2);
}
/// 出站错误分类的判据：**网络类**算环境（env-skip），**其余 TLS 错误**算我们的问题（必须失败），
/// 只有「本进程取不到系统 TLS 凭证」那一码才算环境结论（env-tls）。
/// 现场：DSH 沙箱挡住工作区外的用户凭证存储时，schannel 报 SEC_E_NO_CREDENTIALS——
/// 连 Windows 自带的 curl.exe 都握不了手，放开沙箱后同一条链路立刻通。
/// 这条测试钉的就是「绝不把 TLS 坏了说成环境不允许」这条红线，同时不把环境结论误报成失败。
#[test]
fn outbound_error_classification_keeps_environment_and_our_bug_apart() {
    use crate::adapters::http_agent::classify;
    // 网络类：环境结论。
    assert_eq!(classify(&ureq::Error::HostNotFound), "no-net");
    assert_eq!(classify(&ureq::Error::ConnectionFailed), "no-net");
    // 取不到系统凭证（十进制与十六进制两种原文都要认，且大小写无关）。
    let dec = ureq::Error::Tls("安全包中没有可用的凭证 (os error -2146893042)");
    assert_eq!(classify(&dec), "env-tls", "判据只认稳定错误码，不认本地化文案");
    let hex = ureq::Error::Tls("schannel: AcquireCredentialsHandle failed: SEC_E_NO_CREDENTIALS (0x8009030e)");
    assert_eq!(classify(&hex), "env-tls", "十六进制写法也要认");
    // 其余 TLS 错误：我们链路的问题，必须失败——不许借「环境」二字溜过去。
    let other = ureq::Error::Tls("certificate verify failed");
    assert_eq!(classify(&other), "tls-fail");
    let bad_host = ureq::Error::Tls("hostname mismatch");
    assert_eq!(classify(&bad_host), "tls-fail");

    // 判据本身：只认那一码的两种写法，别的原文一律不算（"环境"不能是个筐）。
    // Windows 上 native-tls 的错误走 NativeTls 变体（不是通用 Tls 壳），那个变体构造不出来，
    // 所以由真机探针（--https-check → env-tls）验接线，这里把判据本身钉死。
    use crate::adapters::http_agent::lacks_system_credentials;
    assert!(lacks_system_credentials("(os error -2146893042)"), "有符号十进制要认");
    assert!(lacks_system_credentials("(0x8009030e)"), "十六进制要认");
    assert!(lacks_system_credentials("(0x8009030E)"), "大小写无关");
    assert!(!lacks_system_credentials("certificate verify failed"));
    assert!(!lacks_system_credentials("hostname mismatch"));
    assert!(!lacks_system_credentials(""), "空原文不能算环境结论");
}
