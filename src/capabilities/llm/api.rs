//! 入站能力面：**其它能力与呈现层只准用这里**（不许碰 `domain` / `ports`）。

pub use crate::capabilities::llm::domain::envelope::{
    parse, Malformed, Reply, Tail, ToolInvoke, Verb,
};
pub use crate::capabilities::llm::domain::malformed::malformed_report;
pub use crate::capabilities::llm::ports::{
    truncated, BoxedChat, Channel, Chat, ChatGateway, Chunk, CompleteOpts, Completion,
    EnvelopeRepair, LlmOpts, ModelCatalog, Msg, ProbeOutcome, RepairOutcome, ReplayReport,
    ReplayShape, ToolCall, ToolDecl, ToolMode,
};
