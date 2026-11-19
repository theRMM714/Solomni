//! 目的：共享区版本化工作区的核心自有工具（`work_pull` / `work_commit` / `work_push` / `work_status` /
//!   `work_versions` / `work_diff` / `work_reset` / `work_resolve` / `work_lost` / `work_sync`）的纯逻辑——
//!   工具名、入参解析与回执拼装。
//! 管：工具名常量与 `is_work_tool`、取参（`str_list` / `str_arg`）、各工具回执的拼装。
//! 不管：真正的拉取 / 提交 / 状态 / 三方视图 / 解决 / 备份恢复（走 `workspace::api::Workspace`）；
//!   冲突、状态与备份判定（在 `workspace`）。
//! 联动：由本能力的 `service/` 调用；工具名与 `systools/tools.yaml` 同一份名单。

use crate::capabilities::workspace::api::{
    ChangeKind, CommitReport, DiffReport, DiffSign, LostReport, PullReport, PushReport,
    ResetReport, ResolveAction, ResolveEntry, ResolveReport, RestoreLostReport, StatusReport,
    SyncReport, VERSION_TEXT_LIMIT, VersionView,
};

/// 目的：把主副本的更新拉进沙箱（`work_pull`）。
pub const PULL: &str = "work_pull";
/// 目的：把沙箱里的改动记进自己的工作线（`work_commit`，同 git commit）。
pub const COMMIT: &str = "work_commit";
/// 目的：把工作线上已提交的改动推送进主线（`work_push`，同 git push）。
pub const PUSH: &str = "work_push";
/// 目的：看主副本与工作副本的当前状态（`work_status`）。
pub const STATUS: &str = "work_status";
/// 目的：看一条路径的基线 / 主副本 / 沙箱三方视图（`work_versions`）。
pub const VERSIONS: &str = "work_versions";
/// 目的：比两个版本的行级差异（`work_diff`，缺省工作线头 ↔ 沙箱）。
pub const DIFF: &str = "work_diff";
/// 目的：把沙箱按工作线头恢复，或把工作线移回某个提交（`work_reset`）。
pub const RESET: &str = "work_reset";
/// 目的：按 `take` 解决冲突之后再提交（`work_resolve`）。
pub const RESOLVE: &str = "work_resolve";
/// 目的：列被覆盖的本地版本备份（`work_lost`）。
pub const LOST: &str = "work_lost";
/// 目的：把沙箱里的改动并入主副本（`work_sync`）。
pub const SYNC: &str = "work_sync";

/// 目的：判定这个名字是不是共享区版本化的工具面（角色表 executor / solo 引用它们）。
pub fn is_work_tool(name: &str) -> bool {
    matches!(
        name,
        PULL | COMMIT | PUSH | STATUS | VERSIONS | DIFF | RESET | RESOLVE | LOST | SYNC
    )
}

/// 回执里最多逐个列出的路径数（超出只说数量，不把整棵共享区灌进上下文）。
const MAX_LISTED: usize = 50;

fn paths(list: &[String]) -> String {
    if list.is_empty() {
        return "无".to_string();
    }
    let mut out = list
        .iter()
        .take(MAX_LISTED)
        .cloned()
        .collect::<Vec<_>>()
        .join("、");
    if list.len() > MAX_LISTED {
        out.push_str(&format!("…（共 {} 条）", list.len()));
    }
    out
}

