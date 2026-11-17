//! 目的：工作形态的词汇：三个取值、与外部字符串的互转、落盘形态。
//! 管：`Mode` 的定义、`as_str` / `parse` / `Display`，以及落盘时的小写形态。
//! 不管：形态对应的装配差异（在 `assemble.rs`）；哪种形态允许几个 agent（在 `work` 的校验里）。
//! 联动：`history.rs` 把它记进 `SessionMeta`；`work` 的 `WorkMode` 是它的别名，`proxy` 用它解析入参。

use serde::{Deserialize, Serialize};

/// 目的：工作形态：单 agent（模块数不限）/ 协作（多 agent 分权协商）/ 代理（决定权整块交给核心）。
///   形态只用于校验与界面标签：会话实现只有「单 agent」与「协作」两种——
///   代理会话在实现上就是一个单会话，只是身份换成 `core_proxy` 角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Single,
    Collab,
    /// 目的：代理：**没有名单**；用户选这一形态就是在授予全权。
    Proxy,
}

impl Mode {
    /// 目的：落盘与界面用的词。
    /// 返回：`single` / `collab` / `proxy`。
    pub fn as_str(self) -> &'static str {
        match self {
            Mode::Single => "single",
            Mode::Collab => "collab",
            Mode::Proxy => "proxy",
        }
    }

    /// 目的：把外部输入（命令行、界面入参、动作参数里的 mode）解析成形态。
    /// 错误：不是这三个词时返回「mode 只能是 single / collab / proxy：<原值>」。
    pub fn parse(text: &str) -> Result<Mode, String> {
        match text.trim() {
            "single" => Ok(Mode::Single),
            "collab" => Ok(Mode::Collab),
            "proxy" => Ok(Mode::Proxy),
            other => Err(format!("mode 只能是 single / collab / proxy：{other}")),
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mode_round_trips_through_its_word() {
        for m in [Mode::Single, Mode::Collab, Mode::Proxy] {
            assert_eq!(Mode::parse(m.as_str()), Ok(m), "词与形态要一一对应");
            assert_eq!(m.to_string(), m.as_str());
        }
    }

    #[test]
    fn parse_trims_and_rejects_unknown_words() {
        assert_eq!(Mode::parse(" collab "), Ok(Mode::Collab));
        assert!(Mode::parse("").is_err(), "空串不是形态");
        assert!(Mode::parse("Single").is_err(), "形态是小写词");
        assert_eq!(
            Mode::parse("all").unwrap_err(),
            "mode 只能是 single / collab / proxy：all",
            "报错要点名入参"
        );
    }
}
