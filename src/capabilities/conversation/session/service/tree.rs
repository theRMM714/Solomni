//! 目的：会话树的派生——父子关系、工作根、配置归属、树级装配 meta 与 guest 作用域。
//! 管：`subtree_of` / `work_root` / `work_root_of` / `config_owner_of` / `tree_meta` / `tree_meta_with` / `session_kind_of` / `role_of` / `guest_scope` / `guest_lease_for` / `guest_lease` / `tree_order` / `flat_modules`。
//! 不管：谁在跑（`turn`）、目录怎么落盘（`detail/fs_history.rs`）、外部工作区的档位策略（`workspace`）。
//! 联动：只经 `History` 端口读 meta 与列表；`runtime` 的建工作、改配、回档与历史列表都调这里。

use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::conversation::session::api::{History, HistoryView, Mode, SessionMeta};
use crate::kernel::api::types::Tier;

/// 目的：整张名单的扁平模块清单（保序去重）：同一模块被多个 agent 共用时只出现一次。
pub fn flat_modules(agents: &[AgentMeta]) -> Vec<String> {
    let mut out = Vec::new();
    for a in agents {
        for m in &a.modules {
            if !out.contains(m) {
                out.push(m.clone());
            }
        }
    }
    out
}

/// 目的：会话列表的**树序**：父在前、子紧跟在后（子会话内部同样按时间倒序），深度优先。
///  为什么必须在后端做：顺序与缩进是同一件事的两面——前端各排各的，就会出现子会话排在父会话上面。
pub fn tree_order(list: Vec<HistoryView>) -> Vec<HistoryView> {
    let mut kids: std::collections::BTreeMap<String, Vec<HistoryView>> =
        std::collections::BTreeMap::new();
    let mut roots: Vec<HistoryView> = Vec::new();
    for h in list {
        match h.parent.clone() {
            Some(p) => kids.entry(p).or_default().push(h),
            None => roots.push(h),
        }
    }
    let mut out: Vec<HistoryView> = Vec::new();
    for r in roots {
        let mut stack = vec![r];
        while let Some(cur) = stack.pop() {
            let mut mine = kids.remove(&cur.name).unwrap_or_default();
            mine.sort_by_key(|h| std::cmp::Reverse(h.ts));
            // 逆序压栈 → 弹出时按时间倒序（与顶层同一口径）。
            for k in mine.into_iter().rev() {
                stack.push(k);
            }
            out.push(cur);
        }
    }
    // 父会话已不在（被删）的子会话：如实列在最后，不能凭空消失。
    for (_, v) in kids {
        out.extend(v);
    }
    out
}

/// 目的：一个会话的整棵子树（含它自己，父在前、子在后）：父子关系以 `meta.parent` 为唯一判据。
/// 约束：列表读失败时只认它自己——树的关系以落盘事实为准，读不到就没有子树。
pub fn subtree_of(history: &dyn History, name: &str) -> Vec<String> {
    let list = tree_order(history.list().unwrap_or_default());
    let mut out = vec![name.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        for h in &list {
            if h.parent.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &h.name) {
                out.push(h.name.clone());
            }
        }
        i += 1;
    }
    out
}

/// 目的：一个会话的**工作根**（顶层会话名）：沿 `meta.parent` 一路走到顶。
///  整棵树不论嵌套多少层只有一个 `work/`——共享区与 agent 沙箱都锚在它上面。
/// 错误：`meta` 读不到或父子关系成环。
pub fn work_root(history: &dyn History, name: &str) -> Result<String, String> {
    let mut cur = history.meta(name)?;
    let mut guard = 0usize;
    while let Some(p) = cur.parent.clone() {
        guard += 1;
        if guard > 64 {
            return Err(format!("会话父子关系成环：{}", name));
        }
        cur = history.meta(&p)?;
    }
    Ok(cur.name)
}

/// 目的：一个会话的**工作根**，从它自己的 meta 起算：`meta` 可能**还没落盘**
///  （建工作区发生在落盘之前），所以有父就上溯、没有父就是它自己。
pub fn work_root_of(history: &dyn History, meta: &SessionMeta) -> Result<String, String> {
    match &meta.parent {
        None => Ok(meta.name.clone()),
        Some(p) => work_root(history, p),
    }
}

/// 目的：一个 agent 的配置归属哪份名单：协作成员会话的名单在主会话，
///   proxy 子工作（父是代理）或顶层会话自己就是名单所有者；嵌套协作继续向上找。
/// 约束：同一 agent 全树只有一份配置——子会话编辑写回这里，不留两份会漂的 meta。
pub fn config_owner_of(history: &dyn History, meta: &SessionMeta) -> Result<SessionMeta, String> {
    let mut owner = meta.clone();
    while let Some(p) = owner.parent.clone() {
        let pm = history.meta(&p)?;
        if pm.mode == Mode::Collab {
            owner = pm;
        } else {
            break;
        }
    }
    Ok(owner)
}