/// 目的：把 work_pull 的结果说成人能读的几行。
/// 返回：写入 / 本地改动 / 跳过 / 冲突 / 主副本没有 各若干行（路径截断规则见 `MAX_LISTED`）。
pub fn render_pull(r: &PullReport) -> String {
    let mut out = format!(
        "已对准主副本提交 {}。写入沙箱 {} 个，已是最新 {} 个，无变化 {} 个，本地有未提交改动 {} 个，本地已删除 {} 个，跳过（沙箱已有同名文件）{} 个，冲突 {} 个，主副本没有 {} 个，工作区有外部修改 {} 个。",
        r.commit,
        r.updated.len(),
        r.already.len(),
        r.no_change.len(),
        r.local_modified.len(),
        r.local_deleted.len(),
        r.occupied.len(),
        r.conflicts.len(),
        r.missing.len(),
        r.external.len()
    );
    if !r.updated.is_empty() {
        out.push_str(&format!("\n写入：{}", paths(&r.updated)));
    }
    if !r.local_modified.is_empty() {
        out.push_str(&format!(
            "\n本地有未提交改动、没被覆盖：{}——上游这些路径没动，沙箱是你的版本；要发布先 work_commit 再 work_push。",
            paths(&r.local_modified)
        ));
    }
    if !r.local_deleted.is_empty() {
        out.push_str(&format!(
            "\n本地已删除、保持删除：{}——上游没动；要发布这次删除先 work_commit deletes=[…] 再 work_push。",
            paths(&r.local_deleted)
        ));
    }
    if !r.occupied.is_empty() {
        out.push_str(&format!(
            "\n跳过（沙箱里已有未被跟踪的同名文件，未覆盖）：{}——先用 work_versions 看主副本与本地，\n合并后用 work_resolve（take=local）登记；放弃本地改动用 take=upstream。",
            paths(&r.occupied)
        ));
    }
    if !r.conflicts.is_empty() {
        let mut lines = Vec::new();
        for c in r.conflicts.iter().take(MAX_LISTED) {
            lines.push(format!("{}（{}）", c.path, c.kind.reason()));
        }
        if r.conflicts.len() > MAX_LISTED {
            lines.push(format!("…（共 {} 条）", r.conflicts.len()));
        }
        out.push_str(&format!(
            "\n冲突（未覆盖）：{}——冲突不会因为把两边内容合进沙箱就消失：\n先用 work_versions 看基线 / 主副本 / 沙箱三方；文本可人工合并后用 work_resolve（take=local）声明已解决，图片 / 视频 / 二进制不能合并，只能选边（take=upstream 取主副本，take=local 保留本地）。\n如果是「你删了、主线改了」：work_pull 不替你决定——take=upstream 接受主线版本，或 take=local 保留删除（随后 work_commit deletes=[…] 才能发布）。",
            lines.join("、")
        ));
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n主副本没有这些路径：{}——可能上游已删除；用 work_versions 看本地与基线，\n再用 work_resolve 选择 take=upstream（删本地）或 take=local（保留为新增）。",
            paths(&r.missing)
        ));
    }
    if !r.external.is_empty() {
        out.push_str(&format!(
            "\n工作区磁盘与主副本不一致（未拉取）：{}——这是产品之外发生的改动；\n先 work_sync 把这些路径采纳成外部提交，再重新 work_pull。",
            paths(&r.external)
        ));
    }
    out
}

/// 目的：把 work_push 的结果说成人能读的几行。
/// 返回：没有新提交 = 说明为什么没有（内容已收敛 / 工作线与主线一致）；有 = 主线新提交号与改动清单。
pub fn render_push(r: &PushReport) -> String {
    let Some(id) = r.commit else {
        if r.unpushed > 0 {
            return format!(
                "没有需要推送的改动：工作线内容已与主线一致（领先 {} 条净改动为零的提交）。\n不必再提交；要清掉这些空提交，用 work_reset to=<它们之前的那条提交>。",
                r.unpushed
            );
        }
        return "没有可推送的内容：你的工作线与主线一致。\n沙箱里的改动要先 work_commit 记进工作线，再 work_push 发布。"
            .to_string();
    };
    let mut adds = Vec::new();
    let mut mods = Vec::new();
    let mut dels = Vec::new();
    for c in &r.changes {
        match c.kind {
            ChangeKind::Add => adds.push(c.path.clone()),
            ChangeKind::Modify => mods.push(c.path.clone()),
            ChangeKind::Delete => dels.push(c.path.clone()),
        }
    }
    let mut out = format!(
        "推送完成：主线提交 {}（新增 {}，修改 {}，删除 {}）。共享区已更新到这个版本，沙箱里没被本地改过的路径也跟着快进了。",
        id,
        adds.len(),
        mods.len(),
        dels.len()
    );
    if !adds.is_empty() {
        out.push_str(&format!("\n新增：{}", paths(&adds)));
    }
    if !mods.is_empty() {
        out.push_str(&format!("\n修改：{}", paths(&mods)));
    }
    if !dels.is_empty() {
        out.push_str(&format!("\n删除：{}", paths(&dels)));
    }
    out
}

