//! 目的：随包提示词书与角色表的自洽与加载契约。
//! 管：角色面、目录保留名、系统工具与角色一致性、会话种类与机制书、`YamlPrompts` 的缺文件与坏文件。
//! 不管：提示词渲染（见 src/tests/prompt.rs）。
//! 联动：隔离根 `tests::scratch`。

use super::*;
/// 角色的工具面**来自角色表**（不是提示词里另写一遍）：给什么写什么，没给的不出现。
/// 这一份面同时喂三处：回合注入的工具块、原生声明槽、越权校验——所以判据只有它一个。
#[test]
fn role_face_comes_from_the_role_table() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读三张表");
    let ids = |role: &str| -> Vec<String> {
        st.tool_face(role)
            .expect("角色表里的角色")
            .into_iter()
            .map(|(id, _)| id.to_string())
            .collect()
    };
    let discussant = ids("discussant");
    for verb in ["say", "agree", "leave", "ask"] {
        assert!(
            discussant.iter().any(|t| t == verb),
            "讨论者该能用 {}：{:?}",
            verb,
            discussant
        );
    }
    // 执行者不讨论：它的面里**不该**出现讨论动词（工具块与声明槽都照这份面走，所以这就够了）。
    let executor = ids("executor");
    for verb in ["say", "agree", "leave", "ask"] {
        assert!(
            !executor.iter().any(|t| t == verb),
            "执行者不该拿到 {}：{:?}",
            verb,
            executor
        );
    }
    // 描述与参数也来自工具总表（模型侧说明与校验同源）。
    let (_, schema) = st
        .tool_face("discussant")
        .expect("讨论者角色在")
        .into_iter()
        .find(|(id, _)| *id == "agree")
        .expect("agree 在讨论席面里");
    assert!(!schema.desc.is_empty(), "工具说明不该为空");
    assert!(
        schema
            .params
            .as_ref()
            .is_some_and(|p| p.contains_key("text")),
        "agree 的参数契约来自 tools.yaml"
    );
    assert!(st.tool_face("不存在的角色").is_err(), "未知角色要如实报错");
    // 单 agent 工作（solo）与任务链执行席（executor）差在**回报工具**：回报只有节点子会话有消费者。
    let solo = ids("solo");
    for tool in ["read", "list", "write", "edit", "patch", "search"] {
        assert!(
            solo.iter().any(|t| t == tool),
            "solo 该能用 {}：{:?}",
            tool,
            solo
        );
    }
    assert!(
        !solo.iter().any(|t| t == "submit_report"),
        "solo 不该拿到回报工具（单 agent 没有消费者）：{:?}",
        solo
    );
    for verb in ["say", "agree", "leave", "ask"] {
        assert!(
            !solo.iter().any(|t| t == verb),
            "solo 不做讨论，不该拿到 {}：{:?}",
            verb,
            solo
        );
    }
}

/// 目录保留名来自仓库自带的名字表（不在代码里硬编码）：布局固定的目录都必须在册。
#[test]
fn names_table_lists_directory_reserved_names() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读三张表");
    for want in ["work", "children"] {
        assert!(
            st.reserved_names.iter().any(|n| n == want),
            "名字表里该有 {}：{:?}",
            want,
            st.reserved_names
        );
    }
}

/// 两张表必须自洽：悬空引用 / 缺能力都会被挡下（不靠人看）。
#[test]
fn system_tools_and_roles_are_self_consistent() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读系统工具与角色");
    assert!(
        st.problems().is_empty(),
        "两张表必须自洽：{:?}",
        st.problems()
    );
    // 越权判据只有一处：**这一回合的工具面**（角色表发放），给了的放行、没给的不放行。
    let face = |role: &str| st.role_face(role).0;
    let has = |role: &str, tool: &str| face(role).iter().any(|t| t == tool);
    assert!(has("discussant", "read"), "discussant 该能读");
    assert!(!has("discussant", "create_session"), "没给的工具必须不放行");
    assert!(face("不存在的角色").is_empty(), "不存在的角色什么都不放行");
    // 悬空引用会被 problems 逮到。
    let mut broken = st.clone();
    broken
        .roles
        .get_mut("discussant")
        .expect("角色在")
        .tools
        .push("no_such_tool".to_string());
    assert!(
        broken
            .problems()
            .iter()
            .any(|p| p.contains("不存在的系统工具")),
        "悬空引用必须被挡下：{:?}",
        broken.problems()
    );
}

