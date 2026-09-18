//! 呈现层：前端实现。CLI 与 Web 转录中心并列，共用同一套入站能力面与事件词汇。
//! 两者的「意图翻译」只有一份：见 intent.rs。
pub mod cli;
pub mod intent;
pub mod web;
