//! 目的：会话历史生命周期的用例面——历史列表、只读打开、删除收尾与回档（留档 / 删除 / 还原）。
//! 管：回档三模式的行 id 与标记顺序、子会话折叠、删除的报账与围栏撤权顺序。
//! 不管：行与事件算术（session 的 domain）、落盘机制（session 的 HistoryStore 端口）、协作链推进（runtime）。
//! 联动：runtime 经 `api` 把用例时机交给这里；`Turn` / `RebuildDeps` 都是方法形参、不是字段。

pub mod api;
pub(crate) mod service;
