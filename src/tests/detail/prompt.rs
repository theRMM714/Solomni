//! 目的：随包提示词书与机制席位的自洽与加载契约。
//! 管：席位面、目录保留名、系统工具与机制席位的自洽、会话种类与机制书、`YamlPrompts` 的缺文件与坏文件。
//! 不管：提示词渲染（见 src/tests/prompt.rs）。
//! 联动：隔离根 `tests::scratch`。

use super::*;
/// 席位的工具面**来自机制声明**（不是提示词里另写一遍）：给什么写什么，没给的不出现。
/// 这一份面同时喂三处：回合注入的工具块、原生声明槽、越权校验——所以判据只有它一个。
#[test]
fn a_seat_face_comes_from_the_mechanism_declaration() {
    use crate::units::business::agentloop::api::role_face;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(assets_at(&root))
        .load()
        .expect("读工具总表");
    let decls = YamlDecls::new(assets_at(&root)).load().expect("读流程段表");
    let ids = |id: &str| -> Vec<String> {
        let face = role_face(decls.loops(), id);
        assert!(
            face.iter().all(|t| st.tools.contains_key(t)),
            "席位面引用的工具都在总表里：{:?}",
            face
        );
        face
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
    let schema = st.tools.get("agree").expect("agree 在工具总表里");
    assert!(!schema.desc.is_empty(), "工具说明不该为空");
    assert!(
        schema
            .params
            .as_ref()
            .is_some_and(|p| p.contains_key("text")),
        "agree 的参数契约来自 tools.yaml"
    );
    assert!(
        decls.loops().role("不存在的角色").is_none(),
        "未知角色要如实查不到"
    );
    assert!(
        ids("solo").contains(&"read".to_string()),
        "单角色该恒有沙箱面的读工具"
    );
    // 单 agent 工作（solo）与任务链执行席（executor）差在**回报工具**：回报只有节点子会话有消费者。
    let solo = ids("solo");
    for tool in ["read", "list", "write", "edit", "patch", "grep"] {
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
    let st = YamlSystools::new(assets_at(&root))
        .load()
        .expect("读三张表");
    for want in ["work", "children"] {
        assert!(
            st.reserved_agents.iter().any(|n| n == want),
            "名字表里该有 {}：{:?}",
            want,
            st.reserved_agents
        );
    }
}

/// 总表与机制面必须自洽：悬空引用 / 缺能力都会被挡下（不靠人看）。
#[test]
fn system_tools_and_mechanism_faces_are_self_consistent() {
    use crate::units::business::conversation::session::api::loop_problems;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let decls = YamlDecls::new(assets_at(&root))
        .load()
        .expect("读会话形状声明");
    let st = YamlSystools::new(assets_at(&root))
        .load()
        .expect("读系统工具与名字表");
    assert!(
        st.problems().is_empty(),
        "总表必须自洽：{:?}",
        st.problems()
    );
    assert!(
        loop_problems(decls.loops(), &st.tools).is_empty(),
        "流程段与总表必须自洽：{:?}",
        loop_problems(decls.loops(), &st.tools)
    );
    // 越权判据只有一处：**这一回合的工具面**（流程段按角色合成），给了的放行、没给的不放行。
    let has = |id: &str, tool: &str| {
        crate::units::business::agentloop::api::role_face(decls.loops(), id)
            .contains(&tool.to_string())
    };
    assert!(has("discussant", "read"), "讨论席该能读");
    assert!(!has("discussant", "create_session"), "没给的工具必须不放行");
    // 悬空引用会被 problems 逮到。
    let mut broken = st.tools.clone();
    broken.remove("agree");
    assert!(
        loop_problems(decls.loops(), &broken)
            .iter()
            .any(|p| p.contains("引用了不存在的系统工具")),
        "悬空引用必须被挡下：{:?}",
        loop_problems(decls.loops(), &broken)
    );
}

/// 机制册必须与席位声明自洽：**适用席位**都在册；而且真实会话路径上每个席位都拿得到机制说明——
/// 对应关系是数据，这里只校验不写死。
#[test]
fn mechanism_book_covers_every_seat_in_play() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let decls = YamlDecls::new(assets_at(&root))
        .load()
        .expect("读会话形状声明");
    for kind in ["single", "collab", "proxy"] {
        assert!(decls.known(kind), "缺会话类型：{}", kind);
    }
    let prompts = YamlPrompts::new(assets_at(&root))
        .load()
        .expect("读提示词册");
    // 装载时已校验两张表；这里只锁机制册的适用角色都在声明里。
    let roles = &decls.loops().roles;
    let role_ids: Vec<&str> = roles.iter().map(|r| r.id.as_str()).collect();
    assert!(!prompts.core.mechanisms.is_empty(), "机制册不能为空");
    for m in &prompts.core.mechanisms {
        assert!(!m.seats.is_empty(), "一条机制至少要有一个适用席位");
        for r in &m.seats {
            assert!(
                role_ids.contains(&r.as_str()),
                "适用席位没在机制声明里：{}",
                r
            );
        }
    }
    for role in ["solo", "discussant", "executor", "core_proxy"] {
        assert!(
            !prompts.core.mechanisms_for(role).trim().is_empty(),
            "{} 没有机制说明",
            role
        );
    }
    assert!(
        prompts.core.mechanisms_for("planner").trim().is_empty(),
        "没配说明的席位拿到空串"
    );
}

#[test]
fn yaml_prompts_loads_the_shipped_book_and_reports_missing_or_broken_files() {
    let root = scratch("yaml-prompts");
    let root_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let decls = YamlDecls::new(assets_at(&root_dir))
        .load()
        .expect("读会话形状声明");
    decls.validate().expect("产品自带的会话形状声明必须合法");
    let ok = YamlPrompts::new(assets_at(&root_dir))
        .load()
        .expect("产品自带提示词册必须合法");
    assert!(!ok.core.path_rules.is_empty());

    // 目录不存在 = 装配错误（如实报，不静默造默认文案）。
    let missing = YamlPrompts::new(assets_at(&root.join("nope")))
        .load()
        .unwrap_err();
    assert!(missing.contains("提示词册目录读不了"), "{}", missing);

    // 空目录 = 一个 .yaml 都没有，同样是装配错误。
    let empty = root.join("empty");
    std::fs::create_dir_all(empty.join("prompts")).expect("造空目录");
    assert!(
        YamlPrompts::new(assets_at(&empty))
            .load()
            .unwrap_err()
            .contains("一个 .yaml 都没有"),
        "空目录必须如实报错"
    );

    // 语法坏 = 如实报。
    let broken = root.join("broken");
    let dir = broken.join("prompts");
    std::fs::create_dir_all(&dir).expect("造坏目录");
    std::fs::write(dir.join("a.yaml"), "core: [不是映射").expect("造坏册子");
    assert!(
        YamlPrompts::new(assets_at(&broken))
            .load()
            .unwrap_err()
            .contains("提示词册非法")
    );

    // 键在两个文件里重复：**拆分时最可能犯的错**，必须被抓到（否则静默丢一份）。
    std::fs::write(dir.join("a.yaml"), "no_model: \"甲\"").expect("造册子");
    std::fs::write(dir.join("b.yaml"), "no_model: \"乙\"").expect("造重复键");
    assert!(
        YamlPrompts::new(assets_at(&broken))
            .load()
            .unwrap_err()
            .contains("重复"),
        "重复键必须如实报错"
    );

    let _ = std::fs::remove_dir_all(&root);
}
