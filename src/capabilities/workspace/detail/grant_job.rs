//! 目的：外部工作区绑定的一次性机器级只读授权——会话建好之后在后台付，不挡住创建。
//! 管：后台线程的按根去重、进度行、状态文件 .home/workspace-grants.json 与成本样本回写。
//! 不管：ACL 怎么读写（内核 confine）、绑定路径合法性（workspace_root）、成本簿格式（estimate）。
//! 联动：conductor 的提交段调 ensure_in_background；--fence-revoke 经内核撤这份授权。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

/// 目的：本进程里正在付这份账的根——多个会话绑同一个工作区时只写一次 ACL。
fn inflight() -> &'static Mutex<HashSet<PathBuf>> {
    static CELL: OnceLock<Mutex<HashSet<PathBuf>>> = OnceLock::new();
    CELL.get_or_init(|| Mutex::new(HashSet::new()))
}

/// 目的：把一次机器级只读授权甩到后台线程去付；会话创建因此仍是毫秒级。
/// 约束：同一个规范化根在付的过程中不重复起线程；失败只打一行并记进状态文件，不阻断会话。
pub fn ensure_in_background(home: &Path, root: &Path) {
    let root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let fresh = inflight()
        .lock()
        .map(|mut set| set.insert(root.clone()))
        .unwrap_or(false);
    if !fresh {
        return;
    }
    let home = home.to_path_buf();
    let target = root.clone();
    let spawned = std::thread::Builder::new()
        .name("workspace-grant".to_string())
        .spawn(move || {
            eprintln!("[围栏] 外部工作区一次性授权开始：{}", target.display());
            match crate::kernel::api::ensure_workspace_grant(&home, &target) {
                Ok(r) => {
                    eprintln!(
                        "[围栏] 外部工作区授权完成：新写 {} 处、复用 {} 处、{} ms",
                        r.written, r.reused, r.ms
                    );
                    let objects = refresh_cost_sample(&home, &target, r.ms, r.written > 0);
                    write_state(&home, &target, Some((objects, r.ms)), None);
                }
                Err(e) => {
                    eprintln!("[围栏] 外部工作区授权失败（不阻断会话）：{}", e);
                    write_state(&home, &target, None, Some(e));
                }
            }
            if let Ok(mut set) = inflight().lock() {
                set.remove(&target);
            }
        });
    if let Err(e) = spawned {
        eprintln!("[围栏] 外部工作区授权线程起不来（不阻断会话）：{}", e);
        if let Ok(mut set) = inflight().lock() {
            set.remove(&root);
        }
    }
}

/// 目的：真实授权成功后把这次花了多少记进成本簿，下一次预估就用本卷实测速率。
/// 返回：这次数到的对象数（授权成本乘数）；0 = 没数成（路径不可达、UNC 或读不动）。
/// 约束：只读计数也有预算；数不完（truncated）或没有对象时宁可继续用旧样本，不写错数字；
/// 只有真写了 ACL（wrote）才记样本，复用时的 0 ms 不能把速率带偏。
fn refresh_cost_sample(home: &Path, root: &Path, ms: u64, wrote: bool) -> u64 {
    let Ok(cost) = crate::capabilities::workspace::api::estimate_workspace_cost(root, home) else {
        return 0;
    };
    if cost.objects == 0 {
        return 0;
    }
    if wrote && !cost.truncated {
        crate::capabilities::workspace::api::record_grant_cost(home, root, cost.objects, ms);
    }
    cost.objects
}

/// 目的：写一个根的授权状态：成功记这次的对象数与耗时，失败只补 last_error（保留上次成功的数字）。
/// 约束：objects 是授权时数到的真对象数（0 = 没数成），不是授权步数；读改写整份文件在进程内串行。
fn write_state(home: &Path, root: &Path, ok: Option<(u64, u64)>, error: Option<String>) {
    static LOCK: Mutex<()> = Mutex::new(());
    let _guard = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let file = home.join("workspace-grants.json");
    let mut book = std::fs::read_to_string(&file)
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .unwrap_or_else(|| serde_json::json!({ "roots": {} }));
    if !book.get("roots").is_some_and(|v| v.is_object()) {
        book["roots"] = serde_json::json!({});
    }
    let key = root.display().to_string();
    let prev = book["roots"]
        .get(key.as_str())
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let num = |field: &str| prev.get(field).and_then(|v| v.as_u64()).unwrap_or(0);
    let (objects, ms, granted_at) = match ok {
        Some((objects, ms)) => (objects, ms, now_unix()),
        None => (num("objects"), num("ms"), num("granted_at")),
    };
    book["roots"][key.as_str()] = serde_json::json!({
        "mode": crate::capabilities::workspace::api::WorkspaceGrant::Workspace.as_str(),
        "granted_at": granted_at,
        "objects": objects,
        "ms": ms,
        "last_error": error,
    });
    let Ok(text) = serde_json::to_string_pretty(&book) else {
        return;
    };
    let _ = std::fs::create_dir_all(home);
    let tmp = file.with_extension("json.tmp");
    if std::fs::write(&tmp, text).is_ok() {
        let _ = std::fs::rename(&tmp, &file);
    }
}

