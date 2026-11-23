//! 目的：流程面——会话流程的插件契约：驱动只认「推一步 / 让出一回合 / 交回」这三件事，流程自己决定下一步问谁。
//! 管：`Flow`（驱动一侧的接口）、`MemberAsk`（让出的那一步）与 `Handback`（交回后的处置）。
//! 不管：流程内部的状态机；会话表、落盘与呈现（分别归 turn 的会话表、session 的历史与 presentation）。
//! 联动：`turn` 的驱动循环与 `turn::generation` 的工作线程按这套接口推进任何流程；协作的实现见 `collab`。

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::capabilities::catalog::registry::api::AgentMeta;
use crate::capabilities::conversation::session::api::{Gate, GateTicket, MemberTurn, SessionEvent};
use crate::capabilities::llm::api::{CompleteOpts, Msg};
use crate::capabilities::tools::api::Tools;

/// 目的：已注册的流程 id：`systools/profiles.yaml` 只能引用这里的名字（新增流程在这里加一行）。
pub const REGISTERED: &[&str] = &["discuss", "execute"];

/// 目的：流程让出的那一步——请驱动取某个成员会话跑一回合。
/// 约束：流程**不自己调模型**（它拿不到别的会话）；这里只交出「问谁、带什么、按什么参数」。
pub struct MemberAsk {
    /// 目的：该成员在流程名单里的下标（交回结果时按它认人）。
    pub index: usize,
    /// 目的：成员 agent 名——它的会话名是「<工作>--<成员>」。
    pub member: String,
    /// 目的：本回合的身份块（流程决定它怎么理解这一步）。
    pub identity: String,
    /// 目的：本回合要喂给模型的上下文。
    pub turn: Vec<Msg>,
    /// 目的：整场工作的回合号（与会话的回合标记同一套）。
    pub turn_id: u64,
    /// 目的：流程的轮次序号（流程自己解释，驱动只透传）。
    pub round: usize,
    /// 目的：本回合的工具面（由流程按角色发放）。
    pub systools: Arc<dyn Tools>,
    /// 目的：本回合的调用参数（流式与预算）。
    pub opts: CompleteOpts<'static>,
}

impl MemberAsk {
    /// 目的：这个成员自己的会话名（讨论与执行共用一个会话）。
    pub fn child_of(&self, sid: &str) -> String {
        format!("{}--{}", sid, self.member)
    }
}

/// 目的：成员回合交回后流程的处置——驱动照它做，不替流程判定。
pub enum Handback {
    /// 目的：吸收这一回合，继续往下推。
    Fed,
    /// 目的：这一回合不算数：把这句话注入它的会话后重问同一个人。
    Remind(String),
    /// 目的：提醒到顶：记一行未回应，放过这个人，本轮继续。
    PassOver,
}

/// 目的：一张裁决票处置完的结果：驱动照它决定还要不要接着跑流水线。
pub struct Settle {
    /// 目的：这一关放行了（开工 / 重派），驱动要接着跑整条流水线。
    pub resume: bool,
    /// 目的：这一答刚定下的名单（写回 meta 并建沙箱）；`None` = 名单没变。
    pub roster: Option<Vec<AgentMeta>>,
}

