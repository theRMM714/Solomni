//! 提示词能力测试：册子装载 / 渲染 / `@` 改写。
//! 归属判据：钉的是**本能力的不变式**；顺手经过别处只是路径，不是归属。
use super::builders::*;
use super::prelude::*;

#[test]
pub(crate) fn prompt_render_replaces_and_rejects_missing() {
    let ok = render("你好 {{name}}！", &[("name", "世界".to_string())]).unwrap();
    assert_eq!(ok, "你好 世界！");
    assert!(render("{{missing}}", &[]).is_err());
}

#[test]
pub(crate) fn prompt_book_loads_from_yaml() {
    let p = test_prompts();
    // 语气约定；**能用哪些表态由角色表渲染**（见 systools/roles.yaml），不在这句话里。
    assert!(!p.core.chat_protocol.trim().is_empty());
    // 机制册按（会话使用类型 × 角色）分发：每条非空、都写了适用角色，类型表非空。
    assert!(
        p.core.mechanisms.len() >= 3,
        "机制册至少要覆盖单 agent / 协作 / 代理"
    );
    assert!(!p.core.session_kinds.is_empty(), "会话使用类型的全表不能空");
    for m in &p.core.mechanisms {
        assert!(!m.text.trim().is_empty(), "机制说明不能为空：{}", m.session);
        assert!(!m.roles.is_empty(), "每条机制都要写清适用角色");
    }
    assert!(p
        .core
        .mechanisms_for("single", "solo")
        .contains("单 agent 工作"));
    let collab = p.core.mechanisms_for("collab", "discussant");
    assert!(collab.contains("一个 agent = 一个会话"), "{}", collab);
    assert!(collab.contains("动词"), "{}", collab);
    assert!(p
        .core
        .mechanisms_for("proxy", "core_proxy")
        .contains("派完活就让出回合"));
    assert!(
        p.core
            .mechanisms_for("single", "core_proxy")
            .trim()
            .is_empty(),
        "会话使用类型对不上就不发"
    );
    assert!(p.core.discuss.opener.contains("{{protocol}}"));
}

#[test]
pub(crate) fn prompt_render_keeps_single_braces() {
    let ok = render("输出 {\"a\":1} 和 {{x}}", &[("x", "Y".to_string())]).unwrap();
    assert_eq!(ok, "输出 {\"a\":1} 和 Y");
}

// ---------- 登记处解析链与密钥治理 ----------

