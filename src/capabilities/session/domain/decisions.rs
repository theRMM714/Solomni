//! 目的：**裁决队列**——一条通道的排队与等待：核心各关卡与工具级确认**共用同一条队**。
//! 管：按会话先来后到排队、只有队首可见可答、回答的校验（卡号是队首 + 选项属于那张卡）、
//!   整队作废（停止 = 拒绝）把等待方解开、卡号的发放与续号。
//! 不管：问什么（发起方给消息与选项）、回答怎么处置（发起方）、界面怎么画（呈现层）。
//! 联动：卡片形状与选项 id 在 `crate::capabilities::session::domain::events`；契约见
//!   docs/session/session-model.md 的「请用户裁决：一条通道，消息 + 选项」；
//!   由核心持有（`src/capabilities/conductor/service/mod.rs` 的 desk），生成线程与回答口都经它。

use super::events::{idle, DecisionAnswer, DecisionQueue, DecisionWaiter, Pending, SessionEvent};
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex};

/// 目的：一次**等在工作线程上**的裁决的等待格：发起方在这里等，回答写进来并唤醒它。
/// 约束：不设超时（不点不继续）；整队作废时解开，`wait` 给 `None`（发起方按「停止 = 拒绝」处置）。
#[derive(Default)]
pub struct AnswerSlot {
    option: Mutex<Option<String>>,
    cv: Condvar,
    released: std::sync::atomic::AtomicBool,
}

impl AnswerSlot {
    pub fn new() -> Arc<AnswerSlot> {
        Arc::new(AnswerSlot::default())
    }

    /// 目的：等用户的回答——**阻塞，不设超时**。
    /// 返回：选中的选项 id；整队作废 / 停止解开时 `None`（发起方按拒绝处置）。
    pub fn wait(&self) -> Option<String> {
        let mut g = self.option.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if self.released.load(std::sync::atomic::Ordering::Relaxed) {
                return None;
            }
            if let Some(v) = g.clone() {
                return Some(v);
            }
            g = self.cv.wait(g).unwrap_or_else(|e| e.into_inner());
        }
    }

    fn resolve(&self, option: &str) {
        let mut g = self.option.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(option.to_string());
        self.cv.notify_all();
    }

    fn release(&self) {
        self.released
            .store(true, std::sync::atomic::Ordering::Relaxed);
        self.cv.notify_all();
    }
}

/// 目的：一张**已经出队**的回答交给发起方处置时带的东西（卡号 + 选项 + 附言 + 这一关的材料）。
pub struct GateTicket {
    pub option: String,
    pub note: String,
    pub pending: Pending,
    pub advice: String,
}

/// 目的：一次回答落在哪一种关上（处置归发起方）。
pub enum Answered {
    /// 工具级确认这类**等在工作线程上**的关：回答已写进等待格，发起方会被唤醒。
    Waiting(Vec<SessionEvent>),
    /// 核心各关卡：处置票 + 要外送的事件（那条回答 + 队首换人后的新卡）。
    Gate(GateTicket, Vec<SessionEvent>),
}

/// 队列里的一格：卡号 + 这一关的机制材料 + 发起方的建议；`slot` 有值 = 这一关等在工作线程上。
struct Entry {
    id: String,
    pending: Pending,
    advice: String,
    slot: Option<Arc<AnswerSlot>>,
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
    ///   `advice` = 发起方的建议（工具级确认为空串）；`slot` = 这一关是否等在工作线程上。
    /// 约束：新卡号在这一处分配；等在工作线程上的那一关**不推空闲**（生成还在跑，只是在等这一答）。
    pub fn push(
        &self,
        id: Option<&str>,
        pending: Pending,
        advice: &str,
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
            pending,
            advice: advice.to_string(),
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

    /// 目的：队首那一关（卡号 + 关卡 + 建议）；没挂起 = `None`。
    pub fn head(&self) -> Option<(String, Pending, String)> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        g.queue
            .front()
            .map(|e| (e.id.clone(), e.pending.clone(), e.advice.clone()))
    }

    /// 目的：当前那一队裁决（队首卡 + 后面还在等的几张）；没挂起 = `None`。
    pub fn queue(&self) -> Option<DecisionQueue> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let head = g.queue.front()?;
        Some(DecisionQueue {
            card: head.pending.card(&head.id, &head.advice),
            waiting: waiters(&g),
        })
    }

    /// 目的：快照形态（会话视图里的 `pending`）——与推的那条卡片事件是**同一份事实**。
    pub fn json(&self) -> Option<serde_json::Value> {
        let g = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let head = g.queue.front()?;
        Some(head.pending.to_json(&head.id, &head.advice, &waiters(&g)))
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
        if !head.pending.card(&head.id, &head.advice).has_option(option) {
            return Err(format!("这张卡上没有这个选项：{}", option));
        }
        // 附言要求也是这一关的声明（发起方给）：出队**之前**校验——被拒的回答不留痕。
        if let Some(why) = head.pending.note_requirement(option) {
            if note.trim().is_empty() {
                return Err(why);
            }
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
                    pending: entry.pending,
                    advice: entry.advice,
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
                s.release();
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
                s.release();
            }
        }
        g.queue.clear();
        g.announced = None;
        cards
    }
}

/// 目的：把**当前队列形态**变成要外送的事件（队首卡 + 后面还在等的那几张）：形态变了才给。
/// 约束：推的与快照的是同一份（`Pending::event`），不做第二真相。
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
    let ev = head.pending.event(&head.id, &head.advice, &waiters(g));
    g.announced = Some(shape);
    vec![ev]
}

/// 目的：队首之后还在等的那几张（谁在等、问的什么 + 各自的重建材料）。
fn waiters(g: &Inner) -> Vec<DecisionWaiter> {
    g.queue
        .iter()
        .skip(1)
        .map(|e| e.pending.waiter(&e.id, &e.advice))
        .collect()
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
