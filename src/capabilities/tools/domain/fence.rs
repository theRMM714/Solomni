//! 目的：从该 agent 的沙箱 / 无会话场景**派生**围栏描述符（策略在这里，机制与类型在 kernel）。
//! 管：`FenceSpec::from_sandbox` / `FenceSpec::standalone`——把 `Sandbox`、模块写授权与 `userdata/` 事实变成 `FenceSpec`。
//! 不管：描述符自身的形状与平台机制（在 `kernel::domain::fence` 与 `kernel::detail::confine`）。
//! 联动：经 `tools::api` 给协调业务与协作会话用；kernel 不反向依赖本模块。

use crate::kernel::api::FenceSpec;

use crate::capabilities::workspace::api::Sandbox;
use std::path::{Path, PathBuf};

impl FenceSpec {
    /// 目的：从该 agent 的沙箱派生（模块目录按模块 id 升序，顺序稳定；同一模块不会同属两个 agent）。
    pub fn from_sandbox(sb: &Sandbox, net: bool) -> FenceSpec {
        // 共享主副本只有在**这一席可写**时才进 rw：agent 会话默认只读，
        // 于是模块外部工具进程也读不到 / 写不了未拉取进沙箱的主副本内容。
        let mut rw = Vec::new();
        let mut ro_tree = Vec::new();
        if sb.shared_writable {
            rw.push(sb.shared.clone());
        }
        rw.push(sb.private.clone());
        // 模块目录默认**只读**：只有 module_write 命中该模块才整块可写。
        // <module>/userdata 派不派，看工作区扫描注入的事实（domain 不读盘）。
        for (id, root) in &sb.modules {
            if sb.permissions.module_write_ok(id) {
                rw.push(root.clone());
            } else {
                ro_tree.push(root.clone());
                if sb.modules_with_userdata.contains(id) {
                    rw.push(root.join("userdata"));
                }
            }
        }
        rw.sort();
        rw.dedup();
        ro_tree.sort();
        ro_tree.dedup();
        FenceSpec {
            agent: sb.agent.clone(),
            lease: sb.session.clone(),
            rw,
            ro: Vec::new(),
            ro_tree,
            private: sb.private.clone(),
            cwd: PathBuf::new(),
            net,
        }
    }

    /// 目的：**无会话**（人直接跑一个模块工具）的围栏——模块目录递归只读 + 它自己的 `userdata/`（事实说有才）可写，外加用户指定的工作目录（缺省 = `userdata/`，没有则退回模块根）。
    /// 参数：has_userdata 是工作区扫描给出的"该模块有没有 `userdata/`"事实（domain 不读盘）。
    /// 约束：与 agent 会话同一条围栏口径——不装机制时只留进程树与环境白名单，如实降级。
    pub fn standalone(
        module_root: &Path,
        work_root: Option<&Path>,
        has_userdata: bool,
    ) -> FenceSpec {
        let userdata = module_root.join("userdata");
        // 缺省工作目录 = 模块的 userdata（事实说有才用）；否则退回 cwd（模块根），不派不存在的落点。
        let private = match work_root {
            Some(p) => p.to_path_buf(),
            None if has_userdata => userdata.clone(),
            None => PathBuf::new(),
        };
        let mut rw: Vec<PathBuf> = Vec::new();
        if !private.as_os_str().is_empty() {
            rw.push(private.clone());
        }
        if has_userdata && private != userdata {
            rw.push(userdata);
        }
        rw.sort();
        rw.dedup();
        FenceSpec {
            agent: "user".to_string(),
            // 无会话（人直接跑模块工具）：没有会话租约可挂。
            lease: String::new(),
            rw,
            ro: Vec::new(),
            ro_tree: vec![module_root.to_path_buf()],
            private,
            cwd: module_root.to_path_buf(),
            net: false,
        }
    }
}
