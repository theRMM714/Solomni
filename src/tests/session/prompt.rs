//! 目的：提示词拼装：环境块、角色与身份。
//! 管：guest/host 路径拼法、模块根、chara 只在设置时进入身份。
//! 不管：会话建立与落盘；提示词书本身见 detail/prompt.rs。
//! 联动：`tests::builders` 的装配；契约见 docs/session/session-model.md。

use super::super::prelude::*;

/// 目的：工作环境块按会话的环境给路径——虚拟机档给 guest 拼法，并说清是哪种 guest。
#[test]
pub(crate) fn env_block_renders_guest_paths_in_vm_tier() {
    use crate::capabilities::session::domain::session::prompt::env_block;
    let prompt = test_prompts();
    let host = test_params("a");
    let host_block = env_block(&prompt, &host);
    assert!(
        host_block.contains("【工作环境】"),
        "本机档要有环境块：{}",
        host_block
    );

    let guest = test_params("a").with_guest(Some(crate::capabilities::vm::api::GuestOs::Linux));
    let guest_block = env_block(&prompt, &guest);
    assert!(
        guest_block.contains("Linux guest"),
        "虚拟机档要说清是哪种 guest：{}",
        guest_block
    );
    if cfg!(windows) {
        assert!(
            guest_block.contains("/solomni-host/"),
            "路径要按 guest 拼法给（镜像根）：{}",
            guest_block
        );
        // 示例里为了讲清楚映射会提到宿主路径；要断言的是**真实的沙箱根**不再以宿主拼法出现。
        let host_private = host.private.display().to_string();
        assert!(
            !host_private.is_empty() && !guest_block.contains(&host_private),
            "不该把宿主拼法的沙箱路径给模型：{}",
            guest_block
        );
    }
}

#[test]
pub(crate) fn agent_system_carries_the_real_roots() {
    // 提示词里给出的根必须是真实绝对路径（模型据此拼路径；外部工具也认它）。
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    let sid = core
        .create_work(work("w", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let system = &core.single_identity(&sid).expect("身份块");
    assert!(
        system.contains(&s(&["w", "work"])),
        "system 要含共享区真实根：{}",
        system
    );
    assert!(
        system.contains(&s(&["w", "a"])),
        "system 要含私有沙箱真实根：{}",
        system
    );
    assert!(
        system.contains(&format!("：{}", s(&["a"]))),
        "system 要含模块目录真实根：{}",
        system
    );
    // 路径写法要用真实根拼（模型照抄它）：环境块里的例子是**工作副本**（沙箱）。
    assert!(
        system.contains(&format!("{}/note.txt", s(&["w", "a"]))),
        "路径写法要用真实根拼：{}",
        system
    );
    // 护栏：提示词里不得出现任何解析不了的路径写法（模型会照抄它）
    assert!(
        !system.contains("work:/") && !system.contains("sandbox:/"),
        "提示词只该给真实根：{}",
        system
    );
}

/// 目的：人设是身份块里的**独立段**——有就出现，没有就不留空标题。
/// 约束：人设从登记处视图快照进会话，和模块 / 模型同一份 copy 语义。
#[test]
pub(crate) fn agent_chara_enters_identity_only_when_set() {
    let mut core = core_with(vec![module_of("a")], gw(BTreeMap::new(), vec!["[]".into()]));
    agent_upsert_chara(
        &mut core,
        "审稿人",
        &["a"],
        "",
        "",
        "严谨的审稿人：先质疑，再给方案。",
    )
    .unwrap();
    let view = core
        .registry()
        .pick_agents(&["审稿人".to_string()])
        .unwrap()
        .into_iter()
        .next()
        .expect("登记处视图");
    let sid = core
        .create_work(WorkSpec {
            name: "chara-w".to_string(),
            mode: WorkMode::Single,
            agents: vec![AgentInstance::from_view(&view)],
            task: None,
            delegate: false,
            permission: None,
            tier: Tier::Host,
            guest: None,
            workspace: None,
        })
        .unwrap()
        .sid;
    let system = core.single_identity(&sid).expect("身份块");
    assert!(
        system.contains("== 人设与身份 ==") && system.contains("严谨的审稿人：先质疑，再给方案。"),
        "登记处人设要进身份块：{}",
        system
    );
    // 空人设：模板里的 chara_block 是空串，不留下空标题。
    let empty = core
        .create_work(work("chara-empty", WorkMode::Single, &["a"]))
        .unwrap()
        .sid;
    let system2 = core.single_identity(&empty).expect("身份块");
    assert!(
        !system2.contains("== 人设与身份 =="),
        "空人设不该留下空标题：{}",
        system2
    );
}
