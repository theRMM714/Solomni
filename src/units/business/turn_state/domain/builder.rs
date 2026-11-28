//! 目的：流程实现的注册面——每一段流程由声明里的实现名（`builder`）在注册表里查到构造器，装配点不认识任何具体流程。
//! 管：`FlowBuilder`（一个实现：名字 + 造一段）、`BuildRequest`（造一段要的现场）、`FlowBuilders`（注册表，同时实现 `Flows`）。
//! 不管：任何一段流程的内部；会话形状的声明（`systools/loops.yaml` / `systools/profiles.yaml`）；回合驱动（在 `turn`）。
//! 联动：组合根把各单元的实现登记进来；`sequence_of` / `turn` / `work` 只经 `Flows` 取段，不认任何实现名。

use std::sync::Arc;

use crate::units::business::session_model::api::{SessionDecls, SessionMeta};
use crate::units::business::turn_state::domain::flow::Flow;
use crate::units::business::turn_state::domain::flows::{FlowSetup, Flows};

/// 目的：造一段流程要的现场：段 id、这一段的成员身份与最少成员数、共用现场、事件窗口。
/// 约束：身份与最少成员数由声明算好再交进来——各实现不自己查表，也不各自解释「谁是成员」。
pub struct BuildRequest<'a> {
    /// 目的：段 id（实现可以拿它查自己的段声明，也可以只用来报错）。
    pub seg: &'a str,
    /// 目的：这一段的成员身份（段 id, 角色 id）；没有成员角色的段为 `None`。
    pub identity: Option<(String, String)>,
    /// 目的：这一段开跑至少要几个成员（按角色位的 `min` 求和）。
    pub min_members: usize,
    /// 目的：造一段的共用现场（登记处快照、名册、沙箱、裁决通道）。
    pub setup: &'a FlowSetup,
    /// 目的：这一段的**事件窗口**：`Some` = 按它恢复（`None` = 新建这一段）。
    pub restore: Option<(&'a SessionMeta, &'a [serde_json::Value])>,
}

/// 目的：一种流程实现的注册项：声明里的 `builder` 名与「按现场造一段」。
/// 约束：实现由组合根登记，注册表不认识任何具体类型；名字是声明与实现之间唯一的键。
pub trait FlowBuilder: Send + Sync {
    /// 目的：这一实现的名字（与 `systools/loops.yaml` 的 `builder` 逐字相同）。
    fn name(&self) -> &str;
    /// 目的：按现场造一段。
    /// 错误：现场缺这一段必需的东西（例如新建却没有核心通道）时如实报错。
    fn build(&self, req: &BuildRequest<'_>) -> Result<Box<dyn Flow>, String>;
}

/// 目的：流程实现注册表：按段声明的实现名查构造器；同时是注入给 `turn` / `work` 的 `Flows`。
/// 约束：注册项在一处登记（组合根）；新增一种实现只加实现文件、注册一行与 `loops.yaml` 一条，本类型与装配点都不动。
pub struct FlowBuilders {
    decls: Arc<SessionDecls>,
    list: Vec<Arc<dyn FlowBuilder>>,
}

impl FlowBuilders {
    /// 目的：装好注册表：会话形状的声明表 + 各单元交来的实现。
    /// 参数：`decls` = 声明聚合（段表在这里查）；`list` = 注册项（顺序无关，按名字查）。
    pub fn new(decls: Arc<SessionDecls>, list: Vec<Arc<dyn FlowBuilder>>) -> FlowBuilders {
        FlowBuilders { decls, list }
    }

    /// 目的：按实现名取注册项（内部查表）。
    fn get(&self, name: &str) -> Option<&Arc<dyn FlowBuilder>> {
        self.list.iter().find(|b| b.name() == name)
    }
}

impl Flows for FlowBuilders {
    fn decls(&self) -> &SessionDecls {
        &self.decls
    }

