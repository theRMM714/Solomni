//! 目的：**裁决队列**——一条通道的排队与等待：核心各关卡与工具级确认**共用同一条队**。
//! 管：按会话先来后到排队、只有队首可见可答、回答的校验（卡号是队首 + 选项属于那张卡）、
//!   整队作废（停止 = 拒绝）把等待方解开、卡号的发放与续号；对外只出两个窄端口 `Decisions` 与 `Gates`。
//! 不管：问什么（发起方给消息与选项）、回答怎么处置（发起方）、界面怎么画（呈现层）。
//! 联动：卡片形状与选项 id 在 `crate::capabilities::conversation::session::domain::events`；契约见
//!   docs/session/session-model.md 的「请用户裁决：一条通道，消息 + 选项」；
//!   由核心持有（`src/capabilities/runtime/service.rs` 的 desk），生成线程与回答口都经 `session::api` 的端口。

use super::events::{DecisionAnswer, DecisionQueue, DecisionWaiter, Gate, SessionEvent, idle};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

/// 目的：等待格被解开的方式——回答、用户停止（整队作废）、还是这一趟根本没有回答者（会话按转录重建、前端断了）。
/// 约束：`Stopped` 与 `Gone` 必须分开——停止是用户的动作（= 拒绝），**不能被发起方的默认项吞成放行**。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SlotWake {
    /// 用户 / 前端答了一个选项 id。
    Answer(String),
    /// 用户按了停止或会话被关闭：整队作废（= 拒绝）。
    Stopped,
    /// 没人答：这一趟没有可回答的前端（等待格被重建解开、前端断了）。
    Gone,
}

/// 目的：等待格的内部状态——**一把锁管全部**：写进来的选项，与"为什么解开"的两个标记。
/// 约束：读写都必须在同一把锁下。旧写法把两个标记放在锁外（`AtomicBool`），而 `wait` 是在持锁时
///   检查它们的——于是"解开"落在"检查完、还没进 `cv.wait`"那一小段里就会**丢掉唤醒**，
///   等待线程永远挂着（用户已经答了 / 已经按了停止，工作线程却再也不动）。机器一有负载就现形。
#[derive(Default)]
struct SlotState {
    option: Option<String>,
    released: bool,
    /// 本次解开是不是"用户停止 / 会话关闭"（整队作废）——只有它不是默认项的适用场景。
    stopped: bool,
}

/// 目的：一次**等在工作线程上**的裁决的等待格：发起方在这里等，回答写进来并唤醒它。
/// 约束：不设超时（不点不继续）；整队作废与重建都会解开它，但**为什么解开**如实分开（见 `SlotWake`）。
#[derive(Default)]
pub struct AnswerSlot {
    state: Mutex<SlotState>,
    cv: Condvar,
}

impl AnswerSlot {
    pub fn new() -> Arc<AnswerSlot> {
        Arc::new(AnswerSlot::default())
    }

    /// 目的：等用户的回答——**阻塞，不设超时**。
    /// 返回：回答的选项 id，或"为什么没有答案"（停止 / 没人答）。
    pub fn wait(&self) -> SlotWake {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if let Some(v) = g.option.clone() {
                return SlotWake::Answer(v);
            }
            if g.released {
                return if g.stopped {
                    SlotWake::Stopped
                } else {
                    SlotWake::Gone
                };
            }
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn resolve(&self, option: &str) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        g.option = Some(option.to_string());
        self.cv.notify_all();
    }

    /// 目的：把这一格解开。`stopped` = 是用户停止 / 会话关闭（整队作废）解开的，而不是"没人答"。
    /// 约束：置位必须在**同一把锁**下（等待方是持锁检查这些标记的，解锁后直接进 `cv.wait`）——
    ///   锁外置位就是丢唤醒：那一小段里置的位没有任何人在等它，等待方醒来前永远不会再检查。
    fn release(&self, stopped: bool) {
        let mut g = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if stopped {
            g.stopped = true;
        }
        g.released = true;
        self.cv.notify_all();
    }
}

