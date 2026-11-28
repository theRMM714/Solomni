//! 目的：流程之间的交接货币——一段流程交给下一段的产物，以及声明里写的产物种类。
//! 管：`Artifact`（带内容的产物，只收真实存在的两种）与 `Kind`（不带内容的种类，声明与装配期校验用）。
//! 不管：谁产出、谁接收（各段自己的 `intake` / `output`）；怎么串（`turn_state` 的 `Sequence`）。
//! 联动：`mechanism` 的 `Loop::accepts` / `produces` 按 `Kind` 声明；`turn_state` 的 `Flow` 用 `Artifact` 搬运。

/// 目的：交接货币的种类（不带内容）：声明「这一段收什么、交什么」时用。
/// 约束：闭合枚举——新增种类要动 core，这是有意的扩展（不是配置）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// 目的：用户需求（会话开场由用户那条消息给出）。
    Requirement,
    /// 目的：上一段流程交出的文本产物（例如讨论段交出的总结）。
    Summary,
}

impl Kind {
    /// 目的：报错文案里的名字（声明校验点名用）。
    pub fn name(&self) -> &'static str {
        match self {
            Kind::Requirement => "用户需求",
            Kind::Summary => "上一段的总结",
        }
    }

    /// 目的：线格式上的名字（进度事实与声明文件共用同一套词汇）。
    pub fn as_str(&self) -> &'static str {
        match self {
            Kind::Requirement => "requirement",
            Kind::Summary => "summary",
        }
    }

    /// 目的：从线格式回到种类；不认识的词返回 `None`（调用方如实拒绝）。
    pub fn parse(name: &str) -> Option<Kind> {
        match name {
            "requirement" => Some(Kind::Requirement),
            "summary" => Some(Kind::Summary),
            _ => None,
        }
    }
}

/// 目的：一段流程交给下一段的东西（带内容）。
/// 约束：组合子只搬运、不解释内容——照原样交给下一段，由接收方自己解释。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Artifact {
    /// 目的：用户需求。
    Requirement(String),
    /// 目的：一段流程的文本产物。
    Summary(String),
}

impl Artifact {
    /// 目的：这件产物是哪一种（推进时对照声明用；组合子只看种类，不看内容）。
    pub fn kind_of(&self) -> Kind {
        match self {
            Artifact::Requirement(_) => Kind::Requirement,
            Artifact::Summary(_) => Kind::Summary,
        }
    }

    /// 目的：取内容（接收方自己解释它；组合子不调这个）。
    pub fn text(self) -> String {
        match self {
            Artifact::Requirement(text) | Artifact::Summary(text) => text,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// 目的：线格式的词汇只有一套：`as_str` / `parse` 与声明文件的 serde 名字必须一致。
    #[test]
    fn the_wire_names_are_the_declared_ones() {
        for kind in [Kind::Requirement, Kind::Summary] {
            let serde_name = serde_json::to_value(kind).expect("种类能序列化");
            assert_eq!(serde_name.as_str(), Some(kind.as_str()), "{:?}", kind);
            assert_eq!(Kind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(Kind::parse("ghost"), None);
    }
}