/// 目的：一台共享 guest 的**树级装配 meta**——把作用域根与其整棵子树的 agent 并成一份。
/// 约束：同一 agent 名只留一份（后代覆盖祖先，便于子会话编辑后按新值装配）；exec / 形态 / 工作名取作用域根。
///   extra 用于"还没落盘"的待建子会话（建工作与改配的准备段要它）。
pub fn tree_meta(
    history: &dyn History,
    root: &SessionMeta,
    extra: Option<&SessionMeta>,
) -> Result<SessionMeta, String> {
    let extras: Vec<&SessionMeta> = extra.into_iter().collect();
    tree_meta_with(history, root, &extras)
}

/// 目的：同上，但是可带多份尚未落盘的 meta（改配时主会话与子树的新值要一起参与装配）。
pub fn tree_meta_with(
    history: &dyn History,
    root: &SessionMeta,
    extra: &[&SessionMeta],
) -> Result<SessionMeta, String> {
    let mut agents: std::collections::BTreeMap<String, AgentMeta> =
        std::collections::BTreeMap::new();
    let mut push = |m: &SessionMeta| {
        for a in &m.agents {
            agents.insert(a.name.clone(), a.clone());
        }
    };
    push(root);
    for sid in subtree_of(history, &root.name) {
        if sid == root.name {
            continue;
        }
        if let Ok(m) = history.meta(&sid) {
            push(&m);
        }
    }
    for m in extra {
        push(m);
    }
    let mut out = root.clone();
    out.agents = agents.into_values().collect();
    out.modules = flat_modules(&out.agents);
    Ok(out)
}

/// 目的：这个会话的**使用类型**（机制册的第一把钥匙）：按落盘形态派生，重建后是同一份。
///  协作派生的成员 / 节点会话算 **collab**（它属于那场协作），其余单 agent 会话算 single，代理算 proxy。
pub fn session_kind_of(history: &dyn History, meta: &SessionMeta) -> Result<String, String> {
    let kind = match meta.mode {
        Mode::Proxy => Mode::Proxy,
        Mode::Collab => Mode::Collab,
        // 单 agent 会话：父是协作会话时它属于那场协作。
        Mode::Single => match meta.parent.as_deref() {
            Some(p) => match history.meta(p) {
                Ok(pm) if pm.mode == Mode::Collab => Mode::Collab,
                _ => Mode::Single,
            },
            None => Mode::Single,
        },
    };
    Ok(kind.as_str().to_string())
}

/// 目的：这一席的**角色**（机制册的第二把钥匙，也是工具面与提示词的角色名）。
///  与实时建立同一把尺子：协作的成员 / 节点 = executor；代理 = core_proxy；其余单 agent 工作 = solo。
pub fn role_of(history: &dyn History, meta: &SessionMeta) -> Result<String, String> {
    if meta.mode == Mode::Proxy {
        return Ok("core_proxy".to_string());
    }
    match meta.parent.as_deref() {
        Some(p) => match history.meta(p) {
            Ok(pm) if pm.mode == Mode::Collab => Ok("executor".to_string()),
            _ => Ok("solo".to_string()),
        },
        None => Ok("solo".to_string()),
    }
}

/// 目的：这条会话的共享 guest 租约；空 = 本机档，或代理父会话自己没有 guest。
/// 约束：一个主会话树一台 guest；proxy 父会话没有 guest，它的每个子工作临时各占一台。
pub fn guest_lease(meta: &SessionMeta, top: &SessionMeta) -> Option<String> {
    if top.exec.tier != Tier::Vm {
        return None;
    }
    if top.mode == Mode::Proxy && meta.parent.is_none() {
        return None;
    }
    Some(top.name.clone())
}

/// 目的：这台会话的**共享 guest 作用域**——(guest 租约, 该租约对应的会话 meta)。
/// 约束：一个主会话树一台 guest；proxy 父会话没有 guest，它的每个子工作临时各占一台。
///   host 档 / proxy 父会话 = None；其余 = (租约, 作用域根 meta)。
pub fn guest_scope(
    history: &dyn History,
    meta: &SessionMeta,
) -> Result<Option<(String, SessionMeta)>, String> {
    if meta.exec.tier != Tier::Vm {
        return Ok(None);
    }
    // 顶层工作名：子会话沿 parent 走到顶（整棵树只有一个 work/）。
    let root = work_root_of(history, meta)?;
    let top = if root == meta.name {
        meta.clone()
    } else {
        history.meta(&root)?
    };
    // 租约判据：proxy 父会话自己没有 guest，子工作共用代理根那台。
    match guest_lease(meta, &top) {
        Some(lease) => Ok(Some((lease, top))),
        None => Ok(None),
    }
}

/// 目的：这台会话的工具执行要用的 guest 租约（空 = 本机档 / 没有 guest）。
pub fn guest_lease_for(history: &dyn History, meta: &SessionMeta) -> Result<String, String> {
    Ok(guest_scope(history, meta)?
        .map(|(lease, _)| lease)
        .unwrap_or_default())
}