/// 目的：把 work_commit 的结果说成人能读的几行。
/// 返回：提交 id 与新增 / 修改 / 删除 各若干行。
pub fn render_commit(r: &CommitReport) -> String {
    let mut adds = Vec::new();
    let mut mods = Vec::new();
    let mut dels = Vec::new();
    for c in &r.changes {
        match c.kind {
            ChangeKind::Add => adds.push(c.path.clone()),
            ChangeKind::Modify => mods.push(c.path.clone()),
            ChangeKind::Delete => dels.push(c.path.clone()),
        }
    }
    let mut out = format!(
        "提交 {} 已记进你的工作线（共享区还没变）：新增 {}，修改 {}，删除 {}。",
        r.id,
        adds.len(),
        mods.len(),
        dels.len()
    );
    if !adds.is_empty() {
        out.push_str(&format!("\n新增：{}", paths(&adds)));
    }
    if !mods.is_empty() {
        out.push_str(&format!("\n修改：{}", paths(&mods)));
    }
    if !dels.is_empty() {
        out.push_str(&format!("\n删除（推送后才从共享区去掉）：{}", paths(&dels)));
        out.push_str(
            "\n沙箱里的同名文件如果还在，用 delete 工具清理；工作副本的删除不会自动跟着提交。",
        );
    }
    out.push_str("\n要让它出现在共享区给别的 agent 看，用 work_push 推送（主线落一条提交）。");
    out
}

/// 目的：把 work_diff 的结果渲染成 unified 格式（`---` / `+++` / `@@`），二进制与过大只说明一句。
/// 返回：两侧标签、逐路径 hunk；没有差异时一句话。
pub fn render_diff(r: &DiffReport) -> String {
    if r.entries.is_empty() {
        return format!("{} → {} 没有差异。", r.from, r.to);
    }
    let mut out = format!("{} → {}（{} 个路径）", r.from, r.to, r.entries.len());
    if r.truncated {
        out.push_str("\n（内容超出预算，只显示了前面一部分）");
    }
    for e in &r.entries {
        let path = &e.path;
        match e.kind {
            ChangeKind::Add => {
                out.push_str(&format!("\n--- /dev/null\n+++ b/{}", path));
            }
            ChangeKind::Delete => {
                out.push_str(&format!("\n--- a/{}\n+++ /dev/null", path));
            }
            ChangeKind::Modify => {
                out.push_str(&format!("\n--- a/{path}\n+++ b/{path}"));
            }
        }
        if e.binary {
            out.push_str("\n（二进制文件，只报变更类型与指纹，不展开行差异）");
            continue;
        }
        if e.too_large {
            out.push_str(
                "\n（超过单文件上限，不展开行差异；用 read 看沙箱版本，或 work_versions 看三方）",
            );
            continue;
        }
        for h in &e.hunks {
            out.push_str(&format!(
                "\n@@ -{},{} +{},{} @@",
                h.old_start, h.old_len, h.new_start, h.new_len
            ));
            for line in &h.lines {
                let sign = match line.sign {
                    DiffSign::Context => ' ',
                    DiffSign::Remove => '-',
                    DiffSign::Add => '+',
                };
                out.push_str(&format!("\n{}{}", sign, line.text));
            }
        }
    }
    out
}

