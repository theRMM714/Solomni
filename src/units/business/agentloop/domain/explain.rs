//! 目的：工具面的可解释视图——每个 id 从哪个来源来、这一段准不准、面上有没有总表不认识的 id。
//! 管：ExplainInput / ExplainReport 的形状与纯函数 explain（与装配同源：同一份来源、同一份策略判定）。
//! 不管：取数（在 runtime 的 explain_tools 动作）；打印（在呈现层）；来源与策略各自的语义（在 sources / policy）。
//! 联动：装配用 assemble 的 offers_of / merge_offers，判定用 policy::verdict_of，两处都不许另写一套。

use std::collections::BTreeMap;

use serde::Serialize;

use crate::units::business::agentloop::domain::assemble::{
    merge_offers, offers_of, service_ops_of, serviced_tools,
};
use crate::units::business::agentloop::domain::member_tools::{
    ModuleTools, service_ops_for_modules, tool_table,
};
use crate::units::business::agentloop::domain::policy::{ToolRequest, Verdict, verdict_of};
use crate::units::business::agentloop::domain::sources::build_sources;
use crate::units::business::catalog::modules::api::Module;
use crate::units::business::residents::api::ServiceView;
use crate::units::business::session_model::api::{
    Loops, SHARED_WRITE, SOURCE_MODULE, SOURCE_WORKSPACE, TurnCtx,
};
use crate::units::business::tools::api::ToolBook;

/// 目的：解释一席工具面要的全部形状事实（都由调用方从会话 meta 与各端口取来，本函数不碰 IO）。
pub struct ExplainInput<'a> {
    /// 目的：会话类型 id（只用于报告）。
    pub kind: &'a str,
    /// 目的：这一回合在哪个流程段。
    pub loop_id: &'a str,
    /// 目的：这一回合的角色 id。
    pub role: &'a str,
    /// 目的：当前回合是哪个 agent。
    pub agent: &'a str,
    /// 目的：这个 agent 的能力组。
    pub abilities: &'a [String],
    /// 目的：会话类型声明的来源 id（挂哪些来源）。
    pub source_ids: &'a [String],
    /// 目的：会话类型展开进会话的 system 工具 id。
    pub system_tools: &'a [String],
    /// 目的：工作名；没有工作区为空。
    pub workspace_name: Option<&'a str>,
    /// 目的：这个 agent 装的模块。
    pub modules: &'a [Module],
    /// 目的：常驻服务视图（只有已就绪的服务才有操作可交）。
    pub services: &'a [ServiceView],
    /// 目的：段与角色的声明表。
    pub loops: &'a Loops,
    /// 目的：工具总表（判「面里的 id 有没有形状」）。
    pub book: &'a ToolBook,
}

/// 目的：一个来源这一刻交出的工具 id。
#[derive(Debug, Clone, Serialize)]
pub struct SourceOffer {
    /// 目的：来源 id（agent / module / loop / workspace / profile）。
    pub source: String,
    /// 目的：它交出的 id（顺序即它在面里的顺序）。
    pub tools: Vec<String>,
}

/// 目的：面里一个 id 的授权判定与来源归属。
#[derive(Debug, Clone, Serialize)]
pub struct ToolLine {
    /// 目的：工具 id。
    pub tool: String,
    /// 目的：判定三档之一：allow / ask / deny。
    pub verdict: String,
    /// 目的：为什么是这个判定（抑制它的是哪一段的哪条能力组、或为什么不用问）。
    pub why: String,
    /// 目的：把这个 id 交出来的来源（同一个 id 可以由多个来源交出）。
    pub from: Vec<String>,
}

/// 目的：一次工具面解释的完整报告。
#[derive(Debug, Clone, Serialize)]
pub struct ExplainReport {
    /// 目的：会话类型 id。
    pub kind: String,
    /// 目的：流程段 id。
    pub loop_id: String,
    /// 目的：角色 id。
    pub role: String,
    /// 目的：agent 实例名。
    pub agent: String,
    /// 目的：工作名（没有工作区为空）。
    pub workspace: Option<String>,
    /// 目的：能力组（这个 agent 带着什么来的）。
    pub abilities: Vec<String>,
    /// 目的：这一段抑制的能力组（Loop.deny）。
    pub deny: Vec<String>,
    /// 目的：这一段要用户点头的动作 id（Loop.ask）。
    pub ask: Vec<String>,
    /// 目的：每个来源交了什么（只列交了东西的来源）。
    pub sources: Vec<SourceOffer>,
    /// 目的：合并后的工具面（去重、按来源顺序）。
    pub face: Vec<String>,
    /// 目的：逐 id 的判定与来源。
    pub tools: Vec<ToolLine>,
    /// 目的：面里但工具总表与模块工具表都没有的 id（来源交了形状，执行侧会当未知工具拒）。
    pub orphans: Vec<String>,
    /// 目的：这次解释发现的不一致（段没在册、角色不在段里、来源 id 不在册等）。
    pub notes: Vec<String>,
}

