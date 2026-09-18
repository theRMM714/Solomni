//! 补丁通道（patch 工具）的纯逻辑：解析一段**自由格式**的改动文本，并在文件内容上应用。
//! 存在的理由：改文件时不必把大段文本塞进 JSON 字符串——换行/引号/CJK 的转义是真实会话里反复出事的点。
//!
//! 格式（标记行 = 行首以 *** 加一个空格开头；一次可写多块，按顺序应用）：
//!
//! ```text
//! *** Add File: <真实绝对路径>
//! 整份内容（原样）
//! *** End File
//! *** Update File: <真实绝对路径>
//! *** SEARCH
//! 要替换掉的原文（连续若干行）
//! *** REPLACE
//! 换成的新文（可以多行；留空 = 删掉这段）
//! *** End File
//! ```
//!
//! 规则：
//! - **每块必须以 *** End File 收尾**：它的后面可以随便写话（不会被当成文件内容）。
//!   这条是硬的——否则模型在补丁后面补一句"我改完了"就会被写进文件里。
//! - 第一个标记之前的散行被忽略（模型常先写一句再给补丁）。
//! - *** 是保留前缀：内容里若必须出现这样的行，请改写它，否则会被当成标记。
//! - 标记名不区分大小写；只认这五个：Add File / Update File / SEARCH / REPLACE / End File。
//! - 路径必须是**真实绝对路径**：越界与相对路径由沙箱拒（寻址规则与其它内置工具一致，不在这里判）。
//! - 匹配按**整行**比较，行尾风格保持原样；找不到 / 多处命中都如实报事实，不做模糊匹配。

/// 一个补丁块。
#[derive(Debug, Clone, PartialEq)]
pub enum Block {
    /// 新建或整份写入：路径已存在 = 整份覆盖（需要"本次会话完整读过"的证据，规则与 write 一致）。
    Add { path: String, content: String },
    /// 在已有文件里做若干处整行替换：按顺序应用（后一处看到前一处的结果）。
    Update { path: String, edits: Vec<Edit> },
}

impl Block {
    /// 这一块改的是哪个路径（原样，还没做沙箱寻址）。
    pub fn path(&self) -> &str {
        match self {
            Block::Add { path, .. } | Block::Update { path, .. } => path,
        }
    }
}

/// 一处替换：search / replace 都是**整块文本**（换行用 \n，不含行尾风格）。
#[derive(Debug, Clone, PartialEq)]
pub struct Edit {
    pub search: String,
    pub replace: String,
}

/// 解析失败的事实（文案在提示词册）。
#[derive(Debug, Clone, PartialEq)]
pub enum Fault {
    /// 整段文本里一个标记都没有。
    NoBlocks,
    /// 看不懂的标记。
    UnknownMarker { line: usize, text: String },
    /// 标记后面没写路径。
    MissingPath { line: usize, marker: String },
    /// Add File 后面没有内容。
    EmptyAdd { line: usize },
    /// Update File 后面一组改动都没有。
    EmptyUpdate { line: usize },
    /// SEARCH 主体为空（要在文件开头插入，就把开头那几行一起写进 SEARCH）。
    EmptySearch { line: usize },
    /// 这一块没有 *** End File 收尾（后面还可能跟着别的话，所以不能猜哪里是结尾）。
    MissingEnd { line: usize },
    /// SEARCH 之后没有 REPLACE。
    SearchWithoutReplace { line: usize },
    /// REPLACE 前面没有 SEARCH。
    ReplaceWithoutSearch { line: usize },
}

#[derive(Debug, Clone, PartialEq)]
enum Marker {
    Add,
    Update,
    Search,
    Replace,
    End,
    Unknown(String),
}

/// 一段区域：一个标记行 + 它后面直到下一个标记行（或结尾）的原样行。
struct Section {
    marker: Marker,
    rest: String,
    body: Vec<String>,
    line: usize,
}