/// 目的：把 work_reset 的结果说成人能读的几行：工作线动没动、沙箱铺回 / 删除了什么、备份了几份。
/// 约束：只看报告里的两个事实（调用前后的工作线头），不猜调用方给的是 paths 还是 to。
pub fn render_reset(r: &ResetReport) -> String {
    let moved = r.before != r.after;
    let mut out = match (r.before, r.after) {
        (Some(a), _) if !moved => format!("工作线没动（还是提交 {}）。", a),
        (Some(a), Some(b)) => format!("工作线从提交 {} 移到 {}。", a, b),
        (None, Some(b)) => format!("工作线现在在提交 {}。", b),
        (Some(a), None) => format!("工作线已从提交 {} 退回未提交状态。", a),
        (None, None) => "工作线还没有提交（工作线没动）。".to_string(),
    };
    if r.restored.is_empty() && r.removed.is_empty() {
        out.push_str("\n沙箱没动");
        match (moved, r.after) {
            (true, _) => out.push_str("（目标版本与沙箱内容本来一致，或你用了 keep_changes）"),
            (false, Some(_)) => out.push_str("（这些路径本来就与工作线头一致）"),
            (false, None) => out.push_str("（这些路径本来就与拉取基线一致）"),
        }
        out.push('.');
        if moved {
            out.push_str("\n要连沙箱一起重铺，再 work_reset 到同一个提交（不带 keep_changes）。");
        }
    } else {
        match (moved, r.after) {
            (true, Some(b)) => out.push_str(&format!("\n沙箱按提交 {} 重铺。", b)),
            (_, Some(_)) => {
                out.push_str("\n沙箱按工作线头恢复（工作线里没有的路径退回拉取基线）。")
            }
            (_, None) => out.push_str("\n沙箱按拉取基线恢复。"),
        }
        if !r.restored.is_empty() {
            out.push_str(&format!(
                "\n铺回 {} 个文件：{}",
                r.restored.len(),
                paths(&r.restored)
            ));
        }
        if !r.removed.is_empty() {
            out.push_str(&format!(
                "\n删除 {} 个文件（{}）：{}",
                r.removed.len(),
                if moved {
                    "那个版本里没有"
                } else {
                    "工作线里没有"
                },
                paths(&r.removed)
            ));
        }
    }
    if r.backed_up > 0 {
        out.push_str(&format!(
            "\n覆盖前备份了 {} 份旧内容（备份位置 lost/@reset-*；work_lost 列出原路径与备份位置）。",
            r.backed_up
        ));
    }
    out
}

/// 目的：把 work_sync 的结果说成人能读的几行。
/// 返回：采纳后的提交 id、新增 / 修改 / 删除、未变与缺失路径。
pub fn render_sync(r: &SyncReport) -> String {
    if r.changes.is_empty() {
        let mut out = "工作区与主副本已经一致，没有需要同步的改动。".to_string();
        if !r.missing.is_empty() {
            out.push_str(&format!(
                "\n工作区与版本库里都没有这些路径：{}",
                paths(&r.missing)
            ));
        }
        return out;
    }
    let mut adds = Vec::new();
    let mut mods = Vec::new();
    let mut dels = Vec::new();
    for c in &r.changes {
        match c.kind {
            ChangeKind::Add => adds.push(c.path.clone()),
            ChangeKind::Modify => mods.push(c.path.clone()),
            ChangeKind::Delete => dels.push(c.path.clone()),
        }
    }
    let mut out = format!(
        "外部改动已采纳为提交 {}：新增 {}，修改 {}，删除 {}；未变 {}。",
        r.commit.unwrap_or(0),
        adds.len(),
        mods.len(),
        dels.len(),
        r.unchanged.len()
    );
    if !adds.is_empty() {
        out.push_str(&format!("\n新增：{}", paths(&adds)));
    }
    if !mods.is_empty() {
        out.push_str(&format!("\n修改：{}", paths(&mods)));
    }
    if !dels.is_empty() {
        out.push_str(&format!("\n删除：{}", paths(&dels)));
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n工作区与版本库里都没有这些路径：{}",
            paths(&r.missing)
        ));
    }
    out.push_str("\n各 agent 的沙箱没动；需要这些内容就再 work_pull。");
    out
}

