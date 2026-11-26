//! 目的：外部工作区的绑定、导入与磁盘回执。
//! 管：首次导入、跨会话写锁、pull 回执、根 meta 绑定、UNC 全链路、起 guest 失败撤销。
//! 不管：共享区提交算法（`work.rs`）。
//! 联动：`docs/storage/README.md`。

use super::super::prelude::*;

/// 目的：真机验收（默认跳过）：UNC 外部工作区的全链路——虚拟机档绑定落成 native UNC、写探测真的落在共享上、
///   首绑导入 / pull / commit / status / sync 都走真实共享、删会话不删外部；产品根的 UNC 别名必须按落点身份拒。
/// 约束：只在 SOLOMNI_UNC_ROOT 指向一个可写的 UNC 目录时执行（如 \\\\localhost\D$\\\\writable\\\\dir）；
///   CI 没有 UNC，缺省打印 [探针] 如实跳过；用例自己在 UNC 根下建子目录并在结束时清掉。
#[test]
pub(crate) fn sandboxes_lookup_is_by_agent() {
    let boxes = crate::units::business::workspace::api::Sandboxes {
        shared: abs(&["w", "work"]),
        session_root: abs(&["w"]),
        list: vec![test_sandbox("甲", &["a"])],
        codec: None,
    };
    assert!(boxes.for_agent("甲").is_some(), "按 agent 实例名取沙箱");
    assert!(boxes.for_agent("a").is_none(), "沙箱不再按模块 id 反查");
}