/// 标记行（行首必须是 "*** "；前导空白不算标记）。
fn marker(line: &str) -> Option<(Marker, String)> {
    let rest = line.strip_prefix("*** ")?;
    let (head, tail) = match rest.find(':') {
        Some(i) => (&rest[..i], &rest[i + 1..]),
        None => (rest, ""),
    };
    let head = head.trim();
    let m = if head.eq_ignore_ascii_case("Add File") || head.eq_ignore_ascii_case("Add") {
        Marker::Add
    } else if head.eq_ignore_ascii_case("Update File") || head.eq_ignore_ascii_case("Update") {
        Marker::Update
    } else if head.eq_ignore_ascii_case("SEARCH") {
        Marker::Search
    } else if head.eq_ignore_ascii_case("REPLACE") {
        Marker::Replace
    } else if head.eq_ignore_ascii_case("End File") || head.eq_ignore_ascii_case("End") {
        Marker::End
    } else {
        Marker::Unknown(head.to_string())
    };
    Some((m, tail.trim().to_string()))
}

/// 解析补丁文本。
pub fn parse(text: &str) -> Result<Vec<Block>, Fault> {
    // 第一遍：按标记行切段（标记之前的散行忽略）。
    let mut sections: Vec<Section> = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        if let Some((m, rest)) = marker(raw) {
            sections.push(Section {
                marker: m,
                rest,
                body: Vec::new(),
                line: i + 1,
            });
        } else if let Some(last) = sections.last_mut() {
            last.body.push(raw.to_string());
        }
    }
    // 第二遍：把段解释成块（每块以 End 收尾）。
    let mut blocks: Vec<Block> = Vec::new();
    let mut i = 0usize;
    while i < sections.len() {
        let s = &sections[i];
        match &s.marker {
            Marker::Add => {
                let path = need_path(s)?;
                if s.body.iter().all(|l| l.trim().is_empty()) {
                    return Err(Fault::EmptyAdd { line: s.line });
                }
                let content = s.body.join("\n");
                // 必须以 End 收尾（否则后面的话会被当成文件内容）
                match sections.get(i + 1) {
                    Some(next) if next.marker == Marker::End => {
                        blocks.push(Block::Add { path, content });
                        i += 2;
                    }
                    _ => return Err(Fault::MissingEnd { line: s.line }),
                }
            }
            Marker::Update => {
                let path = need_path(s)?;
                let start = s.line;
                let mut edits: Vec<Edit> = Vec::new();
                let mut j = i + 1;
                loop {
                    let Some(sec) = sections.get(j) else {
                        return Err(Fault::MissingEnd { line: start });
                    };
                    match &sec.marker {
                        Marker::End => {
                            if edits.is_empty() {
                                return Err(Fault::EmptyUpdate { line: start });
                            }
                            blocks.push(Block::Update { path, edits });
                            j += 1;
                            break;
                        }
                        Marker::Search => {
                            if sec.body.iter().all(|l| l.trim().is_empty()) {
                                return Err(Fault::EmptySearch { line: sec.line });
                            }
                            let search = sec.body.join("\n");
                            let search_line = sec.line;
                            let Some(rep) = sections.get(j + 1) else {
                                return Err(Fault::SearchWithoutReplace { line: search_line });
                            };
                            if rep.marker != Marker::Replace {
                                return Err(Fault::SearchWithoutReplace { line: search_line });
                            }
                            edits.push(Edit {
                                search,
                                replace: rep.body.join("\n"),
                            });
                            j += 2;
                        }
                        // 块里出现了别的标记：如实说清（既不猜，也不静默）
                        Marker::Add | Marker::Update => {
                            return Err(Fault::MissingEnd { line: start })
                        }
                        Marker::Replace => {
                            return Err(Fault::ReplaceWithoutSearch { line: sec.line })
                        }
                        Marker::Unknown(name) => {
                            return Err(Fault::UnknownMarker {
                                line: sec.line,
                                text: name.clone(),
                            })
                        }
                    }
                }
                i = j;
            }
            Marker::Search | Marker::Replace => {
                return Err(Fault::ReplaceWithoutSearch { line: s.line });
            }
            // 没有块时的 End File 是空话：忽略（不为此浪费模型一轮）
            Marker::End => i += 1,
            Marker::Unknown(name) => {
                return Err(Fault::UnknownMarker {
                    line: s.line,
                    text: name.clone(),
                })
            }
        }
    }
    if blocks.is_empty() {
        return Err(Fault::NoBlocks);
    }
    Ok(blocks)
}

