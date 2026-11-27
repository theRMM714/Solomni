//! 目的：转录的入站面——别的单元与呈现层只准用这里。
//! 管：重导出线格式与呈现词汇（事件、转录行、裁决卡与信封、上下文用量）与其构造函数。
//! 不管：这些值怎么产生、怎么落盘（会话单元与历史 detail）；怎么渲染（呈现层）。
//! 联动：会话、collab、turn、runtime、呈现层与测试都经这里取形状。

#[allow(unused_imports)] // 入站面：别的单元与呈现层都经这里取形状
pub use crate::units::business::transcript::domain::content::{
    ArchiveMark, Content, Entry, keep_whole_replies, line_reply_of,
};
#[allow(unused_imports)] // 入站面：别的单元与呈现层都经这里取形状
pub use crate::units::business::transcript::domain::events::{
    ContextUsage, DecisionAnswer, DecisionCard, DecisionEnvelope, DecisionMessage, DecisionQueue,
    DecisionWaiter, LineView, OptionView, SessionEvent, ToolCallView, card_of, event_of, idle,
    interrupted_note, json_of, note_of, qualify, stopped_note, waiter_of, working,
};