/// 目的：流程——会话流程的插件面：驱动只按这套动作推进，不再认识任何具体流程的字段。
/// 约束：同一会话同一时刻只属于一个流程；进入 / 退出由落盘转录派生，不是内存标志。
pub trait Flow: Send {
    /// 目的：把「停止」接到流程上（生成线程登记后交给它）。
    fn set_cancel(&mut self, cancel: Arc<AtomicBool>);
    /// 目的：开场：还没开始过就先开始（驱动循环的第一步）。
    fn start(&mut self, sink: &mut dyn FnMut(SessionEvent));
    /// 目的：继续：从断点推进（用户点「继续」走这条，不需要用户发言）。
    fn resume(&mut self, sink: &mut dyn FnMut(SessionEvent));
    /// 目的：推进一步（开场、喂回、放过之后都走这条）。
    fn step(&mut self, sink: &mut dyn FnMut(SessionEvent));
    /// 目的：取走这一步让出的事；`None` = 没有要问的人，驱动收工。
    fn take_ask(&mut self) -> Option<MemberAsk>;
    /// 目的：成员回合交回后的处置：吸收继续、提醒重问、还是放过（策略在流程里）。
    /// 约束：只借看这一回合——真正的吸收在随后的 `feed`（回合本体归 `feed` 消费）。
    fn handback(&mut self, ask: &MemberAsk, turn: &MemberTurn) -> Handback;
    /// 目的：吸收这一回合（`Handback::Fed` 之后驱动调用）。
    fn feed(&mut self, ask: &MemberAsk, turn: MemberTurn, sink: &mut dyn FnMut(SessionEvent));
    /// 目的：放过这个人、本轮继续（`Handback::PassOver` 之后驱动调用）。
    fn pass_over(&mut self, ask: &MemberAsk, sink: &mut dyn FnMut(SessionEvent));
    /// 目的：成员回合失败：如实记下，下一步推一步时由流程交回中断（不静默吞掉）。
    fn failed(&mut self, err: &str);
    /// 目的：流程自己的业务面（任务链、名单、沙箱）取回具体类型；驱动路径不用它。
    fn as_any(&self) -> &dyn Any;
    /// 目的：同上（可写一份）。
    fn as_any_mut(&mut self) -> &mut dyn Any;
    /// 目的：这一段的业务面：单段流程是自己，复合流程是**当前这一段**（工作视图与派活按它取类型）。
    fn part(&self) -> &dyn Any {
        self.as_any()
    }
    /// 目的：同上（可写一份）。
    fn part_mut(&mut self) -> &mut dyn Any {
        self.as_any_mut()
    }
    /// 目的：本流程转录的下一个行号（界面续号与回档重放都读它）。
    fn next_line(&self) -> u64;
    /// 目的：回档后重设行号（行号只增不退，值是全量最大 id + 1）。
    fn set_next_line(&mut self, next: u64);
    /// 目的：会话名册落定（用户确认代拟的名单之后由驱动通知）：默认什么都不做，要成员出演的流程据此记下谁能上。
    /// 约束：名册是会话级事实（落 meta.agents）；复合流程要转发给每一段，否则后一段不知道有谁。
    fn adopt_roster(&mut self, roster: &[AgentMeta]) {
        let _ = roster;
    }
    /// 目的：上一段流程交来的产物（复合流程的第一段收到的是用户需求）。
    fn intake(&mut self, text: String) {
        let _ = text;
    }
    /// 目的：会话开场把用户需求交给流程（第一段收下它；默认只是当成本段输入）。
    /// 约束：一段流程只开场一次；需求由用户那条消息给出，不是流程自己编的。
    fn begin_with(&mut self, task: &str, sink: &mut dyn FnMut(SessionEvent)) {
        let _ = sink;
        self.intake(task.to_string());
    }
    /// 目的：本段流程的产物（交给下一段；`None` = 没交出东西）。
    fn output(&self) -> Option<String> {
        None
    }
    /// 目的：流程自己判定整场工作已收尾（界面据此显示完成）。
    fn is_done(&self) -> bool {
        false
    }
    /// 目的：把一条内容类系统提示记成流程自己的行（协作记成带行号的转录行）。
    fn notice(&mut self, text: &str, sink: &mut dyn FnMut(SessionEvent)) {
        sink(SessionEvent::Notice(text.to_string()));
    }
    /// 目的：处置一张已经出队的裁决票（这一关是哪一种由流程解释）：返回驱动接着要做什么。
    /// 错误：票不是本流程发的、选项不在这一关能分派的范围里，都如实拒绝。
    fn settle(
        &mut self,
        ticket: &GateTicket,
        sink: &mut dyn FnMut(SessionEvent),
    ) -> Result<Settle, String> {
        let _ = sink;
        Err(format!("这张卡不是本流程发的：{}", ticket.gate.kind))
    }
    /// 目的：这一关要不要**就地收尾**（在核心线程上同步写完再放行，不等工作线程）。
    fn settles_inline(&self, gate: &Gate) -> bool {
        let _ = gate;
        false
    }
}