    fn build(
        &self,
        id: &str,
        setup: &FlowSetup,
        restore: Option<(&SessionMeta, &[serde_json::Value])>,
    ) -> Result<Box<dyn Flow>, String> {
        // 段 id 与实现名的对应关系只在数据里：本类型只做两次查表。
        let loops = self.decls.loops();
        let seg = loops
            .of(id)
            .ok_or_else(|| format!("流程没在册：{}（systools/loops.yaml）", id))?;
        let name = seg
            .builder
            .as_deref()
            .ok_or_else(|| format!("流程段 {} 没声明实现名（systools/loops.yaml）", id))?;
        let builder = self.get(name).ok_or_else(|| {
            format!(
                "没有注册这个流程实现：{}（流程段 {}；systools/loops.yaml）",
                name, id
            )
        })?;
        let req = BuildRequest {
            seg: id,
            identity: loops
                .member_role(id)
                .map(|role| (id.to_string(), role.to_string())),
            min_members: loops.min_members(id),
            setup,
            restore,
        };
        builder.build(&req)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    use crate::kernel::api::types::Ask;
    use crate::units::business::agentloop::api::MemberTurn;
    use crate::units::business::catalog::registry::api::Settings;
    use crate::units::business::session_model::api::{
        AskDesk, Cast, Kind, Loop, Loops, Profiles, Role,
    };
    use crate::units::business::transcript::api::SessionEvent;
    use crate::units::business::turn_state::domain::flow::{Handback, MemberAsk};
    use crate::units::business::workbench::api::Benches;

    struct Stand;

    impl Flow for Stand {
        fn set_cancel(&mut self, _cancel: Arc<AtomicBool>) {}
        fn start(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn resume(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn step(&mut self, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn take_ask(&mut self) -> Option<MemberAsk> {
            None
        }
        fn handback(&mut self, _ask: &MemberAsk, _turn: &MemberTurn) -> Handback {
            Handback::Fed
        }
        fn feed(
            &mut self,
            _ask: &MemberAsk,
            _turn: MemberTurn,
            _sink: &mut dyn FnMut(SessionEvent),
        ) {
        }
        fn pass_over(&mut self, _ask: &MemberAsk, _sink: &mut dyn FnMut(SessionEvent)) {}
        fn failed(&mut self, _err: &str) {}
        fn next_line(&self) -> u64 {
            0
        }
        fn set_next_line(&mut self, _next: u64) {}
    }

    struct StandBuilder;

    impl FlowBuilder for StandBuilder {
        fn name(&self) -> &str {
            "stand"
        }
        fn build(&self, req: &BuildRequest<'_>) -> Result<Box<dyn Flow>, String> {
            // 注册项按名字查到即算装配成功：这里只把段 id 带出来，证明声明确实传到了实现手上。
            assert_eq!(req.seg, "solo");
            Ok(Box::new(Stand))
        }
    }

    struct NoDesk;

    impl AskDesk for NoDesk {
        fn later(&self, card: Option<&str>, _ask: &Ask) -> (String, Vec<SessionEvent>) {
            (card.unwrap_or("c1").to_string(), Vec::new())
        }
        fn pending(&self) -> bool {
            false
        }
        fn head(&self) -> Option<(String, Ask)> {
            None
        }
        fn drop_head(&self) {}
    }

    fn decls(builder: &str) -> SessionDecls {
        SessionDecls::new(
            Loops {
                roles: vec![
                    Role {
                        id: "core".to_string(),
                        prompt: "roles/core".to_string(),
                        core: true,
                    },
                    Role {
                        id: "member".to_string(),
                        prompt: "roles/member".to_string(),
                        core: false,
                    },
                ],
                loops: vec![Loop {
                    id: "solo".to_string(),
                    builder: Some(builder.to_string()),
                    cast: vec![
                        Cast {
                            role: "core".to_string(),
                            min: 0,
                            tools: Vec::new(),
                        },
                        Cast {
                            role: "member".to_string(),
                            min: 2,
                            tools: Vec::new(),
                        },
                    ],
                    deny: Vec::new(),
                    ask: Vec::new(),
                    accepts: vec![Kind::Requirement],
                    produces: None,
                }],
            },
            Profiles::default(),
        )
    }

    fn setup() -> FlowSetup {
        FlowSetup {
            settings: Settings::default(),
            core_channel: None,
            roster: Vec::new(),
            delegated: false,
            sandboxes: Benches::default(),
            ask: Arc::new(NoDesk),
        }
    }

    /// 目的：声明里写一个名字、注册表里有同名的实现，装配点就能造出它——不认识任何具体类型。
    #[test]
    fn a_registered_implementation_is_found_by_the_name_in_the_table() {
        let reg = FlowBuilders::new(Arc::new(decls("stand")), vec![Arc::new(StandBuilder)]);
        let flow = <FlowBuilders as Flows>::build(&reg, "solo", &setup(), None).unwrap();
        assert_eq!(flow.next_line(), 0);
    }

    /// 目的：声明里的实现名没有注册时如实报错，并点名段与实现名。
    #[test]
    fn an_implementation_nobody_registered_is_reported_by_name() {
        let reg = FlowBuilders::new(Arc::new(decls("nobody")), vec![Arc::new(StandBuilder)]);
        let err = match <FlowBuilders as Flows>::build(&reg, "solo", &setup(), None) {
            Ok(_) => panic!("实现名没有注册，不该造出流程"),
            Err(e) => e,
        };
        assert!(err.contains("没有注册这个流程实现：nobody"), "{}", err);
        assert!(err.contains("流程段 solo"), "{}", err);
    }

    /// 目的：成员身份与最少人数由声明算好交进实现（实现不自己查表）。
    #[test]
    fn the_request_carries_the_identity_and_the_minimum_from_the_table() {
        struct Probe;
        impl FlowBuilder for Probe {
            fn name(&self) -> &str {
                "stand"
            }
            fn build(&self, req: &BuildRequest<'_>) -> Result<Box<dyn Flow>, String> {
                assert_eq!(
                    req.identity,
                    Some(("solo".to_string(), "member".to_string()))
                );
                assert_eq!(req.min_members, 2);
                Ok(Box::new(Stand))
            }
        }
        let reg = FlowBuilders::new(Arc::new(decls("stand")), vec![Arc::new(Probe)]);
        <FlowBuilders as Flows>::build(&reg, "solo", &setup(), None).unwrap();
    }
}
