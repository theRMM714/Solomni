//! 目的：执行入口与虚拟机档的测试脚手架——逐成员跑一遍、按档位起执行。
//! 管：`ExecLike` 的共享入口、`run_execution`、`vm_spec` 的档选型。
//! 不管：通道（`chat.rs`）、进程 runner（`runners.rs`）。
//! 联动：`src/capabilities/session/service/engine/round.rs` 的 `converse_with` 与 `src/capabilities/vm/api.rs`。

use super::*;

/// 目的：测试用的"逐成员跑一遍"：内部走**共享的 converse_with**（工具循环本身仍在生产里， 只是执行阶段的入口从平铺的 Execution::run 换成了链驱动）。
pub(crate) struct ExecLike {
    pub reports: BTreeMap<String, String>,
    pub traces: BTreeMap<String, Vec<crate::capabilities::session::api::ToolCallView>>,
    /// 目的：每个成员的各回复原文（每条回复一份）。
    pub raws: BTreeMap<String, Vec<String>>,
}

/// 目的：执行入口的席位：成员本身（不带工具面）或测试席位（成员 + 工具环境）。
pub(crate) trait Seat {
    fn parts_mut(
        &mut self,
    ) -> (
        &mut crate::capabilities::collab::service::discussion::Member,
        Option<&mut MemberTools>,
    );
}

impl Seat for crate::capabilities::collab::service::discussion::Member {
    fn parts_mut(
        &mut self,
    ) -> (
        &mut crate::capabilities::collab::service::discussion::Member,
        Option<&mut MemberTools>,
    ) {
        (self, None)
    }
}

impl Seat for WithTools {
    fn parts_mut(
        &mut self,
    ) -> (
        &mut crate::capabilities::collab::service::discussion::Member,
        Option<&mut MemberTools>,
    ) {
        (&mut self.member, self.tools.as_mut())
    }
}

pub(crate) fn run_execution<S: Seat>(
    members: &mut [S],
    tasks: &str,
    prompt: &dyn Prompt,
) -> ExecLike {
    let mut out = ExecLike {
        reports: BTreeMap::new(),
        traces: BTreeMap::new(),
        raws: BTreeMap::new(),
    };
    for m in members.iter_mut() {
        let (member, tools) = m.parts_mut();
        if !member.present {
            continue;
        }
        let identity = member.params.identity(prompt, member.mode);
        let id = member.id.clone();
        let user = prompt.render(
            Segment::ExecuteUser,
            &[("tasks", tasks.to_string()), ("rework", String::new())],
        );
        let mut views = Vec::new();
        let mut noop = |_c: crate::capabilities::llm::api::Chunk| true;
        let mut sink = |_e: crate::capabilities::session::api::SessionEvent| {};
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rounds = crate::capabilities::session::service::engine::round::converse_with(
            member.chat.as_mut().expect("测试通道").as_mut(),
            tools,
            &identity,
            vec![crate::capabilities::llm::api::Msg::user(user)],
            Default::default(),
            &id,
            &cancel,
            &mut noop,
            &mut |v: &crate::capabilities::session::api::ToolCallView| views.push(v.clone()),
            &mut |_r: &crate::capabilities::session::service::engine::round::Round,
                  _s: &mut dyn FnMut(crate::capabilities::session::api::SessionEvent)| {},
            &mut sink,
            // 测试不接工具级确认：直接执行（确认路径由 permission 的用例单独钉）。
            None,
            // 也不接工具层提问：默认路径就是"没有可回答的前端 = fail-closed 拒绝"。
            None,
            &[],
            false,
        );
        let text = rounds.last().map(|r| r.text.clone()).unwrap_or_default();
        out.reports.insert(id.clone(), text);
        out.traces.insert(id.clone(), views);
        out.raws.insert(
            id,
            rounds
                .iter()
                .map(|r| r.raw.clone())
                .filter(|r| !r.is_empty())
                .collect(),
        );
    }
    out
}

/// 目的：虚拟机档选型：基础根 + 不联网（定版留空 = 让库自己决定；多版本时报歧义）。
pub(crate) fn vm_spec() -> ExecSpec {
    ExecSpec {
        tier: Tier::Vm,
        guest: Some(crate::tests::test_guest("vm-spec")),
        pins: BTreeMap::new(),
    }
}
