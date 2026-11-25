//! 目的：产品布局——安装根与数据根的分界、介质名与介质内的子路径，只在这里拼。
//! 管：两个根（install / data）、介质名，以及从安装根派生介质绝对路径。
//! 不管：介质里装什么、谁在什么时机建它们（归属方在各自的资源单元里）。
//! 联动：`src/main.rs`、`src/entry/assembly.rs`、存储回收、围栏与进程簿都从这里取路径；改布局只改本文件。

use std::path::{Path, PathBuf};

/// 目的：数据根目录名——产品自己的可变状态全部落在它下面，与开发环境、随包只读内容分开。
pub const DATA: &str = "data";

/// 目的：用户可编辑的设置与登记处目录名（数据根下）。
pub const HOME: &str = ".home";

/// 目的：台账目录名（数据根下）——进程簿、围栏授权账、锁与安装痕迹都落这里。
pub const STATE: &str = "state";

/// 目的：介质内部的锁目录名（台账下）。
pub const LOCKS: &str = "locks";

/// 目的：运行日志目录名（数据根下）。
pub const LOGS: &str = "logs";

/// 目的：会话记录目录名（数据根下）——转录、共享区与沙箱。
pub const SESSION: &str = "session";

/// 目的：虚拟机与构建缓存目录名（数据根下）。
pub const VM: &str = ".vm";

/// 目的：模块私有区目录名（数据根下；源码另在安装根的 `modules/`）。
pub const MODULES: &str = "modules";

/// 目的：运行包目录名（数据根下）——用户投放口。
pub const RUNTIMES: &str = "runtimes";

/// 目的：便携启动器目录名（安装根下）。
pub const BIN: &str = "bin";

/// 目的：提示词目录名（安装根下，随包只读）。
pub const PROMPTS: &str = "prompts";

/// 目的：系统工具目录名（安装根下，随包只读）。
pub const SYSTOOLS: &str = "systools";

/// 目的：界面语言目录名（安装根下，随包只读；运行期由编译期编入，不读盘）。
#[allow(dead_code)]
pub const LOCALES: &str = "locales";

/// 目的：一台 guest 的规格文件名，落在它自己的镜像目录里。
pub const GUEST_SPEC: &str = "guest.yaml";

/// 目的：数据根的绝对路径——产品可变状态的唯一落点。
/// 参数：`install` 安装根。
/// 返回：`install/data`。
pub fn data(install: &Path) -> PathBuf {
    install.join(DATA)
}

/// 目的：设置与登记处目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/.home`。
pub fn home(install: &Path) -> PathBuf {
    data(install).join(HOME)
}

/// 目的：台账目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/state`。
pub fn state(install: &Path) -> PathBuf {
    data(install).join(STATE)
}

/// 目的：锁目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/state/locks`。
pub fn locks(install: &Path) -> PathBuf {
    state(install).join(LOCKS)
}

/// 目的：运行日志目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/logs`。
pub fn logs(install: &Path) -> PathBuf {
    data(install).join(LOGS)
}

/// 目的：会话记录目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/session`。
pub fn session(install: &Path) -> PathBuf {
    data(install).join(SESSION)
}

/// 目的：虚拟机与构建缓存目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`data/.vm`。
pub fn vm(install: &Path) -> PathBuf {
    data(install).join(VM)
}

/// 目的：模块私有区根目录的绝对路径（`<id>/userdata`、`<id>/build` 落在它下面）。
/// 参数：`install` 安装根。
/// 返回：`data/modules`。
pub fn modules_data(install: &Path) -> PathBuf {
    data(install).join(MODULES)
}

/// 目的：运行包目录的绝对路径（用户投放口）。
/// 参数：`install` 安装根。
/// 返回：`data/runtimes`。
pub fn runtimes(install: &Path) -> PathBuf {
    data(install).join(RUNTIMES)
}

/// 目的：模块源码目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`install/modules`。
pub fn modules_src(install: &Path) -> PathBuf {
    install.join(MODULES)
}

/// 目的：便携启动器目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`install/bin`。
pub fn bin(install: &Path) -> PathBuf {
    install.join(BIN)
}

/// 目的：提示词目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`install/prompts`。
pub fn prompts(install: &Path) -> PathBuf {
    install.join(PROMPTS)
}

/// 目的：系统工具目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`install/systools`。
pub fn systools(install: &Path) -> PathBuf {
    install.join(SYSTOOLS)
}

/// 目的：界面语言目录的绝对路径。
/// 参数：`install` 安装根。
/// 返回：`install/locales`。
/// 约束：运行期没有读者（文案编译期编入）；只给 `assets` 的落点派生用。
#[allow(dead_code)]
pub fn locales(install: &Path) -> PathBuf {
    install.join(LOCALES)
}

/// 目的：一台 guest 的规格文件路径。
/// 参数：`dir` 该 guest 的镜像目录。
/// 返回：`dir/guest.yaml`。
pub fn guest_spec(dir: &Path) -> PathBuf {
    dir.join(GUEST_SPEC)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_derive_under_their_roots() {
        let install = Path::new("product");
        let root = install.join(DATA);
        assert_eq!(data(install), root, "可变状态全在一个数据根下");
        assert_eq!(state(install), root.join(STATE));
        assert_eq!(locks(install), root.join(STATE).join(LOCKS));
        assert_eq!(home(install), root.join(HOME));
        assert_eq!(session(install), root.join(SESSION));
        assert_eq!(vm(install), root.join(VM));
        assert_eq!(runtimes(install), root.join(RUNTIMES));
        assert_eq!(
            modules_data(install),
            root.join(MODULES),
            "私有区不在源码树里"
        );
        assert_eq!(modules_src(install), install.join(MODULES));
        assert_eq!(prompts(install), install.join(PROMPTS));
        assert_eq!(systools(install), install.join(SYSTOOLS));
        assert_eq!(locales(install), install.join(LOCALES));
        assert_eq!(guest_spec(&home(install)), root.join(HOME).join(GUEST_SPEC));
        assert_ne!(home(install), vm(install), "配置与镜像不共用一个目录");
        assert_ne!(home(install), session(install), "配置与会话不共用一个目录");
        assert_ne!(home(install), state(install), "配置与台账不共用一个目录");
    }
}
