//! 用户输入里的 @ 引用：改写成 agent 能用的**真实绝对路径**（纯逻辑，无 IO、无依赖）。
//! 只认两种前缀：@work:<相对路径> 与 @sandbox:<agent实例名>/<相对路径>；其余一律原样留在文本里（不猜）。
//! 路径范围：带引号时取到下一个引号为止（引号内的空白与标点都算路径），否则到空白或终止标点为止。
//! 终止符**留在原文里不消费**，所以改写只替换「前缀 + 路径」这一段。
//! 改写结果 = 真实根目录 + 相对路径（用 Path 组件拼，输出统一 / 分隔：JSON 里反斜杠是转义符）；绝不泄漏别人的沙箱路径。

use crate::core::prompt::RefsPrompts;
use std::path::{Path, PathBuf};

/// @ 改写需要的真实根：work = 本工作共享区；private = 发言席自己的私有沙箱（协作时 None）。
#[derive(Debug, Clone, Default)]
pub struct RefRoots {
    pub work: PathBuf,
    pub private: Option<PathBuf>,
}

/// 把用户输入里的 @ 引用改写成真实路径；说明文案取自提示词册（texts）。
/// speaker = Some(agent 实例名)（单 agent 形态：它自己的沙箱可达）；None = 协作（多人共读同一段文字）。
pub fn rewrite(text: &str, speaker: Option<&str>, roots: &RefRoots, texts: &RefsPrompts) -> String {
    if text.is_empty() || !text.contains('@') {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 16);
    let mut rest = text;
    loop {
        let Some(i) = rest.find('@') else {
            out.push_str(rest);
            break;
        };
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        // 从 @ 处尝试匹配前缀；成功且根已就绪才改写，否则把 @ 原样留下（不猜）。
        if let Some(body) = after.strip_prefix("work:") {
            if let Some((path, consumed)) = take_ref(body) {
                if let Some(full) = full_path(&roots.work, path) {
                    out.push_str(&full);
                    rest = &body[consumed..];
                    continue;
                }
            }
        } else if let Some(body) = after.strip_prefix("sandbox:") {
            if let Some((agent, rel, consumed)) = take_sandbox_ref(body) {
                // 自己的沙箱：用真实根；别人的/协作：给册子里的说明（不泄漏真实路径）。
                let own = match speaker {
                    Some(me) if me == agent => {
                        roots.private.as_ref().and_then(|p| full_path(p, rel))
                    }
                    _ => None,
                };
                let rendered = match own {
                    Some(full) => Some(full),
                    None if speaker == Some(agent) => None, // 根还没就绪：原样保留，不编路径
                    None => Some(sandbox_note(agent, rel, speaker, texts)),
                };
                if let Some(full) = rendered {
                    out.push_str(&full);
                    rest = &body[consumed..];
                    continue;
                }
            }
        }
        out.push('@');
        rest = after;
    }
    out
}

/// 真实根 + 相对路径：用 Path 组件拼（输出平台分隔符）。根为空（还没准备好）→ None。
fn full_path(root: &Path, rel: &str) -> Option<String> {
    if root.as_os_str().is_empty() {
        return None;
    }
    let mut p = root.to_path_buf();
    for seg in rel.split(['/', '\\']) {
        if !seg.is_empty() {
            p.push(seg);
        }
    }
    // 对外一律 / 分隔：Windows 反斜杠在 JSON 字符串里是转义符，模型照抄会写出非法 JSON。
    Some(crate::core::workspace::slash(&p))
}

/// 引用别人（或协作）的沙箱：用册子文案如实说明谁能读，不给出真实路径。
/// 单 agent 形态（speaker 是别人）用 foreign_sandbox；协作（speaker = None）用 collab_sandbox。
fn sandbox_note(agent: &str, rel: &str, speaker: Option<&str>, texts: &RefsPrompts) -> String {
    let template = if speaker.is_none() {
        &texts.collab_sandbox
    } else {
        &texts.foreign_sandbox
    };
    crate::core::prompt::render(
        template,
        &[("agent", agent.to_string()), ("path", rel.to_string())],
    )
    .expect("refs 文案变量由调用方保证（缺变量属于装配错误）")
}

/// 取一段引用路径：带引号（"…"）时取到下一个引号为止（引号本身不进结果）；
/// 否则到空白或终止标点为止。返回（路径, 消费掉的字节数）；None = 形式不合法（调用方原样输出）。
fn take_ref(body: &str) -> Option<(&str, usize)> {
    if let Some(inner) = body.strip_prefix('"') {
        // 没有收尾引号 = 不猜，整体原样。
        let end = inner.find('"')?;
        let path = &inner[..end];
        if path.is_empty() {
            return None;
        }
        return Some((path, 1 + end + 1));
    }
    let end = body.find(is_terminator).unwrap_or(body.len());
    let path = &body[..end];
    if path.is_empty() {
        return None;
    }
    Some((path, end))
}

/// @sandbox: 的引用体：<agent>/<路径>。agent 名可带引号（名字含空白时用），路径也可以各带各的引号。
/// 返回（agent, 相对路径, 消费字节数）。
fn take_sandbox_ref(body: &str) -> Option<(&str, &str, usize)> {
    let (agent, rel_body, head) = if let Some(inner) = body.strip_prefix('"') {
        // agent 名带引号：读到下一个引号为止，之后必须紧跟 /
        let end = inner.find('"')?;
        let agent = &inner[..end];
        let tail = &inner[end + 1..];
        if agent.is_empty() || !tail.starts_with('/') {
            return None;
        }
        (agent, &tail[1..], 1 + end + 2)
    } else {
        let slash = body.find('/')?;
        let agent = &body[..slash];
        if agent.is_empty() {
            return None;
        }
        (agent, &body[slash + 1..], slash + 1)
    };
    let (rel, consumed) = take_ref(rel_body)?;
    if rel.is_empty() {
        return None;
    }
    Some((agent, rel, head + consumed))
}

/// 裸路径的终止符：空白，或下面任一中英标点（**留在原文里**，所以只替换前缀与路径）。
/// 故意**不含 '.'**：句点更常是扩展名分隔符（note.txt），当终止符会把路径截断。
/// ':' 保留：Windows 文件名不允许冒号，可以安全当终止符。
/// 已知取舍：句尾打英文句点（@work:a.txt.）时句点算进路径——要精确表达就用引号形式（@work:"a.txt."）。
fn is_terminator(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            '，' | '。'
                | '；'
                | '、'
                | '！'
                | '？'
                | '：'
                | ','
                | ';'
                | '!'
                | '?'
                | ':'
                | '）'
                | ')'
                | '」'
                | '』'
                | '】'
                | '》'
        )
}