fn need_path(s: &Section) -> Result<String, Fault> {
    if s.rest.trim().is_empty() {
        return Err(Fault::MissingPath {
            line: s.line,
            marker: match s.marker {
                Marker::Add => "Add File".to_string(),
                _ => "Update File".to_string(),
            },
        });
    }
    Ok(s.rest.trim().to_string())
}

/// 一处改动失败的事实（文案在提示词册）。
#[derive(Debug, Clone, PartialEq)]
pub enum EditFault {
    /// 原文在这份文件里找不到。
    NotFound { total: usize },
    /// 命中了多处。
    Multiple { lines: Vec<usize> },
    /// 只差空白：给出第一个候选行的行号与该行原文（照着改）。
    NearMiss { line: usize, actual: String },
}

/// 在内容上按顺序应用若干处整行替换，返回新内容。
/// - 按整行比较；行尾风格（\r\n 还是 \n）与原文件一致；末尾有没有换行也保持一致。
/// - 失败 = （第几处改动，从 1 数起, 事实）；失败时不返回半成品（调用方整体不写盘）。
pub fn apply_edits(current: &str, edits: &[Edit]) -> Result<String, (usize, EditFault)> {
    let eol = if current.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    };
    let mut lines: Vec<String> = current.lines().map(|l| l.to_string()).collect();
    for (k, edit) in edits.iter().enumerate() {
        let want: Vec<&str> = edit.search.lines().collect();
        let hits = find_lines(&lines, &want);
        if hits.is_empty() {
            return Err((k + 1, near_or_not_found(&lines, &want)));
        }
        if hits.len() > 1 {
            return Err((
                k + 1,
                EditFault::Multiple {
                    lines: hits.iter().map(|i| i + 1).collect(),
                },
            ));
        }
        let at = hits[0];
        let new: Vec<String> = edit.replace.lines().map(|l| l.to_string()).collect();
        lines.splice(at..at + want.len(), new);
    }
    let mut out = lines.join(eol);
    if current.ends_with('\n') && !out.is_empty() {
        out.push_str(eol);
    }
    Ok(out)
}

/// 整行精确匹配的全部起点。
fn find_lines(lines: &[String], want: &[&str]) -> Vec<usize> {
    let mut out: Vec<usize> = Vec::new();
    if want.is_empty() || want.len() > lines.len() {
        return out;
    }
    for i in 0..=(lines.len() - want.len()) {
        if want.iter().enumerate().all(|(j, w)| lines[i + j] == *w) {
            out.push(i);
        }
    }
    out
}

