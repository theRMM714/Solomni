//! 目的：系统维护与自检单元的用例实现——把各单元的用例面组合成启动相命令要的一次调用。
//! 管：SystemService 的依赖字段、System 的逐条实现、写操作的确认门槛与失败码（Fault）。
//! 不管：回执排版（在 presentation/cli/boot.rs）、端口适配器构造（在组合根）、各单元内部规则。
//! 联动：entry::assembly::system 注入依赖；presentation/cli/boot.rs 与 Web 的维护入口消费。

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::Value;

use crate::units::catalog::modules::api::Modules;
use crate::units::catalog::packages::api::BaseFacts;
use crate::units::maintenance::builds::api::{Builds, Built, Target};
use crate::units::maintenance::storage::api::{
    CleanKind, CleanOutcome, CleanRequest, GcReport, Storage,
};
use crate::units::provision::api::Provision;
use crate::units::pure::exec_model::api::JobState;
use crate::units::resource::sessions::api::Sessions;
use crate::units::system::api::{
    BuildRequest, EnsureRequest, Facts, Fault, FenceFacts, GcRequest, ResetRequest, SetupMode,
    SetupOps, System, WorkGcRequest,
};
use crate::units::workspace::api::{WorkGcReport, Workspace, safe_file_name};

/// 目的：系统维护单元的依赖面——全部由组合根注入（能力自己不构造别的单元的适配器）。
pub struct SystemDeps {
    /// 目的：产品根（自检要读 `data/.home` 与 `.tools` 的现场事实）。
    pub root: PathBuf,
    /// 目的：会话盘面（"有没有这个工作"要问会话盘的主人，不在业务层碰文件系统）。
    pub sessions: Arc<dyn Sessions>,
    pub storage: Arc<dyn Storage>,
    pub provision: Arc<dyn Provision>,
    pub builds: Arc<dyn Builds>,
    pub workspace: Arc<dyn Workspace>,
    pub modules: Arc<dyn Modules>,
    /// 目的：基础工具链事实（组合根在装配期算好；自检与启动报告共用）。
    pub toolchains: BaseFacts,
    /// 目的：全局出站网络开关（模块构建要不要放行由它与模块声明取交集）。
    pub net: bool,
    /// 目的：虚拟机档外部工具的位置事实——组合根注入（与界面同一份口径），自检现算不缓存。
    pub vm_tools: Arc<dyn Fn() -> crate::units::vm::api::VmTools + Send + Sync>,
}

/// 目的：系统维护与自检的唯一实现。
pub struct SystemService {
    deps: SystemDeps,
    /// 目的：直读宿主的那几项事实——能力不碰探针，只经这个端口取。
    facts: Arc<dyn Facts>,
    /// 目的：便携维护（用户 PATH 与围栏机器级预授）——写注册表与 ACL 的实现在 detail。
    setup: Arc<dyn SetupOps>,
}

impl SystemService {
    /// 目的：按组合根注入的依赖、宿主事实端口与便携维护端口装一次（实现由单元入口装配）。
    pub fn with_facts(
        deps: SystemDeps,
        facts: Arc<dyn Facts>,
        setup: Arc<dyn SetupOps>,
    ) -> SystemService {
        SystemService { deps, facts, setup }
    }

    /// 目的：按调用方给的本机围栏事实出自检（用例注入合成值，免得再触发一次围栏自检）。
    pub fn doctor_with(&self, fence: FenceFacts) -> Value {
        let tools = (self.deps.vm_tools)();
        self.facts.doctor(
            &self.deps.root,
            &tools,
            self.deps.storage.as_ref(),
            &self.deps.toolchains,
            fence,
        )
    }

    /// 目的：本机围栏能力事实（生产路径只取一次）。
    pub fn fence(&self) -> FenceFacts {
        self.facts.fence()
    }
}

impl System for SystemService {
    fn doctor(&self) -> Value {
        self.doctor_with(self.fence())
    }

    fn storage(&self) -> Result<Value, Fault> {
        let report = self.deps.storage.report().map_err(Fault::from)?;
        serde_json::to_value(report)
            .map_err(|e| Fault::failed(format!("空间报告序列化失败：{}", e)))
    }

