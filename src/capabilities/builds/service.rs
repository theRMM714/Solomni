//! 目的：构建能力的用例实现——按模块声明在目标环境把产物编出来，并复验入口。
//! 管：本机档（宿主 + 产品围栏）与虚拟机档（guest + 编译包）两条构建路径、构建期能力选包、缓存命中、编完复验。
//! 不管：源指纹与状态文件的机制（`detail/fingerprint.rs`）、包库与引擎的机制（`packages` / `vm` / `provision`）、CLI 文案（`diagnostics`）。
//! 联动：`ProcessRunner` / `HostProbe` / `Log` 是 kernel 的共享机制；其余依赖走各能力的 api。

use crate::capabilities::builds::api::{Builds, Built, Target};
use crate::capabilities::builds::detail::fingerprint as fp;
use crate::capabilities::modules::api::{BuildDecl, Module};
use crate::capabilities::packages::api::{
    build_packages, diagnose_text, plan_host_requests, requests_for, Facts, Library, Packages,
    PinPolicy, Provider,
};
use crate::capabilities::provision::api::Provision;
use crate::capabilities::vm::api::{self as vm, ExecPlan, GuestRun, GuestSpec, MissingEntry, Vm};
use crate::kernel::api::{FenceSpec, Tier};
use crate::kernel::ports::{FenceHost, HostProbe, Log, ProcessRunner};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 目的：构建能力——持包库、虚拟机、补齐与执行器，按 `build:` 在目标环境编出模块产物。
pub struct BuildsService {
    root: PathBuf,
    packages: Arc<dyn Packages + Send + Sync>,
    vm: Arc<dyn Vm + Send + Sync>,
    provision: Arc<dyn Provision + Send + Sync>,
    runner: Arc<dyn ProcessRunner + Send + Sync>,
    fence: Arc<dyn FenceHost + Send + Sync>,
    probe: Arc<dyn HostProbe + Send + Sync>,
    log: Arc<dyn Log + Send + Sync>,
    /// 项目内基础工具链的只读安装根：本机档构建围栏要放行它们（编译器要读自己的 libexec / 动态库）。
    toolchain_roots: Vec<PathBuf>,
}