/// 目的：granted_at 用的 unix 秒。
fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "solomni-grant-job-{}-{}-{}",
            tag,
            std::process::id(),
            now_unix()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("建临时目录");
        dir
    }

    #[test]
    fn state_file_keeps_the_last_success_while_failures_add_up() {
        let home = scratch("state");
        let root = home.join("space");
        std::fs::create_dir_all(&root).expect("建根");
        write_state(&home, &root, Some((7, 120)), None);
        let text = std::fs::read_to_string(home.join("workspace-grants.json")).expect("读状态");
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        let key = root.display().to_string();
        assert_eq!(v["roots"][key.as_str()]["objects"], serde_json::json!(7));
        assert_eq!(v["roots"][key.as_str()]["ms"], serde_json::json!(120));
        assert!(v["roots"][key.as_str()]["last_error"].is_null());
        write_state(&home, &root, None, Some("授不上".to_string()));
        let text = std::fs::read_to_string(home.join("workspace-grants.json")).expect("读状态");
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        assert_eq!(v["roots"][key.as_str()]["objects"], serde_json::json!(7));
        assert_eq!(
            v["roots"][key.as_str()]["last_error"],
            serde_json::json!("授不上")
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[cfg(windows)]
    #[test]
    fn cost_sample_is_written_for_a_canonicalized_root() {
        let home = scratch("cost");
        let root = home.join("space");
        std::fs::create_dir_all(&root).expect("建根");
        std::fs::write(root.join("a.txt"), b"a").expect("写文件");
        let canonical = root.canonicalize().expect("规范化根");
        let objects = refresh_cost_sample(&home, &canonical, 12, true);
        assert_eq!(objects, 1, "1 个文件、0 个子目录");
        let costs = home.join("fence-costs.json");
        assert!(costs.is_file(), "扩展前缀不能挡掉样本：{}", costs.display());
        refresh_cost_sample(&home, &canonical, 0, false);
        let book: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&costs).expect("读成本")).expect("JSON");
        // 卷名取决于系统临时目录落在哪个盘（门禁会把 TEMP 固定到 target/tmp），不能写死 "C:"。
        let volumes = book["volumes"].as_object().expect("卷表");
        assert_eq!(volumes.len(), 1, "只记样本所在的那一卷");
        let samples = volumes
            .values()
            .next()
            .and_then(|c| c["samples"].as_array())
            .expect("样本数组");
        assert_eq!(samples.len(), 1, "复用（没写 ACL）不该记样本：{book}");
        write_state(&home, &canonical, Some((objects, 12)), None);
        let text = std::fs::read_to_string(home.join("workspace-grants.json")).expect("读状态");
        let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
        let key = canonical.display().to_string();
        assert_eq!(v["roots"][key.as_str()]["objects"], serde_json::json!(1));
        let _ = std::fs::remove_dir_all(&home);
    }

    /// 目的：后台授权窗口的整条路径——原地起线程、跑完写状态、按根去重后把根从"正在付"里摘掉。
    /// 约束：只在非 Windows 跑：内核桩返回零报告，不碰真实 ACL。
    #[cfg(not(windows))]
    #[test]
    fn background_grant_writes_the_state_under_the_home() {
        let home = scratch("bg");
        let root = home.join("space");
        std::fs::create_dir_all(&root).expect("建工作区");
        std::fs::write(root.join("a.txt"), b"a").expect("写一个对象");
        ensure_in_background(&home, &root);
        ensure_in_background(&home, &root);
        let state = home.join("workspace-grants.json");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !state.is_file() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(
            state.is_file(),
            "后台授权完成要写状态文件：{}",
            state.display()
        );
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&state).expect("读状态")).expect("JSON");
        let key = root.canonicalize().expect("规范化根").display().to_string();
        assert_eq!(
            v["roots"][key.as_str()]["mode"],
            serde_json::json!("workspace"),
            "状态里记的是工作区粒度"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn cost_sample_skips_empty_roots_and_reused_grants() {
        let home = scratch("sample");
        let empty = home.join("empty");
        std::fs::create_dir_all(&empty).expect("建空目录");
        assert_eq!(
            refresh_cost_sample(&home, &empty, 5, true),
            0,
            "没有对象就不记样本"
        );
        let root = home.join("space");
        std::fs::create_dir_all(&root).expect("建根");
        std::fs::write(root.join("a.txt"), b"a").expect("写文件");
        assert_eq!(
            refresh_cost_sample(&home, &root, 0, false),
            1,
            "复用也要把对象数报回去"
        );
        assert!(
            !home.join("fence-costs.json").is_file(),
            "复用时的 0 ms 不能记样本"
        );
        let _ = std::fs::remove_dir_all(&home);
    }

    #[test]
    fn state_file_repairs_a_non_object_roots_value() {
        let home = scratch("repair");
        std::fs::write(home.join("workspace-grants.json"), br#"{"roots": 5}"#).expect("写坏状态");
        let root = home.join("space");
        write_state(&home, &root, Some((3, 9)), None);
        let v: serde_json::Value = serde_json::from_str(
            &std::fs::read_to_string(home.join("workspace-grants.json")).expect("读状态"),
        )
        .expect("JSON");
        let key = root.display().to_string();
        assert_eq!(v["roots"][key.as_str()]["objects"], serde_json::json!(3));
        assert_eq!(v["roots"][key.as_str()]["ms"], serde_json::json!(9));
        let _ = std::fs::remove_dir_all(&home);
    }
}