/// 目的：把 work_status 的结果说成人能读的几行。
/// 返回：主线当前提交、你的工作线头与待推送净改动数（另含领先空提交条数与冲突数）、逐条五态、最近主线提交。
pub fn render_status(r: &StatusReport) -> String {
    let mut out = String::new();
    match &r.head {
        Some(h) => out.push_str(&format!(
            "共享区（主线）当前提交 {}（{}，{}）：{}",
            h.id, h.author, h.time, h.message
        )),
        None => out.push_str("共享区（主线）还没有任何提交"),
    }
    match &r.work_head {
        Some(w) => {
            out.push_str(&format!(
                "\n你的工作线：提交 {}（{}）：{}",
                w.id, w.time, w.message
            ));
            if r.conflicts > 0 {
                out.push_str(&format!(
                    "\n· 推送会冲突：{} 个路径（work_push 会整次被拒，先看冲突清单再解决）",
                    r.conflicts
                ));
            } else if r.pending > 0 {
                out.push_str(&format!(
                    "\n· 未推送 {} 项改动（分布在 {} 条提交里，要进共享区用 work_push）",
                    r.pending, r.unpushed
                ));
            } else if r.unpushed > 0 {
                out.push_str(&format!(
                    "\n· 内容已与主线一致（领先 {} 条净改动为零的提交；不必再推送，要清掉用 work_reset to=<它们的父提交>）",
                    r.unpushed
                ));
            } else {
                out.push_str("\n· 已全部推送");
            }
        }
        None => out.push_str("\n你的工作线：还没有提交（与主线一致）"),
    }
    if !r.tree.is_empty() {
        out.push_str(&format!("\n主副本路径：{}", paths(&r.tree)));
        if r.tree_truncated {
            out.push_str("…（已截断，只列了一部分）");
        }
    }
    if !r.entries.is_empty() {
        out.push_str("\n你的沙箱（相对你的工作线）：");
        for e in &r.entries {
            out.push_str(&format!("\n- {}：{}", e.path, e.state.as_str()));
        }
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n这些路径在基线、主副本与沙箱里都没有：{}",
            paths(&r.missing)
        ));
    }
    if !r.external.is_empty() {
        out.push_str(&format!(
            "\n工作区磁盘与主副本不一致（外部改动 / 删除，未自动处理）：{}——先 work_sync 采纳外部版本再继续。",
            paths(&r.external)
        ));
    }
    if !r.recent.is_empty() {
        out.push_str("\n最近提交：");
        for c in &r.recent {
            out.push_str(&format!("\n- {} {}：{}", c.id, c.author, c.message));
        }
    }
    out
}

