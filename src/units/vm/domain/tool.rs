//! 目的：虚拟机档两件外部工具的位置事实——登记的覆盖值与项目内补齐的那一份。
//! 管：两份事实的优先级，以及"这次用的是哪一份"的如实标注。
//! 不管：路径怎么解析（provision）、能不能执行（HostProbe）。
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
}
