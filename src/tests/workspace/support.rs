//! 目的：本族测试的装配辅助。
//! 管：工作区夹具、请求构造、路径取用。
//! 不管：任何断言。
//! 联动：`src/tests/builders/`。

use super::super::prelude::*;

pub(crate) type WorkFace = Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync>;

/// 目的：装配一个只用内存端口的工作区能力面：目录布局 + 版本库都是内存替身。
pub(crate) fn work_ws() -> (WorkFace, Arc<InMemoryWorkspace>, Arc<InMemoryWorkStore>) {
    let dirs = Arc::new(InMemoryWorkspace::new());
    let store = Arc::new(InMemoryWorkStore::new());
    let ws = test_workspace_store(dirs.clone(), store.clone());
    (ws, dirs, store)
}

pub(crate) fn commit_req(
    paths: &[&str],
    deletes: &[&str],
    message: &str,
    time: i64,
) -> crate::capabilities::workspace::api::CommitRequest {
    crate::capabilities::workspace::api::CommitRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        deletes: deletes.iter().map(|s| s.to_string()).collect(),
        message: message.to_string(),
        time,
        line: 0,
    }
}

pub(crate) fn resolve_req(
    paths: &[&str],
    take: crate::capabilities::workspace::api::ResolveTake,
) -> crate::capabilities::workspace::api::ResolveRequest {
    crate::capabilities::workspace::api::ResolveRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        take,
        expect_head: None,
    }
}

/// 目的：推送请求——`line` 是这条推送的锚行，回档按它选点。
pub(crate) fn push_req(
    message: &str,
    time: i64,
    line: u64,
) -> crate::capabilities::workspace::api::PushRequest {
    crate::capabilities::workspace::api::PushRequest {
        message: message.to_string(),
        time,
        line,
    }
}

/// 目的：把 agent 的工作线推送进共享区（三方合并 + 主线落一条提交）；"提交即发布"的用例用它收口。
pub(crate) fn push(
    ws: &WorkFace,
    work: &str,
    agent: &str,
    message: &str,
    time: i64,
    line: u64,
) -> Result<crate::capabilities::workspace::api::PushReport, String> {
    ws.work_push(
        work,
        agent,
        &format!("w--{}", agent),
        &push_req(message, time, line),
    )
}

/// 目的：take=local 的请求要带当前主副本提交 id（并发窗口守卫）；测试统一从 work_head 取。
pub(crate) fn resolve_local_req(
    ws: &WorkFace,
    work: &str,
    paths: &[&str],
) -> crate::capabilities::workspace::api::ResolveRequest {
    crate::capabilities::workspace::api::ResolveRequest {
        paths: paths.iter().map(|s| s.to_string()).collect(),
        take: crate::capabilities::workspace::api::ResolveTake::Local,
        expect_head: ws.work_head(work).unwrap(),
    }
}

pub(crate) fn sandbox_of(ws: &WorkFace, work: &str, agent: &str) -> PathBuf {
    ws.roots(work, &[agent.to_string()])
        .unwrap()
        .agents
        .get(agent)
        .cloned()
        .unwrap()
}

pub(crate) fn shared_of(ws: &WorkFace, work: &str) -> PathBuf {
    ws.roots(work, &[]).unwrap().shared
}
