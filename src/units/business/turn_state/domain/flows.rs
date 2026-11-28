//! 目的：流程工厂面——装配期按会话类型表里的流程 id 造出一段流程（新增流程只在工厂里加一条映射）。
//! 管：`Flows`（id → 造一段）与 `FlowSetup`（造一段要的现场）；`sequence_of` 把若干段串成一条并按进度事实只造起跑点之后的段。
//! 不管：任何一段流程的内部；会话形状的声明（在 `systools/loops.yaml` 与 `systools/profiles.yaml`）；回合驱动（在 `turn`）。
//! 联动：组合根把实现注入 `turn` 与 `work`；装配点只按 id 取段，不认识任何具体流程的类型。

use std::sync::Arc;

use crate::units::business::catalog::registry::api::Settings;
use crate::units::business::llm::api::Channel;
use crate::units::business::session_model::api::{Artifact, AskDesk, SessionDecls, SessionMeta};
use crate::units::business::turn_state::domain::flow::Flow;
use crate::units::business::turn_state::domain::sequence::{Sequence, StepDecl};
use crate::units::business::turn_state::domain::stage;
use crate::units::business::workbench::api::Benches;
use crate::units::pure::registry_model::api::AgentMeta;

/// 目的：造一段流程要的现场（每一段按 id 自己取用，本段不用的部分不消费）。
pub struct FlowSetup {
    /// 目的：登记处快照（模型解析、工具形态与全局设置）。
    pub settings: Settings,
    /// 目的：核心模型通道：新建时要；从转录重建时按 `settings` 自己解出来，可不给。
    pub core_channel: Option<Channel>,
    /// 目的：本次工作的成员名单（代拟确认前为空）。
    pub roster: Vec<AgentMeta>,
    /// 目的：是否代拟（显式传入，不从名单是否为空推断）。
    pub delegated: bool,
    /// 目的：本工作的沙箱清单（流程按 agent 名取）。
    pub sandboxes: Benches,
    /// 目的：本会话的裁决通道**发起面**（各段共用同一条队的挂起口）。
    pub ask: Arc<dyn AskDesk>,
}

/// 目的：流程工厂：会话形状的声明 + 段 id → 造一段（实现在流程自己的单元里，由组合根注入）。
pub trait Flows: Send + Sync {
    /// 目的：会话形状的声明表——装配按它取某个类型的流程序列，工厂按它查段声明的实现名。
    fn decls(&self) -> &SessionDecls;
    /// 目的：按段 id 造一段：`restore` 是这一段的事件窗口（`None` = 新建这一段）。
    /// 约束：段 id 与实现名的对应关系只在数据里（`systools/loops.yaml` 的 `builder`），本端口不出现具体段 id 的分支。
    /// 错误：段没在册或没声明实现名、现场缺必需项（例如新建却没有核心通道）时如实报错。
    fn build(
        &self,
        id: &str,
        setup: &FlowSetup,
        restore: Option<(&SessionMeta, &[serde_json::Value])>,
    ) -> Result<Box<dyn Flow>, String>;
}

/// 目的：装配计划：从哪一段起跑、这一段之后的段声明、行号基线、切点交来的产物。
struct StagePlan {
    at: usize,
    from: usize,
    decls: Vec<StepDecl>,
    line: u64,
    handoff: Option<Artifact>,
}

/// 目的：算起跑点：进度事实说走到哪一段就从哪一段起（事实缺失或段表变了就从头），段声明按段表取。
/// 参数：`ids` = 声明的段序列；`loops` = 段表；`events` = 全量转录；`restoring` = 是不是重建。
/// 错误：事实指向的段、或序列里的段不在段表里，都如实报错（不半装配）。
fn plan_of(
    ids: &[String],
    loops: &crate::units::business::session_model::api::Loops,
    events: &[serde_json::Value],
    restoring: bool,
) -> Result<StagePlan, String> {
    if ids.is_empty() {
        return Err("这条会话类型的流程序列是空的（systools/profiles.yaml）".to_string());
    }
    // 进度事实说的是绝对段号；段表变了（事实指向的段不在了）就整条从头重推。
    let point = stage::resume(events);
    let point = if point.at < ids.len() {
        point
    } else {
        stage::Resume::start()
    };
    let mut decls: Vec<StepDecl> = Vec::with_capacity(ids.len() - point.at);
    for id in ids.iter().skip(point.at) {
        let seg = loops
            .of(id)
            .ok_or_else(|| format!("流程段没在册：{}（systools/loops.yaml）", id))?;
        decls.push(StepDecl {
            id: id.clone(),
            accepts: seg.accepts.clone(),
            produces: seg.produces,
        });
    }
    let line = if restoring {
        point.line.max(stage::base_line(events))
    } else {
        0
    };
    Ok(StagePlan {
        at: point.at,
        from: if restoring { point.from } else { 0 },
        decls,
        line,
        handoff: point.handoff,
    })
}

