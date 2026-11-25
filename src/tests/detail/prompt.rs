//! 目的：随包提示词书与机制席位的自洽与加载契约。
//! 管：席位面、目录保留名、系统工具与机制席位的自洽、会话种类与机制书、`YamlPrompts` 的缺文件与坏文件。
//! 不管：提示词渲染（见 src/tests/prompt.rs）。
//! 联动：隔离根 `tests::scratch`。

use super::*;
/// 席位的工具面**来自机制声明**（不是提示词里另写一遍）：给什么写什么，没给的不出现。
/// 这一份面同时喂三处：回合注入的工具块、原生声明槽、越权校验——所以判据只有它一个。
#[test]
fn a_seat_face_comes_from_the_mechanism_declaration() {
    use crate::units::agentloop::api::{resolve, seat_face};
    use crate::units::session_model::api::seat;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读工具总表");
    let ids = |id: &str| -> Vec<String> {
        let seat = seat(id).expect("机制声明里的席位");
        resolve(seat, &st.tools)
            .expect("席位面引用的工具都在总表里")
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
    let (_, schema) = resolve(seat("discussant").expect("讨论席在册"), &st.tools)
        .expect("讨论席面引用的工具都在总表里")
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
    assert!(seat("不存在的席位").is_none(), "未知席位要如实查不到");
    assert!(
        seat_face(seat("solo").expect("单席在册")).contains(&"read"),
        "单席该恒有沙箱面的读工具"
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
    let st = YamlSystools::new(root.join("systools"))
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
    use crate::units::conversation::session::api::mechanism_problems;
    use crate::units::session_model::api::seat;
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let st = YamlSystools::new(root.join("systools"))
        .load()
        .expect("读系统工具与名字表");
    assert!(
        st.problems().is_empty(),
        "总表必须自洽：{:?}",
        st.problems()
    );
    assert!(
        mechanism_problems(&st.tools).is_empty(),
        "机制面与总表必须自洽：{:?}",
        mechanism_problems(&st.tools)
    );
    // 越权判据只有一处：**这一回合的工具面**（机制按席位合成），给了的放行、没给的不放行。
    let has = |id: &str, tool: &str| {
        crate::units::agentloop::api::seat_face(seat(id).expect("席位在册")).contains(&tool)
    };
    assert!(has("discussant", "read"), "讨论席该能读");
    assert!(!has("discussant", "create_session"), "没给的工具必须不放行");
    // 悬空引用会被 problems 逮到。
    let mut broken = st.tools.clone();
    broken.remove("agree");
    assert!(
        mechanism_problems(&broken)
            .iter()
            .any(|p| p.contains("引用了不存在的系统工具")),
        "悬空引用必须被挡下：{:?}",
        mechanism_problems(&broken)
    );
}

/// 机制册必须与席位声明自洽：**适用席位**都在册；而且真实会话路径上每个席位都拿得到机制说明——
/// 对应关系是数据，这里只校验不写死。
#[test]
fn mechanism_book_covers_every_seat_in_play() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let profiles = YamlProfiles::new(root.join("systools"))
        .load()
        .expect("读会话类型表");
    for kind in ["single", "collab", "proxy"] {
        assert!(profiles.known(kind), "缺会话类型：{}", kind);
    }
    let prompts = YamlPrompts::new(root.join("prompts"))
        .load()
        .expect("读提示词册");
    use crate::units::session_model::api::SEATS;
    // 会话类型表在装载时已校验机制与流程；这里只锁机制册的适用席位都在声明里。
    let role_ids: Vec<&str> = SEATS.iter().map(|s| s.id).collect();
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
    let real = root_dir.join("prompts");
    let profiles = YamlProfiles::new(root_dir.join("systools"))
        .load()
        .expect("读会话类型表");
    profiles.validate().expect("产品自带的会话类型表必须合法");
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
