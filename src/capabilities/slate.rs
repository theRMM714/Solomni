//! 目的：拟名单能力（协调型）——给一句需求，产出可直接开工的 agent 名单。
//! 管：slate 工具载荷（picks，形状 = registry::api::RosterPick）与 slate.* 提示词段；一次性推荐与协作会话代拟两种用例。
//! 不管：端口与状态（名单是在飞的值）；待确认名单归 collab、呈现归前端；参与方事实由调用方传入。
//! 联动：调用方 conductor（ConductorOps::suggest_models）与 collab（代拟）共用同一条协议；用例在 service.rs，纯规则在 domain/。

pub mod api;
pub mod domain;
pub mod service;