/// 目的：按会话类型声明的顺序造出整条流水线：只造起跑点之后的段，当前段拿切点之后的窗口。
/// 参数：`ids` = 该类型的流程序列（`order` 即执行顺序）；`setup` = 共用现场；`flows` = 工厂；
///   `restore` = 重建现场（`Some` 就按进度事实从中间某段起跑，`None` 就是新建整条）。
/// 错误：任何一段造不出来即整体失败（装配期失败就失败，不半装配）。
pub fn sequence_of(
    ids: &[String],
    setup: &FlowSetup,
    flows: &dyn Flows,
    restore: Option<(&SessionMeta, &[serde_json::Value])>,
) -> Result<Sequence, String> {
    let events: &[serde_json::Value] = restore.map_or(&[], |(_, ev)| ev);
    let plan = plan_of(ids, flows.decls().loops(), events, restore.is_some())?;
    let mut steps: Vec<Box<dyn Flow>> = Vec::with_capacity(plan.decls.len());
    for (i, id) in ids.iter().enumerate().skip(plan.at) {
        // 起跑点那一段拿切点之后的事件；它后面的段还没开始，给一份**空窗口**走恢复路（
        // 此时没有核心通道，也不该把前面那些段的事件塞给它们）。
        let window = match restore {
            Some((meta, ev)) if i == plan.at => Some((meta, &ev[plan.from.min(ev.len())..])),
            Some((meta, _)) => {
                let empty: &[serde_json::Value] = &[];
                Some((meta, empty))
            }
            None => None,
        };
        steps.push(flows.build(id, setup, window)?);
    }
    Ok(Sequence::resumed(
        steps,
        plan.decls,
        plan.at,
        plan.line,
        plan.handoff,
    ))
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::business::session_model::api::{Kind, Loop, Loops};

    fn table(rows: &[(&str, &[Kind], Option<Kind>)]) -> Loops {
        Loops {
            roles: Vec::new(),
            loops: rows
                .iter()
                .map(|(id, accepts, produces)| Loop {
                    id: id.to_string(),
                    builder: Some("discuss".to_string()),
                    cast: Vec::new(),
                    deny: Vec::new(),
                    ask: Vec::new(),
                    accepts: accepts.to_vec(),
                    produces: *produces,
                })
                .collect(),
        }
    }

    fn ids(names: &[&str]) -> Vec<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    /// 目的：同一实现名出现两次时，重启从进度事实点名的那一段起，只造它之后的段。
    #[test]
    fn a_restart_starts_at_the_segment_the_fact_names() {
        let loops = table(&[
            ("d1", &[Kind::Requirement], Some(Kind::Summary)),
            ("e1", &[Kind::Summary], None),
            ("d2", &[Kind::Summary], Some(Kind::Summary)),
        ]);
        let events = vec![
            serde_json::json!({ "type": "transcript", "lines": [{ "id": 1 }, { "id": 2 }] }),
            crate::units::business::turn_state::domain::stage::mark(
                1,
                3,
                Some(&Artifact::Summary("总结".to_string())),
            )
            .to_json(),
            serde_json::json!({ "type": "transcript", "lines": [{ "id": 4 }] }),
        ];
        let plan = plan_of(&ids(&["d1", "e1", "d2"]), &loops, &events, true).unwrap();
        assert_eq!(plan.at, 1);
        assert_eq!(plan.from, 2, "窗口从事实的下一条开始");
        assert_eq!(plan.decls.len(), 2, "前面那段不再造");
        assert_eq!(plan.decls[0].id, "e1");
        assert_eq!(plan.line, 5, "最大行 id + 1");
        assert_eq!(plan.handoff, Some(Artifact::Summary("总结".to_string())));
    }

    /// 目的：没有进度事实就从第一段起，行号仍按全量转录接下去。
    #[test]
    fn without_a_fact_the_run_starts_from_the_top() {
        let loops = table(&[
            ("d1", &[Kind::Requirement], Some(Kind::Summary)),
            ("e1", &[Kind::Summary], None),
        ]);
        let events = vec![serde_json::json!({ "type": "transcript", "lines": [{ "id": 7 }] })];
        let plan = plan_of(&ids(&["d1", "e1"]), &loops, &events, true).unwrap();
        assert_eq!(plan.at, 0);
        assert_eq!(plan.from, 0);
        assert_eq!(plan.decls.len(), 2);
        assert_eq!(plan.line, 8);
    }
}
