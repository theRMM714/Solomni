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

/// 目的：工作环境册按环境分发——三平台各有本机档、guest 档覆盖全表，内容各不相同。
#[test]
pub(crate) fn prompt_env_book_distributes_by_environment() {
    let p = test_prompts();
    assert_eq!(p.core.env_hosts.len(), 3, "三平台各一份本机档");
    assert!(!p.core.env_guests.is_empty(), "guest os 全表不能空");
    for h in &p.core.env_hosts {
        let block = p.core.env_for(h, None);
        assert!(!block.trim().is_empty(), "宿主平台 {} 要有环境块", h);
        assert!(block.contains("{{work_root}}"), "环境块要带路径变量：{}", h);
    }
    let lin = p.core.env_for("linux", None);
    let win = p.core.env_for("windows", None);
    let mac = p.core.env_for("macos", None);
    assert_ne!(lin, win, "不同宿主平台给不同环境块");
    assert_ne!(lin, mac, "不同宿主平台给不同环境块");
    assert!(
        win.contains("Windows"),
        "Windows 宿主的环境块要说清路径写法：{}",
        win
    );
    let guest = p.core.env_for("linux", Some("linux"));
    assert!(
        guest.contains("Linux guest"),
        "虚拟机档要说清给的是 guest 内路径：{}",
        guest
    );
    assert!(p
        .core
        .env_for("linux", Some("windows"))
        .contains("Windows guest"));
    assert!(
        p.core.env_for("linux", Some("nope")).is_empty(),
        "没有的环境不瞎拼（非法值由装载时校验挡住）"
    );
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
    // 文案由 refs 代码返回：期望值在测试里同样直接构造。
    let foreign = |path: &str, agent: &str| {
        format!(
            "（无权读取：{} 在 agent「{}」的私有沙箱里，不是你自己的。可用的是上面列出的真实路径）",
            path, agent
        )
    };
    let collab = |path: &str, agent: &str| {
        format!(
            "（{} 只有 agent「{}」能读，其它 agent 无权；要共享请让用户把它放进本工作的共享区）",
            path, agent
        )
    };
    // 真实根：共享区 + 本 agent（甲）的私有沙箱
    let roots = RefRoots {
        work: abs(&["w", "work"]),
        private: Some(abs(&["w", "甲"])),
        path_codec: None,
    };
    let collab_roots = RefRoots {
        work: abs(&["w", "work"]),
        private: None,
        path_codec: None,
    };
    // @work：与 speaker 无关，一律给出共享区真实路径
    assert_eq!(
        rewrite("@work:a.txt", Some("甲"), &roots),
        s(&["w", "work", "a.txt"])
    );
    assert_eq!(
        rewrite("@work:sub/a.txt", None, &roots),
        s(&["w", "work", "sub", "a.txt"])
    );
    // @sandbox：命中自己的沙箱 → 私有沙箱真实路径
    assert_eq!(
        rewrite("@sandbox:甲/note.txt", Some("甲"), &roots),
        s(&["w", "甲", "note.txt"])
    );
    // @sandbox：别人的沙箱 → 册子文案（带文件名与 agent 名，不泄漏真实路径）
    assert_eq!(
        rewrite("@sandbox:乙/note", Some("甲"), &roots),
        foreign("note", "乙")
    );
    // @sandbox：协作（没有"自己的沙箱"）→ 册子里的协作文案
    assert_eq!(
        rewrite("@sandbox:乙/note", None, &collab_roots),
        collab("note", "乙")
    );
    // 终止标点留在原文里（只替换前缀+路径），所以句中标点/收尾标点都原样保留
    assert_eq!(
        rewrite("@work:a.txt，", Some("甲"), &roots),
        format!("{}，", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("看 @sandbox:甲/b.txt。", Some("甲"), &roots),
        format!("看 {}。", s(&["w", "甲", "b.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt 请读它", Some("甲"), &roots),
        format!("{} 请读它", s(&["w", "work", "a.txt"]))
    );
    assert_eq!(
        rewrite("@work:a.txt，请读它", Some("甲"), &roots),
        format!("{}，请读它", s(&["w", "work", "a.txt"]))
    );
    // 句点不是终止符：note.txt 是完整路径
    assert_eq!(
        rewrite("@work:note.txt", Some("甲"), &roots),
        s(&["w", "work", "note.txt"])
    );
    assert_eq!(
        rewrite("@sandbox:乙/note.txt", Some("甲"), &roots),
        foreign("note.txt", "乙"),
        "扩展名要算进路径，不能截成 note"
    );
    assert_eq!(
        rewrite("@sandbox:乙/\"note.txt\"", Some("甲"), &roots),
        foreign("note.txt", "乙")
    );
    // 已知取舍：句尾英文句点算进路径（要精确表达就用引号形式）
    assert_eq!(
        rewrite("@work:a.txt.", Some("甲"), &roots),
        s(&["w", "work", "a.txt."])
    );
    assert_eq!(
        rewrite("@work:\"a.txt.\"", Some("甲"), &roots),
        s(&["w", "work", "a.txt."])
    );
    // 引号形式：空白与标点都算路径的一部分
    assert_eq!(
        rewrite("@work:\"项目 说明.md\"", Some("甲"), &roots),
        s(&["w", "work", "项目 说明.md"])
    );
    assert_eq!(
        rewrite("@work:\"a,b(1).md\"", Some("甲"), &roots),
        s(&["w", "work", "a,b(1).md"])
    );
    assert_eq!(
        rewrite("@sandbox:甲/\"a b.md\"", Some("甲"), &roots),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@work:\"a b.md\" 看一下", Some("甲"), &roots),
        format!("{} 看一下", s(&["w", "work", "a b.md"]))
    );
    // agent 名也支持引号（名字含空白）：引号后必须紧跟 /
    assert_eq!(
        rewrite(
            "@sandbox:\"调研 助手\"/\"a b.md\"",
            Some("调研 助手"),
            &roots
        ),
        s(&["w", "甲", "a b.md"])
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手\"/a", Some("甲"), &roots),
        foreign("a", "调研 助手")
    );
    // 根还没就绪（代拟确认前）：原样保留引用，不编路径
    assert_eq!(
        rewrite("@work:a.txt", None, &RefRoots::default()),
        "@work:a.txt"
    );
    // 不完整前缀 / 结构不满足 / 引号未闭合：原样输出（不猜）
    assert_eq!(rewrite("@work:", Some("甲"), &roots), "@work:");
    assert_eq!(rewrite("@work: ", Some("甲"), &roots), "@work: ");
    assert_eq!(
        rewrite("@sandbox:甲", Some("甲"), &roots),
        "@sandbox:甲",
        "缺相对路径"
    );
    assert_eq!(
        rewrite("@sandbox:/a.txt", Some("甲"), &roots),
        "@sandbox:/a.txt",
        "缺 agent 名"
    );
    assert_eq!(
        rewrite("@sandbox:甲/", Some("甲"), &roots),
        "@sandbox:甲/",
        "相对路径为空"
    );
    assert_eq!(
        rewrite("@work:\"a b.md", Some("甲"), &roots),
        "@work:\"a b.md",
        "路径引号未闭合"
    );
    assert_eq!(
        rewrite("@work:\"\"", Some("甲"), &roots),
        "@work:\"\"",
        "空引号路径"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研 助手/a.md", Some("甲"), &roots),
        "@sandbox:\"调研 助手/a.md",
        "agent 名引号未闭合"
    );
    assert_eq!(
        rewrite("@sandbox:\"调研\"x/a.md", Some("甲"), &roots),
        "@sandbox:\"调研\"x/a.md",
        "agent 名引号后缺 /"
    );
    // 普通文本 / 误伤：不含两种前缀一律原样
    assert_eq!(rewrite("", Some("甲"), &roots), "");
    assert_eq!(rewrite("没有引用", Some("甲"), &roots), "没有引用");
    assert_eq!(
        rewrite("email@xxx.com 是我的", Some("甲"), &roots),
        "email@xxx.com 是我的"
    );
    assert_eq!(rewrite("@workx:a.txt", Some("甲"), &roots), "@workx:a.txt");
    // 一句里多个引用；引号形式与无引号旧形式并存
    assert_eq!(
        rewrite("@work:a.txt 和 @sandbox:甲/b.txt", Some("甲"), &roots),
        format!(
            "{} 和 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "甲", "b.txt"])
        )
    );
    assert_eq!(
        rewrite("@work:a.txt 与 @work:\"a b.md\"", Some("甲"), &roots),
        format!(
            "{} 与 {}",
            s(&["w", "work", "a.txt"]),
            s(&["w", "work", "a b.md"])
        )
    );
}

/// 目的：虚拟机档里用户 @ 引用落成 **guest 写法**——模块工具在 guest 内直接能用；且系统工具仍译得回同一个宿主落点。
/// 约束：路径翻译器只有一份（沙箱的 path_codec；会话的 @ 根由同一个 guest 派生），两条入口给同一种写法。
#[test]
pub(crate) fn ref_rewrite_uses_the_session_path_dialect() {
    use crate::capabilities::prompt::api::rewrite;
    use crate::capabilities::session::api::SessionParams;
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;
    use crate::capabilities::workspace::api::Sandboxes;
    use crate::capabilities::workspace::domain::workspace::Place;
    use std::sync::Arc;
    // 同方言（本机档）：桥是恒等，改写结果就是原有宿主写法，行为不变。
    let params =
        SessionParams::from_workspace("甲", &test_sandbox("甲", &["data"]), &[], "single", "solo");
    assert_eq!(
        rewrite("@work:a.txt", Some("甲"), &params.ref_roots()),
        s(&["demo", "work", "a.txt"]),
        "本机档行为不变"
    );
    // 跨方言（虚拟机档）：@work 与自己的 @sandbox 都落成 guest 写法。
    let guest = if cfg!(windows) {
        PathDialect::Posix
    } else {
        PathDialect::Windows
    };
    let bridge = PathBridge::new(std::env::consts::OS, guest);
    let mut sb = test_sandbox("甲", &["data"]);
    sb.path_codec = Some(Arc::new(bridge));
    let params = SessionParams::from_workspace("甲", &sb, &[], "single", "solo").with_guest(Some(
        if cfg!(windows) {
            crate::capabilities::vm::api::GuestOs::Macos
        } else {
            crate::capabilities::vm::api::GuestOs::Windows
        },
    ));
    let roots = params.ref_roots();
    let work = rewrite("@work:a.txt 看一下", Some("甲"), &roots);
    assert_eq!(
        work,
        format!(
            "{} 看一下",
            bridge.to_guest(&sb.shared.join("a.txt")).replace('\\', "/")
        ),
        "虚拟机档的 @work 要是 guest 写法"
    );
    // 模型拿到 guest 写法再交给同一座桥寻址：译回宿主、落点仍是那个文件（越界与否的判据不变）。
    let own = rewrite("@sandbox:甲/note.txt", Some("甲"), &roots);
    assert_eq!(
        own,
        bridge
            .to_guest(&sb.private.join("note.txt"))
            .replace('\\', "/"),
        "虚拟机档自己的 @sandbox 要是 guest 写法"
    );
    // 寻址按桥的规范写法（与挂载点、命令 cwd 同一种；转录里给模型看的是上面那种一律 / 形式）。
    let own_raw = bridge.to_guest(&sb.private.join("note.txt"));
    let (place, path) = sb.resolve(&own_raw).expect("guest 写法要能寻址");
    assert_eq!(place, Place::Private);
    assert_eq!(path, sb.private.join("note.txt"));
    // 协作入口（set_task / answer / note_user）没有"自己的沙箱"：根的翻译器由沙箱清单给。
    let boxes = Sandboxes {
        shared: sb.shared.clone(),
        list: Vec::new(),
        codec: sb.path_codec.clone(),
    };
    assert_eq!(
        rewrite("@work:a.txt", None, &boxes.ref_roots()),
        bridge.to_guest(&sb.shared.join("a.txt")).replace('\\', "/"),
        "协作的 @work 也要按会话环境给写法（名单还没定时 list 是空的）"
    );
}

#[test]
pub(crate) fn session_at_roots_go_by_the_session_guest() {
    use crate::capabilities::session::api::SessionParams;
    // 会话的 @ 根由参数派生：没有 guest（本机档）就恒等，有 guest 就与同一 guest 造的桥给同一种写法。
    let sb = test_sandbox("甲", &["data"]);
    let host = SessionParams::from_workspace("甲", &sb, &[], "single", "solo");
    let host_roots = host.ref_roots();
    assert!(host_roots.path_codec.is_some(), "会话的 @ 根要带翻译器");
    assert_eq!(
        host_roots.work, sb.shared,
        "@work 的根是共享区真实根（写法由翻译器给）"
    );
    assert_eq!(host_roots.private, Some(sb.private.clone()));
    for g in [
        crate::capabilities::vm::api::GuestOs::Linux,
        crate::capabilities::vm::api::GuestOs::Macos,
        crate::capabilities::vm::api::GuestOs::Windows,
    ] {
        let p = SessionParams::from_workspace("甲", &sb, &[], "single", "solo").with_guest(Some(g));
        let r = p.ref_roots();
        let codec = r.path_codec.as_ref().expect("带 guest 的会话要有翻译器");
        // 三种 guest 各自给的写法必须与各自方言的桥一致（提示词里的写法与转录里的写法同源）。
        let want = crate::capabilities::vm::api::PathBridge::for_session(
            std::env::consts::OS,
            Some(g.dialect()),
        );
        assert_eq!(codec.to_view(&sb.private), want.to_guest(&sb.private));
    }
}

/// 目的：patch 指南的示例路径与【工作环境】同一种写法——guest 会话里不能出现宿主盘符。
/// 约束：示例路径必须是 guest 内真实存在的写法，模型照抄才落得下文件。
#[test]
pub(crate) fn patch_guide_examples_follow_the_session_path_dialect() {
    use crate::capabilities::vm::api::PathBridge;
    use crate::capabilities::vm::domain::paths::PathDialect;

    let mut sb = test_sandbox("甲", &[]);
    let host_root = crate::kernel::api::slash(&sb.private);
    let host_example = format!("*** Update File: {}/note.txt", host_root);
    let host = crate::capabilities::tools::api::tool_notes(&test_prompts(), &sb, &[], &[]);
    assert!(
        host.patch_guide.contains(&host_example),
        "本机档的示例用宿主写法：{}",
        host.patch_guide
    );

    let guest = if cfg!(windows) {
        PathDialect::Posix
    } else {
        PathDialect::Windows
    };
    let bridge = PathBridge::new(std::env::consts::OS, guest);
    sb.path_codec = Some(Arc::new(bridge));
    let notes = crate::capabilities::tools::api::tool_notes(&test_prompts(), &sb, &[], &[]);
    let guest_root = bridge.to_guest(&sb.private).replace('\\', "/");
    assert!(
        notes
            .patch_guide
            .contains(&format!("*** Update File: {}/note.txt", guest_root)),
        "guest 档的示例要用 guest 写法（{}）：{}",
        guest_root,
        notes.patch_guide
    );
    // 只比对整行示例：guest 路径本身可能包含宿主路径片段（POSIX 宿主的 /solomni-host/home/...），
    // 不能用 contains(宿主根) 判定“没出现宿主写法”。
    assert!(
        !notes.patch_guide.contains(&host_example),
        "guest 档不能再出现宿主写法的示例：{}",
        notes.patch_guide
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
    // 转录的用户行已是确切寻址（不是 @ 引用）
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
