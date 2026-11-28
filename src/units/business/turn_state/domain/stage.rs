//! 目的：段进度——复合流程把自己走到哪一段写成**落盘的事实**（切点），恢复时按它给每段开口。
//! 管：进度事实的线格式（`kind = stage`）、从转录算起跑点与行号基线、把切点交出的产物随事实一起落盘。
//! 不管：段声明（`mechanism.rs`）；流水线推进（`sequence.rs`）；转录的存取（`transcript`）。
//! 联动：`Sequence` 每推进一段写一条；`sequence_of` 恢复时读它决定从哪一段、从哪一条事件开始。

use crate::units::business::session_model::api::{Artifact, Kind};
use crate::units::business::transcript::api::SessionEvent;

/// 目的：进度事实的 `kind`（也就是落盘与线上的 `type`）。
pub const STAGE: &str = "stage";

/// 目的：路径前缀：段号写的是**整条流水线的绝对位置**（`seq[2]` 就是第三段）。
pub const PATH: &str = "seq";

/// 目的：恢复现场：走到第几段（绝对段号）、这一段的事件从 `events` 的哪一条开始、
///   行号基线（这一段开跑时的下一个行号）、切点交出的产物。
#[derive(Debug, Clone)]
pub struct Resume {
    /// 目的：绝对段号（流水线里的第几段，从 0 数）。
    pub at: usize,
    /// 目的：这一段的事件从 `events` 的哪一条开始（切点那条进度事实的下一条）。
    pub from: usize,
    /// 目的：行号基线：切点当时的下一个行号。
    pub line: u64,
    /// 目的：切点交给这一段的产物（没有交出东西就是 `None`）。
    pub handoff: Option<Artifact>,
}

impl Resume {
    /// 目的：从头开始（没有进度事实时的缺省）。
    pub fn start() -> Resume {
        Resume {
            at: 0,
            from: 0,
            line: 0,
            handoff: None,
        }
    }
}

/// 目的：进度事实里的路径（声明了绝对段号的字符串）。
pub fn path_of(index: usize) -> String {
    format!("{}[{}]", PATH, index)
}

/// 目的：造一条进度事实：走到第 `index` 段（绝对）、当时的下一行号 `line`、交出的产物。
pub fn mark(index: usize, line: u64, made: Option<&Artifact>) -> SessionEvent {
    let mut payload = serde_json::Map::new();
    payload.insert("path".to_string(), serde_json::json!(path_of(index)));
    payload.insert("at".to_string(), serde_json::json!(index));
    payload.insert("line".to_string(), serde_json::json!(line));
    if let Some(artifact) = made {
        payload.insert("artifact".to_string(), artifact_json(artifact));
    }
    SessionEvent::push(STAGE, serde_json::Value::Object(payload))
}

/// 目的：从转录算起跑点：最后一条**属于本流水线**的进度事实；一条都没有就从头开始。
/// 约束：路径与段号必须自洽（`seq[2]` 配 `at = 2`），别条流水线或改坏的事实一律不认。
pub fn resume(events: &[serde_json::Value]) -> Resume {
    let mut seen = Resume::start();
    for (i, ev) in events.iter().enumerate() {
        if ev.get("type").and_then(|t| t.as_str()) != Some(STAGE) {
            continue;
        }
        let Some(at) = ev.get("at").and_then(|v| v.as_u64()) else {
            continue;
        };
        let at = at as usize;
        if ev.get("path").and_then(|p| p.as_str()) != Some(path_of(at).as_str()) {
            continue;
        }
        seen = Resume {
            at,
            from: i + 1,
            line: ev.get("line").and_then(|v| v.as_u64()).unwrap_or(0),
            handoff: ev.get("artifact").and_then(artifact_from),
        };
    }
    seen
}

/// 目的：行号基线：全量转录里最大的行 id + 1（没有行就是 0）。
/// 约束：行号只增不退；进度事实里的基线是切点当时的，段里后来又写过行就以转录里的更大值为准。
pub fn base_line(events: &[serde_json::Value]) -> u64 {
    let mut max = 0;
    for ev in events {
        if ev.get("type").and_then(|t| t.as_str()) != Some("transcript") {
            continue;
        }
        let Some(lines) = ev.get("lines").and_then(|l| l.as_array()) else {
            continue;
        };
        for line in lines {
            if let Some(id) = line.get("id").and_then(|v| v.as_u64()) {
                max = max.max(id);
            }
        }
    }
    if max == 0 { 0 } else { max + 1 }
}

/// 目的：产物的线格式（进度事实里随切点一起落盘）。
pub fn artifact_json(artifact: &Artifact) -> serde_json::Value {
    let text = match artifact {
        Artifact::Requirement(text) | Artifact::Summary(text) => text.clone(),
    };
    serde_json::json!({ "kind": artifact.kind_of().as_str(), "text": text })
}

/// 目的：从线格式读回产物；种类不认识或缺内容就是 `None`。
pub fn artifact_from(value: &serde_json::Value) -> Option<Artifact> {
    let kind = Kind::parse(value.get("kind")?.as_str()?)?;
    let text = value.get("text")?.as_str()?.to_string();
    Some(match kind {
        Kind::Requirement => Artifact::Requirement(text),
        Kind::Summary => Artifact::Summary(text),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line_event(id: u64) -> serde_json::Value {
        serde_json::json!({ "type": "transcript", "lines": [{ "id": id }] })
    }

    /// 目的：一条进度事实都读回一件事实，两条就取最后一条。
    #[test]
    fn the_last_stage_fact_is_the_starting_point() {
        let events = vec![
            line_event(1),
            mark(1, 2, None).to_json(),
            line_event(2),
            mark(2, 3, Some(&Artifact::Summary("总结".to_string()))).to_json(),
            line_event(3),
        ];
        let at = resume(&events);
        assert_eq!(at.at, 2);
        assert_eq!(at.from, 4, "从事实的下一条开始");
        assert_eq!(at.line, 3);
        assert_eq!(at.handoff, Some(Artifact::Summary("总结".to_string())));
        assert_eq!(base_line(&events), 4, "最大行号 + 1");
    }

    /// 目的：没有事实、或事实被改坏，都从头开始（不猜）。
    #[test]
    fn a_missing_or_mangled_fact_starts_from_the_top() {
        assert_eq!(resume(&[line_event(7)]).at, 0);
        let mangled = serde_json::json!({ "type": "stage", "path": "seq[1]", "at": 3 });
        let at = resume(&[mangled]);
        assert_eq!((at.at, at.from), (0, 0), "路径与段号对不上就不认");
    }

    /// 目的：产物随事实原样落盘、原样读回。
    #[test]
    fn the_handoff_rides_the_stage_fact() {
        let made = Artifact::Summary("讨论完的总结".to_string());
        let ev = mark(0, 1, Some(&made)).to_json();
        assert_eq!(artifact_from(&ev["artifact"]), Some(made));
        assert_eq!(
            artifact_json(&Artifact::Requirement("需求".to_string()))["kind"],
            "requirement"
        );
    }
}
