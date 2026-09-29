//! **任务链**（纯领域业务）测试：纯数据与纯图算法。
//! 归属判据：钉的是**这个业务的不变式**（阶段派生 / 就绪 / 验收判定）；顺手经过别处只是路径，不是归属。
use super::builders::*;

/// 串：链式依赖按序就绪——前一环没完成，后一环不开始。
/// **阶段**由依赖图派生（最长路径分层）：串行链一个节点一阶段；并 + 混合的两条并行同阶段、汇合点下一阶段。
#[test]
pub(crate) fn chain_stages_lay_out_along_the_dependency_graph() {
    let serial = crate::capabilities::taskchain::api::TaskChain {
        nodes: vec![
            chain_node("a", &[]),
            chain_node("b", &["a"]),
            chain_node("c", &["b"]),
        ],
    };
    assert_eq!(serial.stages(), vec![1, 2, 3]);
    assert_eq!(serial.stage_nodes(1).len(), 1);
    assert_eq!(serial.current_stage(), Some(1));
    let mut parallel = crate::capabilities::taskchain::api::TaskChain {
        nodes: vec![
            chain_node("a", &[]),
            chain_node("b", &[]),
            chain_node("c", &["a", "b"]),
        ],
    };
    assert_eq!(parallel.stages(), vec![1, 1, 2]);
    let mut ids: Vec<String> = parallel
        .stage_ready(1)
        .iter()
        .map(|n| n.id.clone())
        .collect();
    ids.sort();
    assert_eq!(ids, vec!["a".to_string(), "b".to_string()], "同阶段一起派");
    assert!(parallel.stage_ready(1).iter().all(|n| n.id != "c"));
    assert!(parallel.current_stage() == Some(1));
    // 阶段一的两个节点都结束并通过 → 阶段一过了，当前阶段前进到二。
    for i in [0, 1] {
        parallel.nodes[i].status = crate::capabilities::taskchain::api::NodeStatus::Done;
        parallel.nodes[i].acceptance = Some(crate::capabilities::taskchain::api::Acceptance {
            ok: true,
            note: String::new(),
        });
    }
    assert!(parallel.stage_passed(1));
    assert_eq!(parallel.current_stage(), Some(2));
    assert_eq!(parallel.stage_ready(2).len(), 1, "阶段二才轮到 c");
}

/// 序号**由核心按阶段派生**：n1-1 / n1-2（同阶段并行）→ n2-1（下一阶段），依赖整体重映射。
#[test]
pub(crate) fn chain_ids_are_derived_from_stages() {
    let mut chain = crate::capabilities::taskchain::api::TaskChain {
        nodes: vec![
            chain_node("x", &[]),
            chain_node("y", &[]),
            chain_node("z", &["x", "y"]),
        ],
    };
    chain.renumber_by_stage();
    let ids: Vec<&str> = chain.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids, vec!["n1-1", "n1-2", "n2-1"]);
    assert_eq!(chain.nodes[2].deps, vec!["n1-1", "n1-2"], "依赖整体重映射");
    assert_eq!(chain.current_stage(), Some(1));
    assert!(!chain.stage_passed(1), "还没验收就不算通过");
}

/// 装配期自洽：环、悬空依赖、重复 id、空目标、未知负责人——逐条如实列出。
#[test]
pub(crate) fn chain_problems_reject_cycles_and_bad_refs() {
    use crate::capabilities::taskchain::api::TaskChain;
    let good = TaskChain {
        nodes: vec![chain_node("a", &[]), chain_node("b", &["a"])],
    };
    assert!(
        good.problems(&roster()).is_empty(),
        "{:?}",
        good.problems(&roster())
    );

    // 环：a 等 b、b 等 a。
    let cyc = TaskChain {
        nodes: vec![chain_node("a", &["b"]), chain_node("b", &["a"])],
    };
    let p = cyc.problems(&roster());
    assert!(p.iter().any(|x| x.contains("环")), "{:?}", p);

    // 悬空依赖 + 重复 id + 空目标 + 未知负责人。
    let mut bad = TaskChain {
        nodes: vec![chain_node("a", &["没有这个"]), chain_node("a", &[])],
    };
    bad.nodes[1].objective = String::new();
    bad.nodes[1].assignee = "丙".to_string();
    let p = bad.problems(&roster());
    assert!(p.iter().any(|x| x.contains("不存在的节点")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("id 重复")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("没有目标")), "{:?}", p);
    assert!(p.iter().any(|x| x.contains("不在名单里")), "{:?}", p);

    // 空链也算装配错误（没什么可推进的）。
    assert!(!TaskChain::default().problems(&roster()).is_empty());
}

/// 结束判定：链非空、且每个节点都落定（Done / Failed）——空链不算结束。
#[test]
pub(crate) fn chain_finished_needs_every_node_settled() {
    use crate::capabilities::taskchain::api::{NodeStatus, TaskChain};
    let mut chain = TaskChain {
        nodes: vec![chain_node("a", &[]), chain_node("b", &["a"])],
    };
    assert!(!chain.finished());
    chain.nodes[0].status = NodeStatus::Done;
    assert!(!chain.finished(), "b 还没落定");
    // 失败也算落定（链不静默跳过：会暂停并通知用户，但不会永远卡着）。
    chain.nodes[1].status = NodeStatus::Failed;
    assert!(chain.finished());
    assert!(!TaskChain::default().finished(), "空链不算结束");
}