/// 目的：一张**已经出队**的回答交给发起方处置时带的东西（选项 + 附言 + 这一关的材料）。
/// 约束：这一关自带机制名与载荷（含那条建议），队列不解释它——解释归发起方。
pub struct GateTicket {
    pub option: String,
    pub note: String,
    /// 目的：这一关的材料，装箱（它比票本身大一个量级，票随 Answered 传一手不该整块搬）。
    pub gate: Box<Gate>,
}

/// 目的：一次回答落在哪一种关上（处置归发起方）。
pub enum Answered {
    /// 工具级确认这类**等在工作线程上**的关：回答已写进等待格，发起方会被唤醒。
    Waiting(Vec<SessionEvent>),
    /// 核心各关卡：处置票 + 要外送的事件（那条回答 + 队首换人后的新卡）。
    Gate(GateTicket, Vec<SessionEvent>),
}

/// 队列里的一格：卡号 + 这一关（机制名、载荷、卡、附言规则）；`slot` 有值 = 这一关等在工作线程上。
struct Entry {
    id: String,
    gate: Gate,
    slot: Option<Arc<AnswerSlot>>,
}

/// 目的：一个会话的**裁决通道**（入站面）：发起方挂卡与等答、回答口按队首认卡；队列本体与校验在实现里。
/// 约束：别的能力只拿这个端口，拿不到带锁的队列结构（R16）；等答用的等待格在实现里现造、不外传。
pub trait Gates: Send + Sync {
    /// 目的：挂一张卡（**不阻塞**）。
    /// 参数：`id` = 沿用的原卡号（重建时那张没人答的卡用它自己的号；`None` = 新发一个）。
    /// 返回：卡号与要外送的事件（推的与快照同源）。
    fn raise(&self, id: Option<&str>, gate: Gate) -> (String, Vec<SessionEvent>);

    /// 目的：挂一张卡并**阻塞等一个回答**（工具级确认这类等在工作线程上的关）。
    /// 参数：`emit` = 卡片事件的出口——**先外送再等**，界面才看得到那一问。
    /// 返回：回答的选项 id，或"为什么没有答案"（用户停止 / 没人答）。
    /// 约束：不设超时；整队作废（停止 = 拒绝）与重建都会解开它，两种原因如实分开。
    fn ask(&self, gate: Gate, emit: &mut dyn FnMut(SessionEvent)) -> SlotWake;

    /// 目的：回答**队首那张**卡：校验卡号与选项，写回答并出队。
    /// 参数：`note` = 附言（进那条回答记录；怎么用由发起方定）。
    /// 返回：这一答落在哪一种关上（等在工作线程上的那一关已被唤醒）。
    /// 错误：没有挂起、卡号不是队首、选项不属于那张卡——一律如实拒绝且**不留痕**。
    fn answer(&self, card: &str, option: &str, note: &str) -> Result<Answered, String>;

    /// 目的：当前那一队裁决（队首卡 + 后面还在等的几张）。
    /// 返回：没挂起 = `None`。
    fn queue(&self) -> Option<DecisionQueue>;

    /// 目的：快照形态（会话视图里的 `pending`）。
    /// 返回：没挂起 = `None`；有挂起时与推的那条卡片事件是同一份事实。
    fn json(&self) -> Option<serde_json::Value>;

    /// 目的：队首那一关（卡号 + 这一关）。
    /// 返回：没挂起 = `None`。
    fn head(&self) -> Option<(String, Gate)>;

    /// 目的：队列空不空（空 = 没挂起的事）。
    fn is_empty(&self) -> bool;

    /// 目的：把队首**原样出队**（「继续」那条路重派节点用）。
    /// 约束：不记回答、不唤醒任何人。
    fn drop_head(&self);

    /// 目的：整队作废（用户按停止 / 会话被关闭 = 拒绝），等待方一律解开。
    /// 返回：作废的卡号（本来就没挂着 = 空）。
    fn void(&self) -> Vec<String>;

    /// 目的：按转录**重建整队**：清空这一队并续上卡号。
    /// 参数：`issued` = 转录里已经发出过的卡数（新队从这里续号）。
    fn rebuild(&self, issued: u64);
}