/// 机制册必须与两张表自洽：**会话使用类型**在册、**适用角色**在角色表里；
/// 而且真实会话路径（类型 × 角色）都拿得到机制说明——对应关系是数据，这里只校验不写死。
#[test]
fn mechanism_book_matches_session_kinds_and_roles() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读系统工具与角色");
    let prompts = YamlPrompts::new(root.join("prompts"))
        .load()
        .expect("读提示词册");
    let kinds = &prompts.core.session_kinds;
    let role_ids: Vec<&String> = st.roles.keys().collect();
    assert!(!prompts.core.mechanisms.is_empty(), "机制册不能为空");
    for m in &prompts.core.mechanisms {
        assert!(
            kinds.iter().any(|k| k == &m.session),
            "会话使用类型不在全表里：{}",
            m.session
        );
        assert!(!m.roles.is_empty(), "一条机制至少要有一个适用角色");
        for r in &m.roles {
            assert!(role_ids.contains(&r), "适用角色不在角色表里：{}", r);
        }
    }
    for (kind, role) in [
        ("single", "solo"),
        ("collab", "discussant"),
        ("collab", "executor"),
        ("proxy", "core_proxy"),
    ] {
        assert!(
            !prompts.core.mechanisms_for(kind, role).trim().is_empty(),
            "{} × {} 没有机制说明",
            kind,
            role
        );
    }
    assert!(
        prompts
            .core
            .mechanisms_for("single", "core_proxy")
            .trim()
            .is_empty(),
        "类型与角色对不上就不发"
    );
}

#[test]
fn yaml_prompts_loads_the_shipped_book_and_reports_missing_or_broken_files() {
    let root = scratch("yaml-prompts");
    let root_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let real = root_dir.join("prompts");
    let ok = YamlPrompts::new(real)
        .load()
        .expect("产品自带提示词册必须合法");
    assert!(!ok.core.path_rules.is_empty());

    // 目录不存在 = 装配错误（如实报，不静默造默认文案）。
    let missing = YamlPrompts::new(root.join("nope")).load().unwrap_err();
    assert!(missing.contains("提示词册目录读不了"), "{}", missing);

    // 空目录 = 一个 .yaml 都没有，同样是装配错误。
    let empty = root.join("empty");
    std::fs::create_dir_all(&empty).expect("造空目录");
    assert!(
        YamlPrompts::new(empty)
            .load()
            .unwrap_err()
            .contains("一个 .yaml 都没有"),
        "空目录必须如实报错"
    );

    // 语法坏 = 如实报。
    let broken = root.join("broken");
    std::fs::create_dir_all(&broken).expect("造坏目录");
    std::fs::write(broken.join("a.yaml"), "core: [不是映射").expect("造坏册子");
    assert!(
        YamlPrompts::new(broken.clone())
            .load()
            .unwrap_err()
            .contains("提示词册非法")
    );

    // 键在两个文件里重复：**拆分时最可能犯的错**，必须被抓到（否则静默丢一份）。
    std::fs::write(broken.join("a.yaml"), "no_model: \"甲\"").expect("造册子");
    std::fs::write(broken.join("b.yaml"), "no_model: \"乙\"").expect("造重复键");
    assert!(
        YamlPrompts::new(broken)
            .load()
            .unwrap_err()
            .contains("重复"),
        "重复键必须如实报错"
    );

    let _ = std::fs::remove_dir_all(&root);
}