/// 目的：把 work_versions 的三方视图说成人能读的几行。
/// 返回：基线 / 主副本 / 沙箱的指纹、大小与文本预览；二进制只给指纹与大小。
pub fn render_versions(v: &VersionView) -> String {
    let mut out = format!("路径：{}", v.path);
    match &v.media_type {
        Some(t) => out.push_str(&format!("（媒体类型：{}，按扩展名提示）", t)),
        None => out.push_str("（媒体类型：未识别）"),
    }
    if let Some(id) = v.head_commit {
        out.push_str(&format!(
            "\n主副本提交：{}（take=local 时把这个 id 作为 head 传回）",
            id
        ));
    }
    let mut incomplete: Vec<&str> = Vec::new();
    for (label, item) in [("基线", &v.base), ("主副本", &v.head), ("沙箱", &v.local)] {
        if let Some(f) = item
            && (f.truncated || f.too_large)
        {
            incomplete.push(label);
        }
    }
    if !incomplete.is_empty() {
        out.push_str(&format!(
            "\n注意：{}的内容预览不完整/超限，不能只按这段内容合并；请用 read 读沙箱文件与共享主副本同一相对路径的全文，再合并。",
            incomplete.join("、")
        ));
    }
    for (label, item) in [("基线", &v.base), ("主副本", &v.head), ("沙箱", &v.local)] {
        match item {
            None => out.push_str(&format!(
                "\n{}：{}",
                label,
                if label == "基线" {
                    "未拉取"
                } else {
                    "不存在"
                }
            )),
            Some(f) => {
                let kind = if f.too_large {
                    "超过单文件上限"
                } else if f.text.is_some() {
                    "文本"
                } else {
                    "二进制/非 UTF-8"
                };
                out.push_str(&format!(
                    "\n{}：{}，hash={}，大小={} 字节",
                    label, kind, f.hash, f.size
                ));
                if f.too_large {
                    out.push_str("（只给指纹、大小与类型，不预览）");
                }
                if let Some(text) = &f.text {
                    let kb = VERSION_TEXT_LIMIT / 1024;
                    if f.truncated {
                        out.push_str(&format!(
                            "\n--- {}内容（只显示前 {} KiB，共 {} 字节）---\n{}",
                            label, kb, f.size, text
                        ));
                    } else {
                        out.push_str(&format!("\n--- {}内容 ---\n{}", label, text));
                    }
                }
            }
        }
    }
    out.push_str(&format!(
        "\n提示：文本预览最多 {} KiB；图片 / 视频 / 其它二进制只给指纹、大小与类型，不冒充文本；冲突只能显式选边：文本可人工合并后用 take=local（带 head=<上面主副本提交 id>），二进制 / 媒体只能 take=upstream 或 take=local，不做自动合并。",
        VERSION_TEXT_LIMIT / 1024
    ));
    out
}

/// 目的：把 work_resolve 的结果说成人能读的几行。
/// 返回：按动作分组的路径，以及下一步。
pub fn render_resolve(r: &ResolveReport) -> String {
    let mut out = format!(
        "已按 take={} 解决 {} 条路径。",
        r.take.as_str(),
        r.entries.len()
    );
    if let Some(id) = r.head {
        out.push_str(&format!("\n基于主副本提交：{}", id));
    }
    if !r.warnings.is_empty() {
        out.push_str("\n注意：");
        for w in r.warnings.iter().take(MAX_LISTED) {
            out.push_str(&format!("\n- {}：{}", w.path, w.kind.as_str()));
        }
        if r.warnings.len() > MAX_LISTED {
            out.push_str(&format!("\n…（共 {} 条）", r.warnings.len()));
        }
    }
    for action in [
        ResolveAction::WroteUpstream,
        ResolveAction::RemovedLocal,
        ResolveAction::Marked,
        ResolveAction::Cleared,
        ResolveAction::NoChange,
    ] {
        let list: Vec<String> = r
            .entries
            .iter()
            .filter(|e| e.action == action)
            .map(|e| e.path.clone())
            .collect();
        if !list.is_empty() {
            out.push_str(&format!("\n{}：{}", action.as_str(), paths(&list)));
        }
    }
    let backups: Vec<&ResolveEntry> = r.entries.iter().filter(|e| e.backup.is_some()).collect();
    if !backups.is_empty() {
        out.push_str(
            "\n已备份被覆盖/删除的本地版本（位置在版本库 lost/<agent>/ 下，可用 work_lost 找回）：",
        );
        for e in backups.iter().take(MAX_LISTED) {
            out.push_str(&format!(
                "\n- {} → {}",
                e.path,
                e.backup.as_deref().unwrap_or("")
            ));
        }
        if backups.len() > MAX_LISTED {
            out.push_str(&format!("\n…（共 {} 条）", backups.len()));
        }
    }
    if !r.missing.is_empty() {
        out.push_str(&format!(
            "\n基线、主副本与沙箱都没有：{}",
            paths(&r.missing)
        ));
    }
    out.push_str(
        "\n下一步：take=local 保留的是沙箱当前内容（文本合并结果或二进制选边结果），提交会以本地覆盖主副本当前版；请确认已包含上游改动。take=upstream 已把沙箱对齐主副本；被覆盖/删除的本地版本在 lost/ 下，可用 work_lost 找回。",
    );
    out
}