#[derive(Default)]
struct Inner {
    /// 队列本体（先来后到）：队首是本会话唯一可见、可答的那张。
    queue: VecDeque<Entry>,
    /// 已推给界面的队列形态（队首卡号 + 后面等待的卡号）：形态变了才再推一条。
    announced: Option<(String, Vec<String>)>,
    /// 已发出的卡数：新卡号 = issued + 1（会话内唯一；续号见 `DecisionDesk::session`）。
    issued: u64,
}

/// 目的：一个会话的裁决队——先来后到，只有队首那张可见、可答；回答的校验只此一处。
#[derive(Default)]
pub struct DecisionDoor {
    inner: Mutex<Inner>,
}

impl DecisionDoor {
    /// 目的：新挂起一件事：进**队尾**，返回它的卡号与要外送的事件。
    /// 参数：`id` = 沿用的原卡号（重建时那张没人答的卡用它自己的号；`None` = 新发一个）；
    ///   `slot` = 这一关是否等在工作线程上。
    /// 约束：新卡号在这一处分配（卡内 id 以这里为准）；等在工作线程上的那一关**不推空闲**（生成还在跑，只是在等这一答）。
    pub fn push(
        &self,
        id: Option<&str>,
        gate: Gate,
        slot: Option<Arc<AnswerSlot>>,
    ) -> (String, Vec<SessionEvent>) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let id = match id {
            Some(id) => {
                note_card(Some(id), &mut g.issued);
                id.to_string()
            }
            None => {
                g.issued += 1;
                format!("d{}", g.issued)
            }
        };
        let blocking = slot.is_some();
        g.queue.push_back(Entry {
            id: id.clone(),
            gate,
            slot,
        });
        let mut evs = if blocking { Vec::new() } else { vec![idle()] };
        evs.extend(announce(&mut g));
        (id, evs)
    }

    /// 目的：队列空不空（空 = 没挂起的事）。
    pub fn is_empty(&self) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .is_empty()
    }

    /// 目的：队首那一关（卡号 + 这一关）；没挂起 = `None`。
    pub fn head(&self) -> Option<(String, Gate)> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.queue.front().map(|e| (e.id.clone(), e.gate.clone()))
    }

    /// 目的：当前那一队裁决（队首卡 + 后面还在等的几张）；没挂起 = `None`。
    pub fn queue(&self) -> Option<DecisionQueue> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let head = g.queue.front()?;
        Some(DecisionQueue {
            card: head.gate.card_with(&head.id),
            waiting: waiters(&g),
        })
    }

    /// 目的：快照形态（会话视图里的 `pending`）——与推的那条卡片事件是**同一份事实**。
    pub fn json(&self) -> Option<serde_json::Value> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let head = g.queue.front()?;
        Some(head.gate.to_json(&head.id, &waiters(&g)))
    }

    /// 目的：回答**队首那张**卡：校验卡号与选项，写回答并出队。
    /// 参数：`note` = 附言（进那条回答记录；怎么用由发起方定）。
    /// 返回：这一答落在哪一种关上（等在工作线程上的那一关已被唤醒）。
    /// 错误：没有挂起、卡号不是队首、选项不属于那张卡——一律如实拒绝且**不留痕**（不出队、不记回答）。
    pub fn answer(&self, card: &str, option: &str, note: &str) -> Result<Answered, String> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let Some(head) = g.queue.front() else {
            return Err("[裁决] 现在没有等你定的事。".to_string());
        };
        if head.id != card {
            return Err(format!(
                "这张卡已经不是当前那张了（现在等的是 {}）；请按界面上的卡片作答。",
                head.id
            ));
        }
        if !head.gate.card_with(&head.id).has_option(option) {
            return Err(format!("这张卡上没有这个选项：{}", option));
        }
        // 附言要求也是这一关的声明（发起方给）：出队**之前**校验——被拒的回答不留痕。
        if let Some(why) = head.gate.note_requirement(option)
            && note.trim().is_empty()
        {
            return Err(why);
        }
        let note = note.trim().to_string();
        let blocking = head.slot.clone();
        let entry = g.queue.pop_front().expect("队首刚刚取过");
        g.announced = None;
        let mut evs = vec![SessionEvent::DecisionAnswer(DecisionAnswer {
            card: entry.id.clone(),
            by: "用户".to_string(),
            option: option.to_string(),
            note: note.clone(),
        })];
        evs.extend(announce(&mut g));
        match blocking {
            Some(slot) => {
                slot.resolve(option);
                Ok(Answered::Waiting(evs))
            }
            None => Ok(Answered::Gate(
                GateTicket {
                    option: option.to_string(),
                    note,
                    gate: Box::new(entry.gate),
                },
                evs,
            )),
        }
    }

    /// 目的：按转录**重建整队**：清空这一队并续上卡号（重建的判据是转录，不是内存里那一份）。
    /// 约束：只在"按转录重建这个会话对象"时用——生成中的会话不重建（它不在核心表里）。
    pub fn rebuild(&self, issued: u64) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        for e in g.queue.iter() {
            if let Some(s) = &e.slot {
                // 重建 = 这一趟没有回答者（不是用户停止）：按"没人答"解开。
                s.release(false);
            }
        }
        g.queue.clear();
        g.announced = None;
        g.issued = g.issued.max(issued);
    }

    /// 目的：把队首**原样出队**（「继续」那条路重派节点用）：不记回答、不唤醒任何人。
    pub fn drop_head(&self) {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.queue.pop_front();
        g.announced = None;
    }

    /// 目的：整队作废（用户按停止 / 会话被关闭 = 拒绝）：等待方一律解开。
    /// 返回：作废的卡号（本来就没挂着 = 空）。
    pub fn void(&self) -> Vec<String> {
        let mut g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let cards: Vec<String> = g.queue.iter().map(|e| e.id.clone()).collect();
        for e in g.queue.iter() {
            if let Some(s) = &e.slot {
                // 整队作废 = 用户停止 / 会话关闭：按"停止 = 拒绝"解开（不套用默认项）。
                s.release(true);
            }
        }
        g.queue.clear();
        g.announced = None;
        cards
    }
}