/// 找不到时给最有用的那条事实：只差空白就报"只差空白"，否则只说找不到。
fn near_or_not_found(lines: &[String], want: &[&str]) -> EditFault {
    fn flat(s: &str) -> String {
        s.split_whitespace().collect::<Vec<_>>().join(" ")
    }
    let flat_want: Vec<String> = want.iter().map(|w| flat(w)).collect();
    if !flat_want.is_empty() && flat_want.len() <= lines.len() {
        for i in 0..=(lines.len() - flat_want.len()) {
            if flat_want
                .iter()
                .enumerate()
                .all(|(j, w)| flat(&lines[i + j]) == *w)
            {
                return EditFault::NearMiss {
                    line: i + 1,
                    actual: lines[i].clone(),
                };
            }
        }
    }
    EditFault::NotFound { total: lines.len() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn add(path: &str, content: &str) -> Block {
        Block::Add {
            path: path.to_string(),
            content: content.to_string(),
        }
    }

    fn upd(path: &str, edits: Vec<(&str, &str)>) -> Block {
        Block::Update {
            path: path.to_string(),
            edits: edits
                .into_iter()
                .map(|(s, r)| Edit {
                    search: s.to_string(),
                    replace: r.to_string(),
                })
                .collect(),
        }
    }

    #[test]
    fn parses_multiple_blocks_with_prose_around() {
        let text = "我先改两处。\n\n*** Update File: D:/w/a.md\n*** SEARCH\n第一行\n*** REPLACE\n改后的第一行\n*** SEARCH\n旧句\n*** REPLACE\n新句\n*** End File\n中间说一句。\n*** Add File: D:/w/b.md\n新文件的\n两行内容\n*** End File\n我改完了。\n";
        let got = parse(text).expect("应能解析");
        assert_eq!(
            got,
            vec![
                upd(
                    "D:/w/a.md",
                    vec![("第一行", "改后的第一行"), ("旧句", "新句")]
                ),
                add("D:/w/b.md", "新文件的\n两行内容"),
            ],
            "补丁之后的话不会被写进文件"
        );
    }

    #[test]
    fn every_block_must_be_closed_by_end_file() {
        // 少了 End File：不能猜哪里是结尾（后面可能还有话）——如实报错
        assert_eq!(
            parse("*** Add File: /a\n内容\n"),
            Err(Fault::MissingEnd { line: 1 })
        );
        assert_eq!(
            parse("*** Update File: /a\n*** SEARCH\nx\n*** REPLACE\ny\n"),
            Err(Fault::MissingEnd { line: 1 })
        );
        // End File 之后的话一律忽略
        let got = parse("*** Add File: /a\n内容\n*** End File\n以上。\n").expect("解析");
        assert_eq!(got, vec![add("/a", "内容")]);
    }

    #[test]
    fn deleting_a_span_is_an_empty_replace() {
        let got = parse("*** Update File: /a\n*** SEARCH\n删掉这行\n*** REPLACE\n*** End File\n")
            .expect("空 REPLACE = 删除");
        assert_eq!(got, vec![upd("/a", vec![("删掉这行", "")])]);
    }

    #[test]
    fn marker_names_ignore_case() {
        let got = parse("*** update file: /a\n*** search\nx\n*** replace\ny\n*** end file\n")
            .expect("大小写不敏感");
        assert_eq!(got, vec![upd("/a", vec![("x", "y")])]);
        // 没有块时的 End File 是空话：忽略，不浪费一轮
        let got2 = parse("*** End File\n*** Add File: /a\n内容\n*** End File\n")
            .expect("先来一个空 End 也能解析");
        assert_eq!(got2, vec![add("/a", "内容")]);
    }

    #[test]
    fn faults_are_specific_facts() {
        assert_eq!(parse("这里没有标记"), Err(Fault::NoBlocks));
        assert_eq!(parse(""), Err(Fault::NoBlocks));
        assert_eq!(
            parse("*** Update File:\n*** SEARCH\nx\n*** REPLACE\ny\n*** End File\n"),
            Err(Fault::MissingPath {
                line: 1,
                marker: "Update File".to_string()
            })
        );
        assert_eq!(
            parse("*** Add File: /a\n*** End File\n"),
            Err(Fault::EmptyAdd { line: 1 })
        );
        assert_eq!(
            parse("*** Update File: /a\n*** End File\n"),
            Err(Fault::EmptyUpdate { line: 1 })
        );
        assert_eq!(
            parse("*** Update File: /a\n*** SEARCH\n\n*** REPLACE\ny\n*** End File\n"),
            Err(Fault::EmptySearch { line: 2 })
        );
        assert_eq!(
            parse("*** Update File: /a\n*** SEARCH\nx\n*** End File\n"),
            Err(Fault::SearchWithoutReplace { line: 2 })
        );
        assert_eq!(
            parse("*** Update File: /a\n*** REPLACE\ny\n*** End File\n"),
            Err(Fault::ReplaceWithoutSearch { line: 2 })
        );
        assert_eq!(
            parse("*** Add File: /a\n内容\n*** Fix File: /a\n*** End File\n"),
            Err(Fault::MissingEnd { line: 1 })
        );
        assert_eq!(
            parse("*** Fix File: /a\n*** End File\n"),
            Err(Fault::UnknownMarker {
                line: 1,
                text: "Fix File".to_string()
            })
        );
    }

    #[test]
    fn searches_are_matched_by_whole_lines_and_applied_in_order() {
        let current = "一\n二\n三\n";
        let out = apply_edits(
            current,
            &[
                Edit {
                    search: "二".to_string(),
                    replace: "二改".to_string(),
                },
                Edit {
                    search: "三".to_string(),
                    replace: "三改".to_string(),
                },
            ],
        )
        .expect("两处都能改");
        assert_eq!(out, "一\n二改\n三改\n", "末尾换行保持原样");
        let out2 = apply_edits(
            "a\n",
            &[
                Edit {
                    search: "a".to_string(),
                    replace: "b".to_string(),
                },
                Edit {
                    search: "b".to_string(),
                    replace: "c".to_string(),
                },
            ],
        )
        .expect("顺序应用");
        assert_eq!(out2, "c\n");
    }

    #[test]
    fn failures_name_the_edit_and_the_fact() {
        assert_eq!(
            apply_edits(
                "一\n二\n",
                &[Edit {
                    search: "没有这行".to_string(),
                    replace: "x".to_string()
                }]
            ),
            Err((1, EditFault::NotFound { total: 2 }))
        );
        assert_eq!(
            apply_edits(
                "dup\n中\ndup\n",
                &[Edit {
                    search: "dup".to_string(),
                    replace: "x".to_string()
                }]
            ),
            Err((1, EditFault::Multiple { lines: vec![1, 3] }))
        );
        match apply_edits(
            "第一段\n    缩进过的句子\n",
            &[Edit {
                search: "缩进过的句子".to_string(),
                replace: "x".to_string(),
            }],
        ) {
            Err((1, EditFault::NearMiss { line, actual })) => {
                assert_eq!(line, 2);
                assert_eq!(actual, "    缩进过的句子");
            }
            other => panic!("应报只差空白：{:?}", other),
        }
        assert_eq!(
            apply_edits(
                "a\n",
                &[
                    Edit {
                        search: "a".to_string(),
                        replace: "b".to_string()
                    },
                    Edit {
                        search: "nope".to_string(),
                        replace: "x".to_string()
                    },
                ]
            ),
            Err((2, EditFault::NotFound { total: 1 }))
        );
    }

    #[test]
    fn line_endings_and_missing_final_newline_are_preserved() {
        let out = apply_edits(
            "一\r\n二\r\n",
            &[Edit {
                search: "二".to_string(),
                replace: "二改".to_string(),
            }],
        )
        .expect("CRLF");
        assert_eq!(out, "一\r\n二改\r\n");
        let out2 = apply_edits(
            "只有一行",
            &[Edit {
                search: "只有一行".to_string(),
                replace: "改了".to_string(),
            }],
        )
        .expect("无末尾换行");
        assert_eq!(out2, "改了");
        let out3 = apply_edits(
            "留\n删\n",
            &[Edit {
                search: "删".to_string(),
                replace: String::new(),
            }],
        )
        .expect("删除");
        assert_eq!(out3, "留\n");
    }

    #[test]
    fn a_multiline_search_must_match_contiguously() {
        let current = "一\n二\n三\n";
        let out = apply_edits(
            current,
            &[Edit {
                search: "一\n二".to_string(),
                replace: "合并".to_string(),
            }],
        )
        .expect("多行 SEARCH");
        assert_eq!(out, "合并\n三\n");
        assert!(apply_edits(
            current,
            &[Edit {
                search: "二\n一".to_string(),
                replace: "x".to_string()
            }]
        )
        .is_err());
    }
}