impl BuildsService {
    /// 目的：组合根专用——注入包库、虚拟机、补齐、执行器与工具链安装根（具体实现只在组合根 new）。
    /// 参数：`root` = 产品根；`packages` / `vm` / `provision` = 各能力的用例面；`runner` = 外部进程执行；`probe` = 宿主事实；`log` = 运行日志；`toolchain_roots` = 基础工具链安装根。
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        root: PathBuf,
        packages: Arc<dyn Packages + Send + Sync>,
        vm: Arc<dyn Vm + Send + Sync>,
        provision: Arc<dyn Provision + Send + Sync>,
        runner: Arc<dyn ProcessRunner + Send + Sync>,
        fence: Arc<dyn FenceHost + Send + Sync>,
        probe: Arc<dyn HostProbe + Send + Sync>,
        log: Arc<dyn Log + Send + Sync>,
        toolchain_roots: Vec<PathBuf>,
    ) -> BuildsService {
        BuildsService {
            root,
            packages,
            vm,
            provision,
            runner,
            fence,
            probe,
            log,
            toolchain_roots,
        }
    }

    /// 目的：目标环境的操作系统名（入口判据与产物形态按它分）。
    /// 错误：虚拟根不在场或画像不可用。
    fn target_os(&self, target: &Target) -> Result<String, String> {
        match target {
            Target::Host => Ok(std::env::consts::OS.to_string()),
            Target::Guest(guest) => {
                let dir = self.guest_dir(guest)?;
                let image = vm::classify_base(&dir, self.probe.as_ref())
                    .map_err(|why| format!("虚拟根「{}」不可用（{}）", guest, why))?;
                Ok(image.os.as_str().to_string())
            }
        }
    }

    /// 目的：虚拟根的目录（名字 → `.vm/guest/<名>`）；不在场如实拒绝。
    fn guest_dir(&self, guest: &str) -> Result<PathBuf, String> {
        let dir = vm::resolve_guest(&self.root, guest)
            .map_err(|e| format!("虚拟根「{}」解析失败：{}", guest, e))?;
        if !self.probe.is_dir(&dir) {
            return Err(format!(
                "本机没有虚拟根「{}」：先 solomni --provision --ensure --yes（或换一个虚拟根）",
                guest
            ));
        }
        Ok(dir)
    }

    /// 目的：这个模块在目标环境里还缺哪些入口（与建会话时的入口判据同一把尺子）。
    /// 参数：`module` = 模块；`target_os` = 目标平台。
    /// 返回：缺的入口（空 = 都在场）。
    fn entries_left(&self, module: &Module, target_os: &str) -> Vec<MissingEntry> {
        vm::missing_entries_for(std::slice::from_ref(module), target_os, &|p| {
            self.probe.is_executable(p)
                && (target_os == std::env::consts::OS || vm::runs_on_target(p, target_os))
        })
    }

    /// 目的：缓存命中吗——目标与源指纹都得一致，且入口都在场（否则等于没编）。
    fn fresh(
        &self,
        state_path: &Path,
        target: &str,
        target_os: &str,
        fingerprint: &str,
        module: &Module,
    ) -> bool {
        let Some(prev) = fp::read_state(state_path) else {
            return false;
        };
        prev.target == target
            && prev.fingerprint == fingerprint
            && self.entries_left(module, target_os).is_empty()
    }

    /// 目的：构建的收尾——复验入口、落状态；失败如实报（本机档与虚拟机档共用）。
    fn finish(
        &self,
        module: &Module,
        target: &str,
        target_os: &str,
        fingerprint: &str,
        state_path: &Path,
        output: &str,
    ) -> Result<(), String> {
        let left = self.entries_left(module, target_os);
        if !left.is_empty() {
            return Err(format!(
                "构建命令报了成功，但入口仍不在场：{}——脚本可能吞掉了失败（Windows 的 `if errorlevel` 对崩溃码不成立，用 `|| exit /b 1`），或产物写错了地方；检查 build.command{}",
                left.iter()
                    .map(|e| format!("{}（工具 {}）", e.command, e.tool))
                    .collect::<Vec<_>>()
                    .join("、"),
                tail(output)
            ));
        }
        fp::write_state(
            state_path,
            &fp::BuildState {
                module: module.manifest.id.clone(),
                target: target.to_string(),
                fingerprint: fingerprint.to_string(),
            },
        )
    }

    /// 目的：按声明跑一次构建（入口预判已由调用方做过）。
    /// 参数：`module` = 模块；`decl` = 构建声明；`target` = 目标环境；`target_os` = 目标平台；`net` = 放不放行出站网络。
    /// 返回：这次的事实；`skipped` = 缓存命中没编。
    fn run(
        &self,
        module: &Module,
        decl: &BuildDecl,
        target: &Target,
        target_os: &str,
        net: bool,
    ) -> Result<Built, String> {
        match target {
            Target::Host => self.host(module, decl, target_os, net),
            Target::Guest(guest) => self.guest(module, decl, guest, target_os, net),
        }
    }

    /// 目的：本机档构建——在宿主上按声明的构建命令编，围栏由产品给（只写模块目录与状态目录）。
    fn host(
        &self,
        module: &Module,
        decl: &BuildDecl,
        target_os: &str,
        net: bool,
    ) -> Result<Built, String> {
        let base = self.packages.base();
        let current_path = std::env::var_os("PATH").unwrap_or_default();
        let has_exe = |name: &str| self.probe.has_exe(name);
        let bin_ok = |path: &Path| self.probe.is_dir(path);
        let facts = Facts {
            has_exe: &has_exe,
            bin_ok: &bin_ok,
            current_path: &current_path,
            base: &base.active,
            host_fallback: &base.fallback,
        };
        let requests = requests_for(&decl.needs, &format!("build:{}", module.manifest.id));
        let plan = plan_host_requests(
            &requests,
            &BTreeMap::new(),
            &self.packages.library(),
            PinPolicy::First,
            &facts,
        );
        if !plan.problems.is_empty() {
            return Err(format!(
                "本机没有这些构建期能力：{}（装工具链、把适用于本机目标的包放进 runtimes/、自己编好放进来，或让作者发产物）",
                diagnose_text(&plan.problems)
            ));
        }
        let providers = providers_of(&plan);
        let fingerprint = fp::source(&module.root, decl, &providers)?;
        let state_path = fp::state_path(&self.root, &module.manifest.id);
        if self.fresh(&state_path, "host", target_os, &fingerprint, module) {
            return Ok(Built {
                module: module.manifest.id.clone(),
                target: "host".to_string(),
                skipped: true,
                output: String::new(),
            });
        }
        let state_dir = self.root.join(".vm").join("build-state");
        let home = state_dir.join(&module.manifest.id).join("home");
        std::fs::create_dir_all(&home).map_err(|e| format!("建状态目录失败：{}", e))?;
        // 构建也是一次调用：按次 lease → Windows 上独立容器身份，跑完自己的 profile/ACE 自己收。
        let agent = format!("build:{}", module.manifest.id);
        let fence = FenceSpec {
            agent: agent.clone(),
            lease: crate::kernel::api::call_lease(&agent),
            rw: vec![module.root.clone(), state_dir.join(&module.manifest.id)],
            ro: self.toolchain_roots.clone(),
            ro_tree: plan.roots.clone(),
            private: home,
            cwd: module.root.clone(),
            net,
        };
        self.log.info(
            "builds::host",
            &format!(
                "模块 {} 在宿主上编（出站网络：{}）",
                module.manifest.id, net
            ),
        );
        let out = self
            .runner
            .run(&fence, &decl.command, "{}", &[], plan.path.as_deref(), None);
        // 按次收尾：撤 ACE、删按次 profile 与临时目录；失败如实记录（残留由 reconcile 按死归属回收）。
        if let Err(e) = self.fence.release_call(&fence) {
            self.log
                .warn("builds::host", &format!("围栏按次收尾未完成：{}", e));
        }
        if !out.ok {
            return Err(format!("构建命令失败（目标 host）{}", tail(&out.output)));
        }
        self.finish(
            module,
            "host",
            target_os,
            &fingerprint,
            &state_path,
            &out.output,
        )?;
        Ok(Built {
            module: module.manifest.id.clone(),
            target: "host".to_string(),
            skipped: false,
            output: out.output,
        })
    }

    /// 目的：虚拟机档构建——在目标虚拟根里用它的编译包编，模块目录读写挂进去（产物直接落回宿主那份）。
    fn guest(
        &self,
        module: &Module,
        decl: &BuildDecl,
        guest: &str,
        target_os: &str,
        net: bool,
    ) -> Result<Built, String> {
        let dir = self.guest_dir(guest)?;
        let image = vm::classify_base(&dir, self.probe.as_ref())
            .map_err(|why| format!("虚拟根「{}」不可用（{}）", guest, why))?;
        let deps = self.provision.status().unwrap_or_default();
        let pick = |prefix: &str| {
            deps.iter()
                .find(|d| d.present && d.id.starts_with(prefix))
                .map(|d| d.probe.clone())
        };
        let Some(engine_path) = pick("vm.qemu.") else {
            return Err("没有可用的 QEMU：先 solomni --provision --ensure --yes".to_string());
        };
        let share_tool = if std::env::consts::OS == "windows" {
            pick("vm.smbd.")
        } else {
            pick("vm.virtiofsd.")
        };
        let platform = vm::guest_platform(image.os, image.arch);
        let eligible = Library::for_target(
            &self.packages.library(),
            &crate::capabilities::packages::api::Target::guest(&platform),
        );
        let (packages, missing) = build_packages(&decl.needs, &eligible, &image.runtimes);
        if !missing.is_empty() {
            return Err(format!(
                "guest「{}」里没有这些构建期能力，包库也没提供：{}（把适用于该 guest 目标的包放进 runtimes/，或给虚拟根装一个）",
                guest,
                missing.join("、")
            ));
        }
        let providers: Vec<String> = packages
            .iter()
            .map(|p| format!("{}@{}", p.id, p.version))
            .collect();
        let fingerprint = fp::source(&module.root, decl, &providers)?;
        let state_path = fp::state_path(&self.root, &module.manifest.id);
        if self.fresh(&state_path, guest, target_os, &fingerprint, module) {
            return Ok(Built {
                module: module.manifest.id.clone(),
                target: guest.to_string(),
                skipped: true,
                output: String::new(),
            });
        }
        let lease = format!("build-{}", module.manifest.id);
        let plan = ExecPlan {
            tier: Tier::Vm,
            net,
            guest: Some(guest.to_string()),
            packages,
        };
        let fence = FenceSpec {
            agent: format!("build:{}", module.manifest.id),
            lease: lease.clone(),
            rw: vec![module.root.clone()],
            ro: Vec::new(),
            ro_tree: Vec::new(),
            private: PathBuf::new(),
            cwd: module.root.clone(),
            net,
        };
        let mounts = vm::mounts_from(&plan, &fence, &image.bridge());
        let spec = GuestSpec {
            lease: lease.clone(),
            image: image.clone(),
            mounts,
            net,
            engine_path: Some(engine_path),
            share_path: share_tool.clone(),
            shares: vm::share_chain(
                std::env::consts::OS,
                share_tool.as_deref(),
                self.probe.as_ref(),
            ),
            seed: None,
            isolated: true,
        };
        self.log.info(
            "builds::guest",
            &format!(
                "模块 {} 在 guest「{}」里编（出站网络：{}）",
                module.manifest.id, guest, net
            ),
        );
        self.vm
            .guest_start(&spec)
            .map_err(|e| format!("起 guest 失败：{}", e))?;
        if !self
            .vm
            .guest_wait_agent(&lease, std::time::Duration::from_secs(15 * 60))
        {
            self.vm.guest_stop(&lease);
            return Err("等 guest 的 qemu-guest-agent 超时".to_string());
        }
        // 先把编译包接进 guest（prefix 的 bin 进 PATH、system 的载荷叠写进系统路径），再跑构建。
        for script in vm::package_setup_scripts(&plan) {
            let _ = self.vm.guest_run(
                &lease,
                &GuestRun {
                    command: script,
                    args_json: "{}".to_string(),
                    env: Vec::new(),
                    cwd: String::new(),
                    timeout_secs: 30,
                    output_limit: crate::capabilities::vm::api::OutputLimit::Head,
                },
            );
        }
        let out = self.vm.guest_run(
            &lease,
            &GuestRun {
                command: decl.command.clone(),
                args_json: "{}".to_string(),
                env: Vec::new(),
                cwd: module.root.display().to_string(),
                timeout_secs: 30,
                output_limit: crate::capabilities::vm::api::OutputLimit::Head,
            },
        );
        self.vm.guest_stop(&lease);
        if !out.ok {
            return Err(format!(
                "构建命令失败（目标 guest「{}」）{}",
                guest,
                tail(&out.output)
            ));
        }
        self.finish(
            module,
            guest,
            target_os,
            &fingerprint,
            &state_path,
            &out.output,
        )?;
        Ok(Built {
            module: module.manifest.id.clone(),
            target: guest.to_string(),
            skipped: false,
            output: out.output,
        })
    }
}

