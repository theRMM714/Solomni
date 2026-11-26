//! 目的：虚拟机档两件外部工具的位置事实——登记的覆盖值与项目内补齐的那一份。
//! 管：两份事实的优先级、"这次用的是哪一份"的如实标注，以及一条可以给界面看的现状（ToolReport）。
//! 不管：路径怎么解析（provision）、PATH 上叫什么名字（provision 按平台给）。
//! 联动：exec.rs 的就绪判据、work 的启动装配、system 的自检。

use serde::Serialize;

/// 目的：一个外部工具的两份位置事实；登记值优先，空登记 = 按项目内那份自动发现。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ToolPath {
    /// 目的：设置里登记的路径（已去空白；None = 没登记）。
    pub registered: Option<String>,
    /// 目的：依赖补齐装进项目内的那一份（绝对路径；None = 项目内也没有）。
    pub provisioned: Option<String>,
}

impl ToolPath {
    /// 目的：按"登记的原文 + 项目内那份"造一份事实；空串按没登记处理。
    pub fn new(registered: Option<&str>, provisioned: Option<&str>) -> ToolPath {
        ToolPath {
            registered: registered
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
            provisioned: provisioned
                .map(str::trim)
                .filter(|p| !p.is_empty())
                .map(str::to_string),
        }
    }

    /// 目的：交给运行期的确定路径——只取"用户指定或已装好"的那份，不拿 PATH 上的裸名冒充路径。
    /// 约束：None = 两份都没有，运行期按自己的 PATH 兜底找（与就绪判据的第三条路同一语义）。
    pub fn located(&self) -> Option<&str> {
        self.registered.as_deref().or(self.provisioned.as_deref())
    }

    /// 目的：这次该试哪一份——登记值优先，其次项目内那份；None = 只剩 PATH 兜底。
    pub fn pick(&self) -> Option<(&str, ToolSource)> {
        match (&self.registered, &self.provisioned) {
            (Some(p), _) => Some((p.as_str(), ToolSource::Registered)),
            (None, Some(p)) => Some((p.as_str(), ToolSource::Provisioned)),
            (None, None) => None,
        }
    }
}

/// 目的：这次实际用的那一份从哪来——界面与就绪文案共用同一套词，不各编各的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSource {
    /// 目的：设置里登记的路径（用户显式指定，永远优先）。
    Registered,
    /// 目的：依赖补齐装进项目内的那一份。
    Provisioned,
    /// 目的：PATH 上找到的那一个。
    Path,
}

impl ToolSource {
    /// 目的：给用户看的来源名。
    pub fn label(&self) -> &'static str {
        match self {
            ToolSource::Registered => "设置登记",
            ToolSource::Provisioned => "项目内补齐",
            ToolSource::Path => "PATH",
        }
    }
}

/// 目的：虚拟机档要用的两件外部工具（QEMU 与平台那一个共享 helper）的位置事实。
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct VmTools {
    /// 目的：虚拟机监视器（qemu）。
    pub qemu: ToolPath,
    /// 目的：本平台共享档要用的那一个（Windows = smbd；其余 = virtiofsd）。
    pub share: ToolPath,
}

/// 目的：一个工具这次的三种结局——用上确定的那份、只剩 PATH、都不行。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolState {
    /// 目的：确定路径（登记或项目内）在场且能执行。
    Ok,
    /// 目的：没有确定路径，但 PATH 上有这个可执行文件——运行期能兜底用上。
    Path,
    /// 目的：确定路径不在场/不能执行，PATH 上也没有；如实报缺，不假装可用。
    Missing,
}

/// 目的：一条外部工具的现状（登记了什么、项目内有没有、这次用哪份、能不能用）——界面与自检同读。
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolReport {
    /// 目的：稳定标识（qemu / share）。
    pub id: &'static str,
    /// 目的：给人看的名字。
    pub label: &'static str,
    /// 目的：PATH 兜底时要找的可执行文件名（裸名，不是路径）。
    pub binary: &'static str,
    pub registered: Option<String>,
    pub provisioned: Option<String>,
    /// 目的：这次会用的那份（登记值或项目内那份；None = 只剩 PATH）。
    pub using: Option<String>,
    /// 目的：这次用的是哪一份（PATH 兜底时是 Path；都不行时是 None）。
    pub source: Option<ToolSource>,
    pub state: ToolState,
}

