//! 目的：路径与工具夹具——绝对根、路径书写形式与内置工具调用的现成素材。
//! 管：测试根、路径期望值、工具调用与模块工具声明、沙箱与提示词素材。
//! 不管：替身本身（其余同目录文件）。
//! 联动：`src/kernel/ports.rs` 的路径模型与 `src/capabilities/tools/domain/`。

use super::*;

/// 目的：测试用绝对根：以当前工作目录为锚（只读 env，不碰盘）——真实路径模型下所有根都是绝对路径。
pub(crate) fn abs(parts: &[&str]) -> PathBuf {
    let mut b = std::env::current_dir().expect("取当前目录");
    for x in parts {
        b.push(x);
    }
    b
}

/// 目的：绝对路径的字符串期望值（平台分隔符；用于 PathBuf / 内存 IO 的键）。
pub(crate) fn p(parts: &[&str]) -> String {
    abs(parts).to_string_lossy().into_owned()
}

/// 目的：路径的**书写形式**（进 JSON / 提示词 / 转录都是它）：一律 / 分隔（Windows 反斜杠在 JSON 里非法）。
pub(crate) fn s(parts: &[&str]) -> String {
    p(parts).replace(std::path::MAIN_SEPARATOR, "/")
}

/// 目的：什么都不修的修复端口（严格要求合法信封）：测试基线，也是"宁缺毋滥"部署的对照实现。
pub(crate) struct NoRepair;

impl crate::capabilities::llm::ports::EnvelopeRepair for NoRepair {
    fn repair(
        &self,
        _raw: &str,
        _kind: &crate::capabilities::llm::api::Malformed,
    ) -> crate::capabilities::llm::api::RepairOutcome {
        crate::capabilities::llm::api::RepairOutcome {
            repaired: None,
            what: Vec::new(),
        }
    }
}

/// 目的：一次内置工具调用（空账本）：只关心工具行为本身的用例用它；关心"改动前有没有读过"的用例自带 Observations 再调它。这里直接走 tools 的 domain（测试层允许）：生产路径是 `ToolExec::run_builtin`。
pub(crate) fn run_builtin(
    sb: &crate::capabilities::workspace::api::Sandbox,
    io: &dyn crate::capabilities::tools::ports::SysIo,
    name: &str,
    args_json: &str,
) -> crate::kernel::api::ToolOutcome {
    let mut obs = crate::capabilities::tools::api::Observations::default();
    crate::capabilities::tools::service::systool::execute(
        sb,
        &test_systools().tools,
        io,
        &mut obs,
        name,
        args_json,
    )
}

/// 目的：测试用模块工具声明：只给启动命令（参数契约在需要的用例里另行声明）。
pub(crate) fn decl(command: &str) -> crate::capabilities::modules::api::ToolDecl {
    crate::capabilities::modules::api::ToolDecl {
        command: command.to_string(),
        desc: String::new(),
        params: None,
        parallel: false,
    }
}

/// 目的：测试用模块工具声明：带参数契约（YAML 里的 params 段）。
pub(crate) fn decl_with(
    command: &str,
    params_yaml: &str,
) -> crate::capabilities::modules::api::ToolDecl {
    let mut d = decl(command);
    d.params = Some(yaml_serde::from_str(params_yaml).expect("测试参数声明要能解析"));
    d
}

/// 目的：测试沙箱：work 共享区 + agent 私有区 + 指定模块目录（都是绝对路径）。
pub(crate) fn test_sandbox(
    agent: &str,
    modules: &[&str],
) -> crate::capabilities::workspace::api::Sandbox {
    let mut map = BTreeMap::new();
    for id in modules {
        map.insert(id.to_string(), abs(&["mods", id]));
    }
    crate::capabilities::workspace::api::Sandbox {
        work_name: "demo".to_string(),
        session: "demo".to_string(),
        agent: agent.to_string(),
        shared: abs(&["demo", "work"]),
        private: abs(&["demo", agent]),
        modules: map,
        modules_with_userdata: std::collections::BTreeSet::new(),
        permissions: Default::default(),
        path_codec: None,
    }
}

/// 目的：只读共享区的测试沙箱：模拟**生产里 agent 会话**的默认（共享主副本只读，写入走 work_commit）。
pub(crate) fn test_sandbox_readonly(
    agent: &str,
    modules: &[&str],
) -> crate::capabilities::workspace::api::Sandbox {
    test_sandbox(agent, modules)
}

/// 目的：测试用会话参数：agent 名 + 该 agent 的沙箱（无模块）。身份块由它现渲染。
pub(crate) fn test_params(agent: &str) -> crate::capabilities::session::api::SessionParams {
    crate::capabilities::session::api::SessionParams::from_workspace(
        agent,
        &test_sandbox(agent, &[]),
        &[],
        "single",
        "solo",
    )
}

/// 目的：测试用工具说明块素材（patch 语法 + 给定的模块工具）。
pub(crate) fn test_notes(
    sb: &crate::capabilities::workspace::api::Sandbox,
    modules: &[crate::capabilities::modules::api::Module],
) -> crate::capabilities::tools::api::ToolNotes {
    crate::capabilities::tools::api::tool_notes(&test_prompts(), sb, modules, &[])
}

/// 目的：内存会话历史：供测试断言落盘与回放。
pub(crate) struct InMemoryHistory {
    pub(crate) metas: Mutex<BTreeMap<String, SessionMeta>>,
    pub(crate) events: Mutex<BTreeMap<String, Vec<serde_json::Value>>>,
    pub(crate) fail: Option<String>,
}