impl Gates for DecisionDoor {
    fn raise(&self, id: Option<&str>, gate: Gate) -> (String, Vec<SessionEvent>) {
        self.push(id, gate, None)
    }

    fn ask(&self, gate: Gate, emit: &mut dyn FnMut(SessionEvent)) -> SlotWake {
        // **先登记再推卡**：否则用户手快会答在一张还没进队的卡上，答案丢掉、发起方干等。
        let slot = AnswerSlot::new();
        let (_, evs) = self.push(None, gate, Some(Arc::clone(&slot)));
        for e in evs {
            emit(e);
        }
        slot.wait()
    }

    fn answer(&self, card: &str, option: &str, note: &str) -> Result<Answered, String> {
        DecisionDoor::answer(self, card, option, note)
    }

    fn queue(&self) -> Option<DecisionQueue> {
        DecisionDoor::queue(self)
    }

    fn json(&self) -> Option<serde_json::Value> {
        DecisionDoor::json(self)
    }

    fn head(&self) -> Option<(String, Gate)> {
        DecisionDoor::head(self)
    }

    fn is_empty(&self) -> bool {
        DecisionDoor::is_empty(self)
    }

    fn drop_head(&self) {
        DecisionDoor::drop_head(self)
    }

    fn void(&self) -> Vec<String> {
        DecisionDoor::void(self)
    }

    fn rebuild(&self, issued: u64) {
        DecisionDoor::rebuild(self, issued)
    }
}

/// 目的：把**当前队列形态**变成要外送的事件（队首卡 + 后面还在等的那几张）：形态变了才给。
/// 约束：推的与快照的是同一份（`Gate::event`），不做第二真相。
fn announce(g: &mut Inner) -> Vec<SessionEvent> {
    let Some(head) = g.queue.front() else {
        g.announced = None;
        return Vec::new();
    };
    let shape = (
        head.id.clone(),
        g.queue
            .iter()
            .skip(1)
            .map(|e| e.id.clone())
            .collect::<Vec<String>>(),
    );
    if g.announced.as_ref() == Some(&shape) {
        return Vec::new();
    }
    let ev = head.gate.event(&head.id, &waiters(g));
    g.announced = Some(shape);
    vec![ev]
}