/// 目的：按形状事实重算一席工具面，并逐 id 给出判定与来源（与真实装配同源，不另写判定）。
/// 参数：input = 会话形状事实（见 ExplainInput）。
/// 返回：报告（来源、面、逐 id 判定、孤儿、不一致）。
pub fn explain(input: ExplainInput<'_>) -> ExplainReport {
    let ctx = TurnCtx {
        loop_id: Some(input.loop_id).filter(|id| !id.is_empty()),
        agent_id: input.agent,
        abilities: input.abilities,
        profile: input.kind,
        workspace: input.workspace_name,
    };
    let with_modules = input.source_ids.iter().any(|id| id == SOURCE_MODULE);
    let mut table = tool_table(input.modules);
    if with_modules {
        let started = service_ops_for_modules(service_ops_of(input.services), input.modules);
        serviced_tools(&started, &mut table);
    }
    let loop_tools = input
        .loops
        .cast_of(input.loop_id, input.role)
        .map(|cast| cast.tools.clone())
        .unwrap_or_default();
    let sources = build_sources(input.source_ids, &table, input.system_tools, &loop_tools);
    let offers = offers_of(&ctx, &sources);
    let (face, _) = merge_offers(&offers);
    let (deny, ask) = match input.loops.of(input.loop_id) {
        Some(seg) => (seg.deny.clone(), seg.ask.clone()),
        None => (Vec::new(), Vec::new()),
    };
    let mut notes: Vec<String> = Vec::new();
    if input.loops.of(input.loop_id).is_none() {
        notes.push(format!(
            "流程段 {} 不在册（systools/loops.yaml）：段来源交空面，也不抑制任何能力",
            input.loop_id
        ));
    } else if input.loops.cast_of(input.loop_id, input.role).is_none() {
        notes.push(format!(
            "角色 {} 不在这一段（{}）的角色位里：段来源交空面",
            input.role, input.loop_id
        ));
    }
    for id in input.source_ids {
        if !is_known_source(id) {
            notes.push(format!(
                "来源 id 不在册：{}（会话类型声明了它，装配时会被跳过）",
                id
            ));
        }
    }
    if with_modules && table.is_empty() {
        notes.push(format!(
            "agent {} 没有装任何模块：模块来源交空面",
            input.agent
        ));
    }
    if input.source_ids.iter().any(|s| s == SOURCE_WORKSPACE) {
        if input.workspace_name.is_none() {
            notes.push("这个会话没有工作区：共享区工具一个也不发".to_string());
        } else if !input.abilities.iter().any(|a| a == SHARED_WRITE) {
            notes.push(format!(
                "agent {} 没有 {} 能力：共享区工具一个也不发",
                input.agent, SHARED_WRITE
            ));
        }
    }
    let mut tools: Vec<ToolLine> = Vec::new();
    let mut orphans: Vec<String> = Vec::new();
    for id in &face {
        let from: Vec<String> = offers
            .iter()
            .filter(|(_, ids)| ids.iter().any(|t| t == id))
            .map(|(source, _)| source.clone())
            .collect();
        let req = ToolRequest {
            tool: id,
            sources: &from,
            loop_id: input.loop_id,
            role: input.role,
        };
        let (verdict, why) = match verdict_of(input.loop_id, &deny, &ask, &req) {
            Verdict::Allow => (
                "allow",
                format!(
                    "这一段（{}）不抑制它，也没有把它列进要点头的动作",
                    input.loop_id
                ),
            ),
            Verdict::Ask { why } => ("ask", why),
            Verdict::Deny(why) => ("deny", why),
        };
        if input.book.get(id).is_none() && !is_module_tool(&table, id) {
            orphans.push(id.clone());
        }
        tools.push(ToolLine {
            tool: id.clone(),
            verdict: verdict.to_string(),
            why,
            from,
        });
    }
    ExplainReport {
        kind: input.kind.to_string(),
        loop_id: input.loop_id.to_string(),
        role: input.role.to_string(),
        agent: input.agent.to_string(),
        workspace: input.workspace_name.map(str::to_string),
        abilities: input.abilities.to_vec(),
        deny,
        ask,
        sources: offers
            .into_iter()
            .map(|(source, tools)| SourceOffer { source, tools })
            .collect(),
        face,
        tools,
        orphans,
        notes,
    }
}

/// 目的：来源 id 是不是五个在册来源之一（装配期已拦，这里给诊断用）。
fn is_known_source(id: &str) -> bool {
    use crate::units::business::session_model::api::{SOURCE_AGENT, SOURCE_LOOP, SOURCE_PROFILE};
    matches!(
        id,
        SOURCE_AGENT | SOURCE_MODULE | SOURCE_LOOP | SOURCE_WORKSPACE | SOURCE_PROFILE
    )
}

