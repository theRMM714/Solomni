//! 目的：本族测试的装配辅助。
//! 管：工作区夹具、请求构造、路径取用。
//! 不管：任何断言。
//! 联动：`src/tests/builders.rs`。

use super::super::prelude::*;

pub(crate) type WorkFace = Arc<dyn crate::capabilities::workspace::api::Workspace + Send + Sync>;

/// 目的：从一个 UNC 目录取出它的共享根（\\server\share）——用来断言"共享根不能当工作区"。
#[cfg(windows)]
pub(crate) fn share_root_of(unc: &std::path::Path) -> Option<std::path::PathBuf> {
    let (server, share, _) = crate::kernel::api::unc_parts(unc)?;
    Some(std::path::PathBuf::from(format!(
        r"\\{}\{}",
        server.to_string_lossy(),
        share.to_string_lossy()
    )))
}

/// 目的：把本机盘符路径换成同一个盘符共享（X$）下的 UNC 写法——用来造"产品根的 UNC 别名"。
/// 返回：共享名不是 <字母>$ 或盘符对不上时为 None（调用方如实跳过这条断言）。
#[cfg(windows)]
pub(crate) fn drive_share_alias(unc: &std::path::Path, local: &std::path::Path) -> Option<std::path::PathBuf> {
    let (_, share, rest) = crate::kernel::api::unc_parts(unc)?;
    if !rest.as_os_str().is_empty() {
        return None;
    }
    let share = share.to_string_lossy().into_owned();
    let letter = share.strip_suffix('$')?;
    let mut letter_chars = letter.chars();
    let drive = letter_chars.next()?.to_ascii_uppercase();
    if letter_chars.next().is_some() {
        return None;
    }
    let local_text = local.to_str()?;
    let mut chars = local_text.chars();
    if chars.next()?.to_ascii_uppercase() != drive || chars.next()? != ':' {
        return None;
    }
    let tail = chars.as_str().trim_start_matches(std::path::MAIN_SEPARATOR);
    let mut alias = unc.to_path_buf();
    for part in tail.split(std::path::MAIN_SEPARATOR) {
        if !part.is_empty() {
            alias.push(part);
        }
    }
    Some(alias)
}

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

pub(crate) fn store_of(ws: &WorkFace, work: &str) -> PathBuf {
    ws.roots(work, &[]).unwrap().store
}
