//! 目的：lifecycle 单元的测试——测试侧的回档入口，与回档标记续号这类纯判据。
//! 管：`rewind` 助手（会话本体与重建要的两个能力面在这里一次交齐）与标记号续号。
//! 不管：回档三模式的端到端行为（在 session/rewind.rs、runtime.rs 等处按真实会话验证）。
//! 联动：src/tests 各处用 `crate::tests::lifecycle::rewind` 回档；单元说明见 docs/lifecycle/README.md。

use super::prelude::*;
use crate::units::lifecycle::api::{Lifecycle, RebuildDeps, RewindTarget};

/// 目的：测试用回档入口——把会话本体与重建要的两个能力面一并交给生命周期，返回主会话活动视图。
pub(crate) fn rewind(
    core: &mut Core,
    sid: &str,
    target: RewindTarget,
) -> Result<Vec<serde_json::Value>, String> {
    let deps = RebuildDeps {
        registry: core.registry.as_ref(),
        probe: core.probe.as_ref(),
    };
    core.lifecycle.rewind(&mut core.turn, deps, &target, sid)
}

#[test]
fn mark_id_continues_after_existing_marks() {
    // 没有留档过：第一条标记从 1 开始。
    assert_eq!(Lifecycle::next_mark_id(&[]), 1);
    // 已有标记：接在最大号之后（既不按行号、也不按条数，与出现顺序无关）。
    let raw = vec![
        serde_json::json!({"type": "rewind", "mark": 7, "mode": "archive", "before": 9}),
        serde_json::json!({"type": "rewind", "mark": 3, "mode": "archive", "before": 2}),
    ];
    assert_eq!(Lifecycle::next_mark_id(&raw), 8);
    // 只有留档标记占号：别的行即便带 mark 字段也不参与。
    let raw = vec![
        serde_json::json!({"type": "say", "mark": 9}),
        serde_json::json!({"type": "rewind", "mark": 2, "mode": "archive", "before": 5}),
    ];
    assert_eq!(Lifecycle::next_mark_id(&raw), 3);
}
