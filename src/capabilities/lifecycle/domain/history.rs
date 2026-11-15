//! 目的：会话历史的纯派生——树序排列与子树收集，不碰盘、不碰日志。
//! 管：`tree_order`（父在前、子紧跟父）与 `subtree_of`（按 `meta.parent` 展开整棵子树）。
//! 不管：读盘（`service/history.rs`）；删除收尾（`service/delete.rs`）。
//! 联动：`service/history.rs` 的列表先过树序；`conductor` 的删除按子树收集目录。

use crate::capabilities::session::api::HistoryView;

/// 目的：会话列表按**树序**排：顶层保持后端给的时间倒序，每个会话后面**紧跟**它的子会话
///  （子会话内部同样按时间倒序），深度优先。
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
pub fn subtree_of(list: &[HistoryView], name: &str) -> Vec<String> {
    let mut out = vec![name.to_string()];
    let mut i = 0;
    while i < out.len() {
        let cur = out[i].clone();
        for h in list {
            if h.parent.as_deref() == Some(cur.as_str()) && !out.iter().any(|x| x == &h.name) {
                out.push(h.name.clone());
            }
        }
        i += 1;
    }
    out
}