/// 目的：把 work_lost 的清单说成人能读的几行：原路径、备份位置与来源、指纹、当前沙箱指纹与恢复方式。
pub fn render_lost(r: &LostReport) -> String {
    if r.entries.is_empty() {
        return "没有找到被覆盖的本地版本。覆盖 / 删除 / 回退 / 留档还原前被顶掉的沙箱版本都会备份在 lost/ 下。"
            .to_string();
    }
    let mut out = format!("被覆盖的本地版本（agent {}）：共 {} 份", r.agent, r.total);
    if r.truncated {
        out.push_str(&format!("（只列前 {} 份）", r.entries.len()));
    }
    for e in r.entries.iter().take(MAX_LISTED) {
        out.push_str(&format!(
            "\n- {}：备份位置 {}，来源 {}；hash={}，大小={} 字节；当前沙箱={}",
            e.path,
            e.backup,
            e.origin.as_deref().unwrap_or("未标记（旧备份）"),
            e.hash,
            e.size,
            match &e.current_hash {
                Some(hash) => format!("hash={}", hash),
                None => "不存在".to_string(),
            }
        ));
    }
    if r.entries.len() > MAX_LISTED {
        out.push_str(&format!("\n…（共 {} 条）", r.entries.len()));
    }
    out.push_str(
        "\n恢复：work_lost path=<路径> restore=true expect_current=<当前沙箱 hash；不存在传空串>；恢复会覆盖当前沙箱同路径，先确认新内容已另存/提交。\n同一路径有多份备份时，把 path 传成备份位置（@<来源>/<路径>）指明一份。",
    );
    out
}

/// 目的：把 work_restore_lost 的结果说成人能读的几行。
pub fn render_restore_lost(r: &RestoreLostReport) -> String {
    format!(
        "已恢复 {}（用备份 {}）：hash={}，大小={} 字节；覆盖前沙箱={}。备份仍保留在 lost/ 下，可再次恢复。恢复后请 work_versions 看三方，合并后再提交。",
        r.path,
        r.backup,
        r.hash,
        r.size,
        match &r.previous {
            Some(hash) => format!("hash={}", hash),
            None => "不存在".to_string(),
        }
    )
}

/// 目的：从 args 里取一个字符串数组（缺省 = 空）。
/// 错误：不是数组、或数组里有非字符串时如实报错。
pub fn str_list(args: &serde_json::Value, key: &str) -> Result<Vec<String>, String> {
    match args.get(key) {
        None | Some(serde_json::Value::Null) => Ok(Vec::new()),
        Some(serde_json::Value::Array(items)) => {
            let mut out = Vec::new();
            for it in items {
                match it.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err(format!("{} 的每一项都必须是字符串", key)),
                }
            }
            Ok(out)
        }
        Some(_) => Err(format!("{} 必须是一个字符串数组", key)),
    }
}