/// 目的：队首之后还在等的那几张（谁在等、问的什么 + 各自的重建材料）。
fn waiters(g: &Inner) -> Vec<DecisionWaiter> {
    g.queue
        .iter()
        .skip(1)
        .map(|e| e.gate.waiter(&e.id))
        .collect()
}

/// 目的：全会话的裁决队（按会话 id 取，入站面）：核心持有一份，生成线程与回答口取到的是同一份。
/// 约束：别的能力只拿这个端口，取到的是会话通道 `Gates`；带锁的队列结构不出这个单元（R16）。
pub trait Decisions: Send + Sync {
    /// 目的：取这个会话的裁决通道；还没有就按 `issued` 建一个。
    /// 参数：`issued` = 转录里**已经发出过的卡数**（新队从这里续号，跨重启不撞号）；已建则不动。
    fn gate(&self, sid: &str, issued: u64) -> Arc<dyn Gates>;

    /// 目的：只看不动（快照与回答口用）。
    /// 返回：这个会话还没有队 = `None`。
    fn peek_gate(&self, sid: &str) -> Option<Arc<dyn Gates>>;

    /// 目的：会话没了，它的队随会话一起消失（等待方与转录都不复存在）。
    fn forget_gate(&self, sid: &str);
}

/// 目的：造一份全会话的裁决队（核心持有一份；跨能力只经 `session::api` 取）。
pub fn decisions() -> Arc<dyn Decisions> {
    DecisionDesk::new()
}

/// 目的：全会话的裁决队（按会话 id 取）：核心持有，生成线程与回答口拿到的是同一份。
#[derive(Default)]
pub struct DecisionDesk {
    doors: Mutex<HashMap<String, Arc<DecisionDoor>>>,
}

impl DecisionDesk {
    pub fn new() -> Arc<DecisionDesk> {
        Arc::new(DecisionDesk::default())
    }

    /// 目的：取这个会话的裁决队；还没有就按 `issued` 建一个。
    /// 参数：`issued` = 转录里**已经发出过的卡数**（新队从这里续号，跨重启不撞号）；已建则不动。
    pub fn session(&self, sid: &str, issued: u64) -> Arc<DecisionDoor> {
        let mut g = self.doors.lock().unwrap_or_else(|e| e.into_inner());
        Arc::clone(g.entry(sid.to_string()).or_insert_with(|| {
            Arc::new(DecisionDoor {
                inner: Mutex::new(Inner {
                    issued,
                    ..Inner::default()
                }),
            })
        }))
    }

    /// 目的：只看不动（快照与回答口用）：这个会话还没有队 = `None`。
    pub fn peek(&self, sid: &str) -> Option<Arc<DecisionDoor>> {
        self.doors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(sid)
            .cloned()
    }

    /// 目的：会话没了，它的队随会话一起消失（等待方与转录都不复存在）。
    pub fn forget(&self, sid: &str) {
        self.doors
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(sid);
    }
}

impl Decisions for DecisionDesk {
    fn gate(&self, sid: &str, issued: u64) -> Arc<dyn Gates> {
        self.session(sid, issued)
    }

    fn peek_gate(&self, sid: &str) -> Option<Arc<dyn Gates>> {
        self.peek(sid).map(|d| d as Arc<dyn Gates>)
    }

    fn forget_gate(&self, sid: &str) {
        self.forget(sid);
    }
}

/// 目的：一份转录里**已经发出过的卡数**（按卡号的最大序号，判据就是转录里的卡号本身）：新一张从这里续号。
pub fn issued_in(events: &[serde_json::Value]) -> u64 {
    let mut max = 0u64;
    for ev in events {
        match ev.get("type").and_then(|t| t.as_str()) {
            Some("decision_card") => {
                note_card(ev.get("id").and_then(|v| v.as_str()), &mut max);
                if let Some(list) = ev.get("waiting").and_then(|w| w.as_array()) {
                    for w in list {
                        note_card(w.get("id").and_then(|v| v.as_str()), &mut max);
                    }
                }
            }
            Some("decision_void") => {
                if let Some(list) = ev.get("cards").and_then(|c| c.as_array()) {
                    for id in list {
                        note_card(id.as_str(), &mut max);
                    }
                }
            }
            _ => {}
        }
    }
    max
}