#[test]
pub(crate) fn refs_rewrite_covers_prefixes_speakers_and_punctuation() {
    use crate::capabilities::prompt::api::{rewrite, RefRoots};
    // 文案来自提示词册：期望值也用册子渲染出来，代码里不复制那两句中文。
    let t = test_prompts().refs();
    let foreign = |path: &str, agent: &str| {
        crate::capabilities::prompt::domain::prompt::render(
            &t.foreign_sandbox,
            &[("agent", agent.to_string()), ("path", path.to_string())],
        )
        .expect("册子变量齐全")
    };
    let collab = |path: &str, agent: &str| {
        crate::capabilities::prompt::domain::prompt::render(
            &t.collab_sandbox,
            &[("path", path.to_string()), ("agent", agent.to_string())],
        )
        .expect("册子变量齐全")
    };
    assert!(
        t.foreign_sandbox.contains("{{agent}}") && t.foreign_sandbox.contains("{{path}}"),
        "无权文案要同时报出文件与 agent"
    );
    // 真实根：共享区 + 本 agent（甲）的私有沙箱
    let roots = RefRoots {
        work: abs(&["w", "work"]),
        private: Some(abs(&["w", "甲"])),
    };
    let collab_roots = RefRoots {
        work: abs(&["w", "work"]),
        private: None,
    };
    // @work：与 speaker 无关，一律给出共享区真实路径
    assert_eq!(
        rewrite("@work:a.txt", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt"])
    );
    assert_eq!(
        rewrite("@work:sub/a.txt", None, &roots, &t),
        s(&["w", "work", "sub", "a.txt"])
    );
    // @sandbox：命中自己的沙箱 → 私有沙箱真实路径
    assert_eq!(
        rewrite("@sandbox:甲/note.txt", Some("甲"), &roots, &t),
        s(&["w", "甲", "note.txt"])
    );
    // @sandbox：别人的沙箱 → 册子文案（带文件名与 agent 名，不泄漏真实路径）
    assert_eq!(
        rewrite("@sandbox:乙/note", Some("甲"), &roots, &t),
        foreign("note", "乙")
    );
    // @sandbox：协作（没有"自己的沙箱"）→ 册子里的协作文案
    assert_eq!(
        rewrite("@sandbox:乙/note", None, &collab_roots, &t),
        collab("note", "乙")
    );
    // 终止标点留在原文里（只替换前缀+路径），所以句中标点/收尾标点都原样保留
    assert_eq!(
        rewrite("@work:a.txt，", Some("甲"), &roots, &t),
        format!("{}，", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("看 @sandbox:甲/b.txt。", Some("甲"), &roots, &t),
        format!("看 {}。", s(&["w", "甲", "b.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt 请读它", Some("甲"), &roots, &t),
        format!("{} 请读它", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt，请读它", Some("甲"), &roots, &t),
        format!("{}，请读它", s(&["w", "work", "a.txt"]))
    );
    // 句点不是终止符：note.txt 是完整路径
    assert_eq!(
        rewrite("@work:note.txt", Some("甲"), &roots, &t),
        s(&["w", "work", "note.txt"])
    );
    assert_eq!(
        rewrite("@sandbox:乙/note.txt", Some("甲"), &roots, &t),
        foreign("note.txt", "乙"),
        "扩展名要算进路径，不能截成 note"
    );
    assert_eq!(
        rewrite("@sandbox:乙/\"note.txt\"", Some("甲"), &roots, &t),
        foreign("note.txt", "乙")
    );
    // 已知取舍：句尾英文句点算进路径（要精确表达就用引号形式）
    assert_eq!(
        rewrite("@work:a.txt.", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt."])
    );
    assert_eq!(
        rewrite("@work:\"a.txt.\"", Some("甲"), &roots, &t),
        s(&["w", "work", "a.txt."])
    );
    // 引号形式：空白与标点都算路径的一部分
    assert_eq!(
        rewrite("@work:\"项目 说明.md\"", Some("甲"), &roots, &t),
        s(&["w", "work", "项目 说明.md"])
    );
    assert_eq!(
        rewrite("@work:\"a,b(1).md\"", Some("甲"), &roots, &t),
        s(&["w", "work", "a,b(1).md"])
    );
    assert_eq!(
        rewrite("@sandbox:甲/\"a b.md\"", Some("甲"), &roots, &t),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@work:\"a b.md\" 看一下", Some("甲"), &roots, &t),
        format!("{} 看一下", s(&["w", "work", "a b.md"]))
    );
    // agent 名也支持引号（名字含空白）：引号后必须紧跟 /
    assert_eq!(
        rewrite(
            "@sandbox:\"调研 助手\"/\"a b.md\"",
            Some("调研 助手"),
            &roots,
            &t
        ),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手\"/a", Some("甲"), &roots, &t),
        foreign("a", "调研 助手")
    );
    // 根还没就绪（代拟确认前）：原样保留引用，不编路径
    assert_eq!(
        rewrite("@work:a.txt", None, &RefRoots::default(), &t),
        "@work:a.txt"
    );
    // 不完整前缀 / 结构不满足 / 引号未闭合：原样输出（不猜）
    assert_eq!(rewrite("@work:", Some("甲"), &roots, &t), "@work:");
    assert_eq!(rewrite("@work: ", Some("甲"), &roots, &t), "@work: ");
    assert_eq!(
        rewrite("@sandbox:甲", Some("甲"), &roots, &t),
        "@sandbox:甲",
        "缺相对路径"
    );
    assert_eq!(
        rewrite("@sandbox:/a.txt", Some("甲"), &roots, &t),
        "@sandbox:/a.txt",
        "缺 agent 名"
    );
    assert_eq!(
        rewrite("@sandbox:甲/", Some("甲"), &roots, &t),
        "@sandbox:甲/",
        "相对路径为空"
    );
    assert_eq!(
        rewrite("@work:\"a b.md", Some("甲"), &roots, &t),
        "@work:\"a b.md",
        "路径引号未闭合"
    );
    assert_eq!(
        rewrite("@work:\"\"", Some("甲"), &roots, &t),
        "@work:\"\"",
        "空引号路径"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手/a.md", Some("甲"), &roots, &t),
        "@sandbox:\"调研 助手/a.md",
        "agent 名引号未闭合"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研\"x/a.md", Some("甲"), &roots, &t),
        "@sandbox:\"调研\"x/a.md",
        "agent 名引号后缺 /"
    );
    // 普通文本 / 误伤：不含两种前缀一律原样
    assert_eq!(rewrite("", Some("甲"), &roots, &t), "");
    assert_eq!(rewrite("没有引用", Some("甲"), &roots, &t), "没有引用");
    assert_eq!(
        rewrite("email@xxx.com 是我的", Some("甲"), &roots, &t),
        "email@xxx.com 是我的"
    );
    assert_eq!(
        rewrite("@workx:a.txt", Some("甲"), &roots, &t),
        "@workx:a.txt"
    );
    // 一句里多个引用；引号形式与无引号旧形式并存
    assert_eq!(
        rewrite("@work:a.txt 和 @sandbox:甲/b.txt", Some("甲"), &roots, &t),
        format!(
            "{} 和 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "甲", "b.txt"])
        )
    );
    assert_eq!(
        rewrite("@work:a.txt 与 @work:\"a b.md\"", Some("甲"), &roots, &t),
        format!(
            "{} 与 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "work", "a b.md"])
        )
    );
}

#[test]
pub(crate) fn user_at_reference_is_rewritten_in_transcript_and_history() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好的\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let events = with_live(|l| core.single_say(&sid, "@work:a.txt 看一下", l)).unwrap();
    // 转录的用户行已是确切寻址（不再是 @ 引用）
    let rows = transcript_rows(&events);
    let want = format!("[用户] {} 看一下", s(&["w", "work", "a.txt"]));
    assert_eq!(rows[0].1, want, "{:?}", rows);
    // 进上下文的是同一份文本（转录即内容）
    let h = core.single_history(&sid).unwrap();
    let want_msg = format!("{} 看一下", s(&["w", "work", "a.txt"]));
    assert!(
        h.iter().any(|m| m.role == "user" && m.content == want_msg),
        "{:?}",
        h
    );
    assert!(
        !h.iter().any(|m| m.content.contains("@work:")),
        "历史里不得残留 @ 引用：{:?}",
        h
    );
}