impl Builds for BuildsService {
    fn ensure(&self, module: &Module, target: &Target, net: bool) -> Result<Built, String> {
        let target_os = self.target_os(target)?;
        let left = self.entries_left(module, &target_os);
        let Some(decl) = module.manifest.build.clone() else {
            // 没有 build 段：入口在目标环境是否可用只能看入口本身。
            if left.is_empty() {
                return Ok(Built {
                    module: module.manifest.id.clone(),
                    target: target.label(),
                    skipped: true,
                    output: String::new(),
                });
            }
            return Err(format!(
                "模块 {} 的工具 {} 入口不在场：{}（模块没声明 build 段：要自带产物，或按模块 README 自己编一次）",
                module.manifest.id,
                left.first().map(|e| e.tool.as_str()).unwrap_or(""),
                left.first().map(|e| e.command.as_str()).unwrap_or("")
            ));
        };
        // 有 build 段：入口存在不等于**这个目标**的产物。只有构建状态明确属于当前目标才跳过；
        // 状态是别的目标（宿主产物进 guest）就交给 run 重编。状态缺失视为随仓自带产物，仍跳过。
        if left.is_empty() {
            let state_path = fp::state_path(&self.root, &module.manifest.id);
            match fp::read_state(&state_path) {
                Some(prev) if prev.target == target.label() => {
                    return Ok(Built {
                        module: module.manifest.id.clone(),
                        target: target.label(),
                        skipped: true,
                        output: String::new(),
                    });
                }
                Some(_) => return self.run(module, &decl, target, &target_os, net),
                None => {
                    return Ok(Built {
                        module: module.manifest.id.clone(),
                        target: target.label(),
                        skipped: true,
                        output: String::new(),
                    });
                }
            }
        }
        self.run(module, &decl, target, &target_os, net)
    }

    fn build(&self, module: &Module, target: &Target, net: bool) -> Result<Built, String> {
        let target_os = self.target_os(target)?;
        let Some(decl) = module.manifest.build.clone() else {
            return Err(format!(
                "模块 {} 没声明 build 段：在 module.yaml 里写 build.command（见 MODULE_SPEC 的「构建」）",
                module.manifest.id
            ));
        };
        self.run(module, &decl, target, &target_os, net)
    }
}

/// 目的：选型结果 → 指纹用的提供者清单（`能力=提供者`）。
fn providers_of(plan: &crate::capabilities::packages::api::PackagePlan) -> Vec<String> {
    plan.providers
        .iter()
        .map(|(cap, p)| match p {
            Provider::Base { id, version } | Provider::Package { id, version } => {
                format!("{}={}@{}", cap, id, version)
            }
            Provider::Host => format!("{}=host", cap),
        })
        .collect()
}

/// 目的：构建失败时把命令输出收成一段（没有输出就明说）。
fn tail(output: &str) -> String {
    let text = output.trim();
    if text.is_empty() {
        "：命令没有输出".to_string()
    } else {
        format!("：{}", text)
    }
}