impl ToolReport {
    /// 目的：按一份位置事实与探针，算出给界面看的现状。
    /// 约束：确定路径以**能执行**为准（登记了但不是可执行文件同样算缺），PATH 那条只问存在性。
    pub fn inspect(
        id: &'static str,
        label: &'static str,
        binary: &'static str,
        tool: &ToolPath,
        probe: &dyn crate::kernel::ports::HostProbe,
    ) -> ToolReport {
        let (using, source) = match tool.pick() {
            Some((p, s)) => (Some(p.to_string()), Some(s)),
            None => (None, None),
        };
        let state = match (&using, source) {
            (Some(p), Some(_)) if probe.is_executable(std::path::Path::new(p)) => ToolState::Ok,
            (None, _) if probe.has_exe(binary) => ToolState::Path,
            _ => ToolState::Missing,
        };
        let source = match state {
            ToolState::Path => Some(ToolSource::Path),
            ToolState::Missing if using.is_none() => None,
            _ => source,
        };
        ToolReport {
            id,
            label,
            binary,
            registered: tool.registered.clone(),
            provisioned: tool.provisioned.clone(),
            using,
            source,
            state,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registered_wins_and_empty_means_unset() {
        let t = ToolPath::new(Some("  "), Some("/p/qemu"));
        assert_eq!(t.pick(), Some(("/p/qemu", ToolSource::Provisioned)));
        let t = ToolPath::new(Some(" C:\\tools\\qemu.exe "), Some("/p/qemu"));
        assert_eq!(t.pick().map(|(p, _)| p), Some("C:\\tools\\qemu.exe"));
        assert_eq!(ToolPath::default().pick(), None);
    }

    /// 目的：交给运行期的只有"确定路径"——两份都没有时给 None，让运行期自己走 PATH，
    ///   不把裸名当路径（裸名交给按文件判存在的那一侧会被当成"不在场"）。
    #[test]
    fn located_never_hands_out_a_bare_name() {
        assert_eq!(ToolPath::default().located(), None);
        assert_eq!(
            ToolPath::new(Some("C:\\q\\qemu.exe"), None).located(),
            Some("C:\\q\\qemu.exe")
        );
        assert_eq!(
            ToolPath::new(None, Some("/p/qemu")).located(),
            Some("/p/qemu"),
            "登记为空时用项目内那份"
        );
    }

    /// 目的：现状的三种结局——登记的那份能执行算用上；登记了但不在场算缺（不偷偷回退）；
    ///   两份都没有而 PATH 有时如实说走 PATH。
    #[test]
    fn report_says_which_route_this_tool_actually_takes() {
        struct Probe {
            exe: &'static [&'static str],
            path: &'static [&'static str],
        }
        impl crate::kernel::ports::HostProbe for Probe {
            fn is_file(&self, p: &std::path::Path) -> bool {
                self.exe.iter().any(|e| std::path::Path::new(e) == p)
            }
            fn is_dir(&self, _: &std::path::Path) -> bool {
                false
            }
            fn canonicalize(&self, p: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
                Ok(p.to_path_buf())
            }
            fn host_metadata(
                &self,
                _: &std::path::Path,
            ) -> std::io::Result<crate::kernel::ports::HostMetadata> {
                Ok(crate::kernel::ports::HostMetadata { is_dir: false })
            }
            fn probe_writable(&self, _: &std::path::Path) -> std::io::Result<()> {
                Ok(())
            }
            fn has_exe(&self, name: &str) -> bool {
                self.path.contains(&name)
            }
            fn path(&self) -> std::ffi::OsString {
                std::ffi::OsString::new()
            }
            fn hypervisor_available(&self) -> bool {
                true
            }
        }
        let live = Probe {
            exe: &["/p/qemu"],
            path: &[],
        };
        let hit = ToolReport::inspect(
            "qemu",
            "QEMU",
            "qemu-system-x86_64",
            &ToolPath::new(Some("/p/qemu"), None),
            &live,
        );
        assert_eq!(
            (hit.state, hit.source),
            (ToolState::Ok, Some(ToolSource::Registered))
        );

        let dead = ToolReport::inspect(
            "qemu",
            "QEMU",
            "qemu-system-x86_64",
            &ToolPath::new(Some("/gone/qemu"), None),
            &live,
        );
        assert_eq!(
            dead.state,
            ToolState::Missing,
            "登记了不在场照实说缺，不偷偷用项目内那份"
        );
        assert_eq!(dead.using.as_deref(), Some("/gone/qemu"));

        let bare = ToolReport::inspect(
            "share",
            "共享 helper",
            "smbd",
            &ToolPath::default(),
            &Probe {
                exe: &[],
                path: &["smbd"],
            },
        );
        assert_eq!(
            (bare.state, bare.source),
            (ToolState::Path, Some(ToolSource::Path))
        );
        assert_eq!(bare.using, None, "PATH 那条不算确定路径");
    }
}
