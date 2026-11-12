//! 目的：守门进程的入参形状与命令行拼装（由主程序与探针共用）。
//! 管：FenceSpec 的扁平 JSON、`prepared` / `home` / `policy` 的缺省规则、launcher 与 `run_fenced` 的执行转交。
//! 不管：围栏怎么装、命令怎么跑——平台差异在 `linux.rs` / `macos.rs` / `windows/` / `other.rs`。
//! 联动：由 `confine.rs` 门面与根/探针的 `--fence-run` 消费；作业 JSON 的字段与 `contracts/fence.rs` 的 `FenceSpec` 同步。

use super::backend;
use super::FENCE_FLAG;
use crate::kernel::api::FenceSpec;
use std::path::{Path, PathBuf};
use std::process::Command;

/// 目的：守门进程的入参：conductor 的围栏策略 + **外层是否已把本机授权做完** + 产品私有区（台账落点）。
///   授权是改本机目录 ACL 的动作（只有 Windows 的容器围栏需要），所以它不进 conductor 的 `FenceSpec`，
///   由适配层随这次执行一起交给守门进程。JSON 是**扁平**的：FenceSpec 的字段同层再加 `prepared` 与 `home`。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FenceJob {
    pub spec: FenceSpec,
    pub prepared: bool,
    /// 目的：产品私有区（`.home/`）：守门进程把**它建过的容器 profile** 记进这里的台账，供 `--fence-clean` 精确回收。
    ///   守门进程是唯一真正建 profile 的地方，外层只知道"该建"、不知道"建成了"。
    ///   没有台账的调用方（探针）给 `None`：那类 profile 由 `--fence-clean` 的前缀清扫兜底。
    pub home: Option<PathBuf>,
    /// 目的：全局沙箱要求与“本进程是否已同意降级”——机制装不上时据此拒绝或降级。
    pub policy: crate::kernel::api::FenceRunPolicy,
}

impl FenceJob {
    pub fn to_json(&self) -> String {
        let mut fields = serde_json::to_value(&self.spec)
            .expect("FenceSpec 一定序列化为对象")
            .as_object()
            .cloned()
            .expect("FenceSpec 一定序列化为对象");
        fields.insert(
            "prepared".to_string(),
            serde_json::Value::Bool(self.prepared),
        );
        fields.insert(
            "home".to_string(),
            serde_json::to_value(&self.home).unwrap_or(serde_json::Value::Null),
        );
        fields.insert(
            "policy".to_string(),
            serde_json::to_value(self.policy).unwrap_or(serde_json::Value::Null),
        );
        serde_json::Value::Object(fields).to_string()
    }

    pub fn from_json(text: &str) -> Result<FenceJob, String> {
        #[derive(serde::Deserialize)]
        struct Raw {
            #[serde(flatten)]
            spec: FenceSpec,
            /// 缺了这一项 = 调用方没说清有没有授权，如实报错（不默认成"有"）。
            prepared: bool,
            /// 缺省 = 没有台账可落（探针、别的调用方）；这不是"忘记传"，所以允许缺省。
            #[serde(default)]
            home: Option<PathBuf>,
            /// 缺省 = 旧探针/旧调用方：按“允许降级”处理（不改变既有降级路径的语义）。
            #[serde(default)]
            policy: crate::kernel::api::FenceRunPolicy,
        }
        serde_json::from_str::<Raw>(text)
            .map(|raw| FenceJob {
                spec: raw.spec,
                prepared: raw.prepared,
                home: raw.home,
                policy: raw.policy,
            })
            .map_err(|e| format!("围栏参数非法：{}", e))
    }
}

/// 目的：组装守门进程的命令行：工具命令作为**数据**传递（不拼进 shell 字符串，杜绝注入）。
pub fn launcher(exe: &Path, job: &FenceJob, command: &str) -> Command {
    let mut cmd = Command::new(exe);
    cmd.arg(FENCE_FLAG)
        .arg(job.to_json())
        .arg("--")
        .arg(command);
    cmd
}

/// 目的：守门进程内：装围栏 → 跑命令 → 返回退出码。失败必须报错（stderr）并用 FENCE_FAILED 退出。
pub fn run_fenced(job: &FenceJob, command: &str) -> i32 {
    backend::run_fenced(
        &job.spec,
        job.prepared,
        job.home.as_deref(),
        command,
        &job.policy,
    )
}