/// 目的：一个限定 id（模块.工具 或 模块.服务.操作）在不在这一席的模块工具表里。
/// 参数：table = 模块工具表；id = 面里的限定 id。
/// 返回：有形状为 true；模块没装或工具没声明为 false。
fn is_module_tool(table: &BTreeMap<String, ModuleTools>, id: &str) -> bool {
    let Some((module, name)) = id.split_once('.') else {
        return false;
    };
    match table.get(module) {
        Some(mt) => mt.commands.contains_key(name) || mt.services.contains_key(name),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::units::business::session_model::api::{
        Cast, FS_READ, FS_WRITE, Loop, Role, SOURCE_AGENT, SOURCE_LOOP,
    };

    fn declared() -> Loops {
        Loops {
            roles: vec![
                Role {
                    id: "core".to_string(),
                    prompt: "roles/core".to_string(),
                    core: true,
                },
                Role {
                    id: "worker".to_string(),
                    prompt: "roles/executor".to_string(),
                    core: false,
                },
            ],
            loops: vec![Loop {
                id: "talk".to_string(),
                builder: Some("discuss".to_string()),
                cast: vec![
                    Cast {
                        role: "core".to_string(),
                        min: 0,
                        tools: Vec::new(),
                    },
                    Cast {
                        role: "worker".to_string(),
                        min: 1,
                        tools: vec!["say".to_string()],
                    },
                ],
                deny: vec![FS_WRITE.to_string()],
                ask: Vec::new(),
                accepts: Vec::new(),
                produces: None,
            }],
        }
    }

    fn book() -> ToolBook {
        let text = [
            "read: {desc: 读}",
            "write: {desc: 写}",
            "delete: {desc: 删}",
            "list: {desc: 列}",
            "grep: {desc: 搜}",
            "edit: {desc: 改}",
            "patch: {desc: 补}",
            "say: {desc: 说}",
        ]
        .join("\n");
        yaml_serde::from_str(&text).expect("工具表要解析得了")
    }

    fn abilities() -> Vec<String> {
        vec![FS_READ.to_string(), FS_WRITE.to_string()]
    }

    fn shape<'a>(
        loops: &'a Loops,
        book: &'a ToolBook,
        abilities: &'a [String],
        sources: &'a [String],
    ) -> ExplainInput<'a> {
        ExplainInput {
            kind: "single",
            loop_id: "talk",
            role: "worker",
            agent: "alice",
            abilities,
            source_ids: sources,
            system_tools: &[],
            workspace_name: None,
            modules: &[],
            services: &[],
            loops,
            book,
        }
    }

    #[test]
    fn every_id_says_which_source_handed_it_out() {
        let (loops, book) = (declared(), book());
        let abilities = abilities();
        let sources = [SOURCE_AGENT.to_string(), SOURCE_LOOP.to_string()];
        let report = explain(shape(&loops, &book, &abilities, &sources));
        let read = report
            .tools
            .iter()
            .find(|t| t.tool == "read")
            .expect("读面在面里");
        assert_eq!(read.from, vec![SOURCE_AGENT.to_string()]);
        let say = report
            .tools
            .iter()
            .find(|t| t.tool == "say")
            .expect("段声明的动作在面里");
        assert_eq!(say.from, vec![SOURCE_LOOP.to_string()]);
        assert!(report.orphans.is_empty(), "{:?}", report.orphans);
    }

    #[test]
    fn a_suppressed_ability_shows_up_as_deny_with_its_group() {
        let (loops, book) = (declared(), book());
        let abilities = abilities();
        let sources = [SOURCE_AGENT.to_string()];
        let report = explain(shape(&loops, &book, &abilities, &sources));
        let write = report
            .tools
            .iter()
            .find(|t| t.tool == "write")
            .expect("写面在面里");
        assert_eq!(write.verdict, "deny");
        assert!(write.why.contains("fs-write"), "{}", write.why);
        let delete = report
            .tools
            .iter()
            .find(|t| t.tool == "delete")
            .expect("写面在面里");
        assert_eq!(delete.verdict, "deny");
    }

    #[test]
    fn an_id_without_a_shape_is_reported_as_an_orphan() {
        let mut loops = declared();
        loops.loops[0].cast[1].tools = vec!["say".to_string(), "ghost_tool".to_string()];
        let book = book();
        let abilities = abilities();
        let sources = [SOURCE_LOOP.to_string()];
        let report = explain(shape(&loops, &book, &abilities, &sources));
        assert_eq!(report.orphans, vec!["ghost_tool".to_string()]);
        assert!(report.face.contains(&"ghost_tool".to_string()));
    }

    #[test]
    fn a_segment_outside_the_table_is_called_out() {
        let (loops, book) = (declared(), book());
        let abilities = abilities();
        let sources = [SOURCE_LOOP.to_string()];
        let mut input = shape(&loops, &book, &abilities, &sources);
        input.loop_id = "nowhere";
        let report = explain(input);
        assert!(report.face.is_empty());
        assert!(
            report.notes.iter().any(|n| n.contains("不在册")),
            "{:?}",
            report.notes
        );
    }

    #[test]
    fn a_workspace_without_the_shared_write_ability_offers_nothing() {
        let (loops, book) = (declared(), book());
        let abilities = abilities();
        let sources = [crate::units::business::session_model::api::SOURCE_WORKSPACE.to_string()];
        let mut input = shape(&loops, &book, &abilities, &sources);
        input.workspace_name = Some("dev");
        let report = explain(input);
        assert!(report.face.is_empty());
        assert!(
            report.notes.iter().any(|n| n.contains("shared-write")),
            "{:?}",
            report.notes
        );
    }
}
