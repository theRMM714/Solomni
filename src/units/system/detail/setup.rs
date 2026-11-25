//! 目的：便携维护（`--setup`）的实现——PATH shim 与用户 PATH、围栏机器级一次性预授、只读状态。
//! 管：模式分派、状态视图（JSON）、围栏缺项对账；PATH 分段与 shim 内容在 path.rs / shim.rs，ACL 机制在 kernel 的 confine。
//! 不管：命令语法、回执前缀与退出码（在 presentation/cli/boot.rs）；模式名与服务语义的对应（在 api 的 SetupMode）。
//! 联动：SystemService 经 SetupOps 端口调用；entry 的启动对账用 fence_missing / fence_ensure；预授成功后经 FencePolicy 端口把授权记进设置（组合根接 entry::assembly::write_acl_policy）。

pub(crate) mod fence;
pub(crate) mod path;
pub(crate) mod shim;

use crate::kernel::api::layout;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::units::system::api::{Fault, FencePolicy, SetupMode, SetupOps};
use std::sync::Arc;

/// 目的：便携维护的实现——只持有产品根，所有落点都从它推出（`bin` 与 `.home`）。
pub(crate) struct SetupHost {
    root: PathBuf,
    /// 预授成功后记授权的那一笔：`--setup` 这一路由组合根接上；启动对账与用例不接。
    policy: Option<Arc<dyn FencePolicy>>,
    /// 本平台有没有外部 ACL 那一步：生产取 kernel 的事实，用例两档都走（与 entry::portable 同一写法）。
    acl_step: bool,
}

impl SetupHost {
    pub(crate) fn new(root: &Path) -> SetupHost {
        SetupHost {
            root: root.to_path_buf(),
            policy: None,
            acl_step: crate::kernel::api::platform::HAS_EXTERNAL_ACL_STEP,
        }
    }

    /// 目的：接上设置写入面：机器级预授成功的那一刻把授权记进设置，下次启动不再要授权。
    pub(crate) fn with_policy(mut self, policy: Arc<dyn FencePolicy>, acl_step: bool) -> SetupHost {
        self.policy = Some(policy);
        self.acl_step = acl_step;
        self
    }

    fn home(&self) -> PathBuf {
        layout::home(&self.root)
    }

    /// 目的：预授完成后把授权记进设置；没有 ACL 一步或没接端口时什么都不做。
    /// 约束：失败只降级成一行进度（预授本身已成功），不改退出码。
    fn record_authorization(&self, sink: &mut dyn FnMut(&str)) {
        if !self.acl_step {
            return;
        }
        let Some(policy) = &self.policy else {
            return;
        };
        match policy.record_authorization() {
            Ok(line) => sink(&line),
            Err(e) => sink(&format!("未写 sandbox.policy.windows.write_acl：{e}")),
        }
    }
}

impl SetupOps for SetupHost {
    fn apply(&self, mode: SetupMode, on_line: &mut dyn FnMut(&str)) -> Result<Value, Fault> {
        let mut lines: Vec<String> = Vec::new();
        {
            // 进度既进回执又当场交出：预授一条一条写 ACL 时人要看得见进度。
            let mut sink = |line: &str| {
                lines.push(line.to_string());
                on_line(line);
            };
            match mode {
                // 状态是只读的：回执是状态视图本身（键值），排版在呈现层，这里不出进度行。
                SetupMode::Status => return status_value(&self.root).map_err(Fault::failed),
                SetupMode::Path => sink(&path::path_line(&self.root).map_err(Fault::failed)?),
                SetupMode::Unset => sink(&path::unset_line(&self.root).map_err(Fault::failed)?),
                SetupMode::Undo => sink(&path::undo_line(&self.root).map_err(Fault::failed)?),
                SetupMode::Fence => {
                    fence::grant(&self.root, &mut sink).map_err(Fault::failed)?;
                    self.record_authorization(&mut sink);
                }
                SetupMode::All => {
                    sink(&path::path_line(&self.root).map_err(Fault::failed)?);
                    fence::grant(&self.root, &mut sink).map_err(Fault::failed)?;
                    self.record_authorization(&mut sink);
                }
            }
        }
        Ok(json!({ "mode": mode.id(), "lines": lines }))
    }

    fn fence_missing(&self) -> Result<Vec<String>, String> {
        fence::missing(&self.root)
    }

    fn fence_ensure(&self, on_line: &mut dyn FnMut(&str)) -> Result<(), String> {
        fence::ensure(&self.root, &self.home(), on_line)
    }
}

/// 目的：状态视图：平台、根、PATH/shim 事实、机器级预授缺项。只读，不改任何状态。
fn status_value(root: &Path) -> Result<Value, String> {
    let path = path::info(root);
    let fence = fence_view(fence::missing(root));
    Ok(json!({
        "platform": std::env::consts::OS,
        "root": root.display().to_string(),
        "path": path,
        "fence": fence,
    }))
}

/// 目的：把围栏缺项折成状态视图里的 fence 字段；本平台没有 ACL 一步时如实标 supported=false 并带原因。
fn fence_view(missing: Result<Vec<String>, String>) -> Value {
    match missing {
        Ok(missing) => json!({
            "supported": true,
            "ready": missing.is_empty(),
            "missing": missing,
            "error": Value::Null,
        }),
        Err(e) => json!({
            "supported": false,
            "ready": false,
            "missing": Vec::<String>::new(),
            "error": e,
        }),
    }
}

#[cfg(test)]
mod tests {
    /// 目的：状态视图里的围栏字段两档都成型——有 ACL 一步的平台给缺项清单，没有的如实标不可用并带原因。
    #[test]
    fn fence_view_covers_both_platforms() {
        use super::*;
        let ready = fence_view(Ok(Vec::new()));
        assert_eq!(ready["supported"], json!(true));
        assert_eq!(ready["ready"], json!(true));
        assert_eq!(ready["error"], Value::Null);
        let partial = fence_view(Ok(vec!["产品根读属性".to_string()]));
        assert_eq!(partial["ready"], json!(false));
        assert_eq!(partial["missing"], json!(["产品根读属性"]));
        let unsupported = fence_view(Err("本平台没有 ACL 一步".to_string()));
        assert_eq!(unsupported["supported"], json!(false));
        assert_eq!(unsupported["missing"], json!([]));
        assert_eq!(unsupported["error"], json!("本平台没有 ACL 一步"));
    }
}