/// 目的：从 args 里取一个字符串（缺省 = `None`）。
pub fn str_arg(args: &serde_json::Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capabilities::workspace::api::FileVersion;

    fn version(hash: &str, size: u64, text: Option<&str>, too_large: bool) -> FileVersion {
        FileVersion {
            hash: hash.to_string(),
            size,
            text: text.map(str::to_string),
            truncated: false,
            too_large,
        }
    }

    /// 目的：回执按媒体类型说人话；二进制不给“内容”段，也不假装文本。
    #[test]
    fn versions_receipt_reports_media_type_and_never_previews_binary() {
        let v = VersionView {
            path: "clip.mp4".to_string(),
            head_commit: None,
            media_type: Some("video/mp4".to_string()),
            base: None,
            head: Some(version("h", 9, None, false)),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("video/mp4"), "{}", text);
        assert!(text.contains("二进制/非 UTF-8"), "{}", text);
        assert!(
            !text.contains("--- 主副本内容"),
            "二进制不能给内容段：{}",
            text
        );
    }

    /// 目的：超过上限只给元数据，回执明确说“不预览”。
    #[test]
    fn versions_receipt_marks_oversize_metadata_only() {
        let v = VersionView {
            path: "big.bin".to_string(),
            head_commit: None,
            media_type: None,
            base: None,
            head: Some(version("h", 70_000_000, None, true)),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("超过单文件上限"), "{}", text);
        assert!(text.contains("不预览"), "{}", text);
        assert!(text.contains("不能只按"), "{}", text);
        assert!(!text.contains("--- 主副本内容"), "{}", text);
    }

    /// 目的：文本预览仍按上限显示，并在截断时说明。
    #[test]
    fn versions_receipt_keeps_text_preview_with_limit() {
        let mut f = version("h", (VERSION_TEXT_LIMIT + 10) as u64, Some("开头"), false);
        f.truncated = true;
        let v = VersionView {
            path: "note.md".to_string(),
            head_commit: Some(7),
            media_type: Some("text/markdown".to_string()),
            base: None,
            head: Some(f),
            local: None,
        };
        let text = render_versions(&v);
        assert!(text.contains("--- 主副本内容"), "{}", text);
        assert!(text.contains("只显示前"), "{}", text);
        assert!(text.contains("主副本提交：7"), "{}", text);
        assert!(text.contains("不能只按"), "{}", text);
        assert!(text.contains("read"), "{}", text);
    }

    /// 目的：resolve 回执要说清基于哪个主副本、哪些路径有风险、哪些本地版本已备份。
    #[test]
    fn resolve_receipt_shows_head_warnings_and_backups() {
        use crate::capabilities::workspace::api::{
            ResolveReport, ResolveTake, ResolveWarning, ResolveWarningKind,
        };
        let r = ResolveReport {
            take: ResolveTake::Local,
            head: Some(5),
            entries: vec![
                ResolveEntry {
                    path: "note.md".to_string(),
                    action: ResolveAction::Marked,
                    backup: None,
                },
                ResolveEntry {
                    path: "cover.png".to_string(),
                    action: ResolveAction::WroteUpstream,
                    backup: Some("@resolve/cover.png".to_string()),
                },
            ],
            missing: Vec::new(),
            warnings: vec![ResolveWarning {
                path: "other.md".to_string(),
                kind: ResolveWarningKind::NoBase,
            }],
        };
        let text = render_resolve(&r);
        assert!(text.contains("基于主副本提交：5"), "{}", text);
        assert!(
            text.contains("other.md") && text.contains("从未拉取"),
            "{}",
            text
        );
        assert!(text.contains("@resolve/cover.png"), "{}", text);
        assert!(text.contains("work_lost"), "{}", text);
    }

    /// 目的：work_lost / restore 回执给出备份指纹、当前沙箱指纹与恢复结果。
    #[test]
    fn lost_receipts_show_backup_and_current_hashes() {
        use crate::capabilities::workspace::api::LostEntry;
        let r = LostReport {
            agent: "a".to_string(),
            total: 1,
            entries: vec![LostEntry {
                path: "note.md".to_string(),
                backup: "@reset-paths/note.md".to_string(),
                origin: Some("reset-paths".to_string()),
                hash: "hb".to_string(),
                size: 3,
                current_hash: Some("hc".to_string()),
            }],
            truncated: false,
        };
        let text = render_lost(&r);
        assert!(text.contains("note.md") && text.contains("hb") && text.contains("hc"));
        assert!(text.contains("@reset-paths/note.md"), "{}", text);
        assert!(text.contains("work_lost"), "{}", text);
        let rr = RestoreLostReport {
            path: "note.md".to_string(),
            backup: "@reset-paths/note.md".to_string(),
            hash: "hb".to_string(),
            size: 3,
            previous: Some("hc".to_string()),
        };
        let text = render_restore_lost(&rr);
        assert!(text.contains("已恢复") && text.contains("hb") && text.contains("hc"));
    }
}