/// 目的：把一个卡号并进"已发出的最大序号"（认不出序号的不算）。
fn note_card(id: Option<&str>, max: &mut u64) {
    let Some(n) = id
        .and_then(|s| s.strip_prefix('d'))
        .and_then(|s| s.parse::<u64>().ok())
    else {
        return;
    };
    if n > *max {
        *max = n;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    /// 目的：回答写进格子之后，等待**无论是在它之前还是之后**开始的，都要立刻拿到 Answer。
    #[test]
    fn an_answer_is_seen_whether_the_waiter_started_before_or_after_it() {
        let before = AnswerSlot::new();
        before.resolve("allow");
        assert_eq!(before.wait(), SlotWake::Answer("allow".to_string()));

        let after = AnswerSlot::new();
        let w = {
            let s = Arc::clone(&after);
            std::thread::spawn(move || s.wait())
        };
        std::thread::sleep(Duration::from_millis(20));
        after.resolve("deny");
        assert_eq!(
            w.join().expect("等待线程"),
            SlotWake::Answer("deny".to_string()),
            "等待中被回答要能被唤醒"
        );
    }

    /// 目的：`release` 的**原因**如实分开（停止 ≠ 没人答）——释放在等待之前与之后都成立。
    #[test]
    fn the_release_reason_stays_distinct_in_both_orders() {
        let stopped = AnswerSlot::new();
        stopped.release(true);
        assert_eq!(stopped.wait(), SlotWake::Stopped, "先释放在先也是停止");

        let gone = AnswerSlot::new();
        gone.release(false);
        assert_eq!(gone.wait(), SlotWake::Gone, "先释放在先是没人答");

        let blocking = AnswerSlot::new();
        let h = {
            let s = Arc::clone(&blocking);
            std::thread::spawn(move || s.wait())
        };
        std::thread::sleep(Duration::from_millis(20));
        blocking.release(true);
        assert_eq!(h.join().expect("等待线程"), SlotWake::Stopped);
    }

    /// 目的：**释放在"检查完、还没进挂起"那一小段里也不能丢唤醒**。
    ///   用"刚起线程就释放"的大量轮次去撞那个窗口：旧写法（标记位在锁外置位）撞上就永远挂着，
    ///   所以每次等待都带超时——挂住即失败，不会把测试套件吊死。
    #[test]
    fn a_release_racing_the_waiter_never_leaves_it_blocked() {
        for i in 0..200 {
            let slot = AnswerSlot::new();
            let (tx, rx) = mpsc::channel();
            let s = Arc::clone(&slot);
            let h = std::thread::spawn(move || {
                tx.send(s.wait()).ok();
            });
            let stopped = i % 2 == 0;
            slot.release(stopped);
            let got = rx
                .recv_timeout(Duration::from_secs(2))
                .expect("释放之后等待方必须被解开（丢唤醒就会一直挂着）");
            let want = if stopped {
                SlotWake::Stopped
            } else {
                SlotWake::Gone
            };
            assert_eq!(got, want, "第 {} 轮：原因也要如实", i);
            h.join().expect("等待线程");
        }
    }

    /// 目的：同一个窗口对"回答"也一样——回答写进格子后，等待方必须立刻拿到。
    #[test]
    fn an_answer_racing_the_waiter_never_leaves_it_blocked() {
        for i in 0..200 {
            let slot = AnswerSlot::new();
            let (tx, rx) = mpsc::channel();
            let s = Arc::clone(&slot);
            let h = std::thread::spawn(move || {
                tx.send(s.wait()).ok();
            });
            slot.resolve("allow");
            let got = rx
                .recv_timeout(Duration::from_secs(2))
                .expect("回答之后等待方必须被解开");
            assert_eq!(got, SlotWake::Answer("allow".to_string()), "第 {} 轮", i);
            h.join().expect("等待线程");
        }
    }
}
