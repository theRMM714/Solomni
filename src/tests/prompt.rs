//! 目的：钉提示词册装载、渲染与 @ 改写的契约。
//! 管：渲染占位替换与拒绝缺失、册按环境分发、yaml 装载、单花括号保留、ref 改写前缀/说话人/标点、会话路径方言、@ 根、补丁指南示例、用户 @ 引用。
//! 不管：讨论发言与协作状态（collab）。
//! 联动：src/units/business/prompt/、session 路径方言。

use super::builders::*;
use super::prelude::*;
use crate::units::business::session_model::api::SINGLE;

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
    assert!(
        p.core
            .env_for("linux", Some("windows"))
            .contains("Windows guest")
    );
    assert!(
        p.core.env_for("linux", Some("nope")).is_empty(),
        "没有的环境不瞎拼（非法值由装载时校验挡住）"
    );
}

#[test]
pub(crate) fn prompt_book_loads_from_yaml() {
    let p = test_prompts();
    // 语气约定；**能用哪些表态由席位面渲染**（见 session/domain/mechanism.rs 的 SEATS），不在这句话里。
    assert!(!p.core.chat_protocol.trim().is_empty());
    // 机制册按**适用席位**分发：每条非空、都写了适用席位（席位归 session/domain/mechanism.rs 的 SEATS）。
    assert!(
        p.core.mechanisms.len() >= 4,
        "机制册至少要覆盖单机 / 讨论 / 执行 / 核心代理"
    );
    for m in &p.core.mechanisms {
        assert!(!m.text.trim().is_empty(), "机制说明不能为空：{:?}", m.seats);
        assert!(!m.seats.is_empty(), "每条机制都要写清适用席位");
    }
    assert!(p.core.mechanisms_for("solo").contains("单 agent 工作"));
    assert!(
        p.core.mechanisms_for("discussant").contains("动词"),
        "讨论席要看到表态约定"
    );
    assert!(
        p.core.mechanisms_for("executor").contains("submit_report"),
        "执行席要看到收尾回报"
    );
    assert!(
        p.core
            .mechanisms_for("core_proxy")
            .contains("派完活就让出回合")
    );
    assert!(
        p.core.mechanisms_for("planner").trim().is_empty(),
        "没配说明的席位拿到空串"
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
    use crate::units::business::prompt::api::{RefRoots, rewrite};
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

#[test]
pub(crate) fn session_at_roots_go_by_the_session_guest() {
    use crate::units::business::agentloop::api::SessionParams;
    // 会话的 @ 根由参数派生：没有 guest（本机档）就恒等，有 guest 就与同一 guest 造的桥给同一种写法。
    let sb = test_sandbox("甲", &["data"]);
    let host = SessionParams::from_workspace("甲", &sb, &[], "standard", "solo");
    let host_roots = host.ref_roots();
    assert!(host_roots.path_codec.is_some(), "会话的 @ 根要带翻译器");
    assert_eq!(
        host_roots.work, sb.shared,
        "@work 的根是共享区真实根（写法由翻译器给）"
    );
    assert_eq!(host_roots.private, Some(sb.private.clone()));
    for g in [
        crate::units::business::vm::api::GuestOs::Linux,
        crate::units::business::vm::api::GuestOs::Macos,
        crate::units::business::vm::api::GuestOs::Windows,
    ] {
        let p =
            SessionParams::from_workspace("甲", &sb, &[], "standard", "solo").with_guest(Some(g));
        let r = p.ref_roots();
        let codec = r.path_codec.as_ref().expect("带 guest 的会话要有翻译器");
        // 三种 guest 各自给的写法必须与各自方言的桥一致（提示词里的写法与转录里的写法同源）。
        let want = crate::units::business::vm::api::PathBridge::for_session(
            std::env::consts::OS,
            Some(g.dialect()),
        );
        assert_eq!(codec.to_view(&sb.private), want.to_guest(&sb.private));
    }
}

#[test]
pub(crate) fn user_at_reference_is_rewritten_in_transcript_and_history() {
    let mut member = BTreeMap::new();
    member.insert(
        "a".to_string(),
        vec!["{\"type\":\"say\",\"text\":\"好的\"}".to_string()],
    );
    let mut core = core_with(vec![module_of("a")], gw(member, vec!["[]".into()]));
    let sid = core.create_work(work("w", SINGLE, &["a"])).unwrap().sid;
    let events = with_turn_io(|io, cancel, emit| {
        core.turn.single_say(
            &sid,
            "@work:a.txt 看一下",
            io,
            cancel,
            emit,
            core.registry.as_ref(),
            core.probe.as_ref(),
        )
    })
    .unwrap();
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