    fn gc(&self, req: GcRequest) -> Result<CleanOutcome, Fault> {
        match req.kind.as_deref() {
            None => Ok(CleanOutcome::Report(
                self.deps.storage.gc(req.confirm).map_err(Fault::from)?,
            )),
            Some(kind) => {
                let kind = CleanKind::parse(kind).map_err(Fault::usage)?;
                let clean = CleanRequest {
                    kind,
                    id: req.guest,
                    module: req.module,
                    confirm: req.confirm,
                };
                self.deps.storage.clean(&clean).map_err(Fault::from)
            }
        }
    }

    fn provision(&self) -> Result<Value, Fault> {
        let deps = self.deps.provision.status().map_err(Fault::from)?;
        let missing: Vec<&str> = deps
            .iter()
            .filter(|d| !d.present)
            .map(|d| d.id.as_str())
            .collect();
        Ok(serde_json::json!({
            "platform": std::env::consts::OS,
            "arch": std::env::consts::ARCH,
            "missing": missing,
            "deps": deps,
        }))
    }

    fn ensure(&self, req: EnsureRequest, on_line: &mut dyn FnMut(&str)) -> Result<(), Fault> {
        if !req.confirm {
            return Err(Fault::usage(
                "会从网络下载并写进项目内。确认后加 --yes（先用 --provision 看清单）。",
            ));
        }
        match req.spec.as_deref() {
            Some(spec) if !spec.trim().is_empty() => self
                .deps
                .provision
                .ensure_spec(spec, on_line)
                .map_err(Fault::from),
            _ => self
                .deps
                .provision
                .ensure(&[], on_line)
                .map_err(Fault::from),
        }
    }

    fn reset(&self, req: ResetRequest, on_line: &mut dyn FnMut(&str)) -> Result<GcReport, Fault> {
        if req.guest.trim().is_empty() {
            return Err(Fault::usage("--reset 要给虚拟根名（清单里的 guest）"));
        }
        if !req.confirm {
            return Err(Fault::usage(
                "还原虚拟根会删除 guest 内状态并重建；确认后加 --yes",
            ));
        }
        let guest = req.guest;
        self.deps
            .provision
            .reset_guest(&guest, on_line)
            .map_err(Fault::from)?;
        let clean = CleanRequest {
            kind: CleanKind::BuildCache,
            id: Some(guest),
            module: None,
            confirm: true,
        };
        match self.deps.storage.clean(&clean) {
            Ok(CleanOutcome::Report(r)) => Ok(r),
            Ok(CleanOutcome::Started(_)) | Err(_) => Ok(GcReport::default()),
        }
    }

    fn work_gc(&self, req: WorkGcRequest) -> Result<WorkGcReport, Fault> {
        let work = safe_file_name(&req.work).map_err(Fault::usage)?;
        if !self.deps.sessions.exists(&work) {
            return Err(Fault::usage(format!("没有这个工作：{}", work)));
        }
        self.deps
            .workspace
            .work_gc(&work, req.confirm)
            .map_err(Fault::from)
    }

    fn build(&self, req: BuildRequest) -> Result<Built, Fault> {
        let roster = self.deps.modules.roster();
        let Some(module) = roster.modules.iter().find(|m| m.manifest.id == req.module) else {
            return Err(Fault::usage(format!(
                "没有模块「{}」（清单被拒收的话见 --doctor）",
                req.module
            )));
        };
        let Some(decl) = module.manifest.build.as_ref() else {
            return Err(Fault::usage(format!(
                "模块「{}」没声明 build 段（模块根 module.yaml 的 build:）",
                req.module
            )));
        };
        let target = match req.guest {
            Some(guest) => Target::Guest(guest),
            None => Target::Host,
        };
        let net = self.deps.net && decl.net;
        self.deps
            .builds
            .build(module, &target, net)
            .map_err(Fault::from)
    }

    fn job(&self) -> JobState {
        self.deps.storage.job()
    }

    fn setup(&self, mode: SetupMode, on_line: &mut dyn FnMut(&str)) -> Result<Value, Fault> {
        self.setup.apply(mode, on_line)
    }
}
