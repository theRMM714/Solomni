//! 目的：工作区过滤的**唯一**名单与匹配规则——`.workignore` 的语义、编译与判定。
//! 管：规则文件名与种子内容、把规则文本编译成与 git 同引擎的匹配器、按相对路径判定忽略。
//! 不管：遍历机制（各适配层用自己的目录遍历）；权限判定（沙箱与工具各自收口）。
//! 联动：内容面（初始导入、未跟踪清单、提交 / 同步的文件夹展开）与检索面（grep）都经本模块取规则；
//!   `.gitignore` 只在检索面由 `ignore` crate 自带，内容面不读它。

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use std::path::Path;

/// 目的：过滤规则文件名——放在版本库目录 `.work/` 下，随会话树共享，不进共享区也不进沙箱。
pub const WORKIGNORE: &str = ".workignore";

/// 目的：创建会话时播下的初始规则：VCS 元数据与依赖缓存（用户之后可经 `work_ignore` 增删）。
pub const WORKIGNORE_SEED: &str = "\
# 工作区内容过滤：本文件决定哪些路径不算工作区内容（初始导入、未跟踪清单、提交展开与 grep 都按它）。
# 语法同 .gitignore：一行一条规则；# 开头是注释；! 开头表示放行；/ 开头锚定工作区根；dir/ 只匹配目录。
# 由 work_ignore 工具维护；显式点名的单个文件路径不受它限制。
.git
.svn
.hg
node_modules
target
__pycache__
.venv
venv";

/// 目的：一份规则文本编译出的匹配器（语法与 git 同源，语义不自己发明）。
pub struct Rules {
    inner: Option<Gitignore>,
}

impl Rules {
    /// 目的：把 `.workignore` 文本编译成匹配器；空文本 = 不过滤任何路径。
    /// 错误：某行规则语法不合法时如实报错——不静默跳过用户的规则。
    pub fn compile(root: &Path, text: &str) -> Result<Rules, String> {
        if text.trim().is_empty() {
            return Ok(Rules { inner: None });
        }
        let mut builder = GitignoreBuilder::new(root);
        for (i, line) in text.lines().enumerate() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if let Err(e) = builder.add_line(None, line) {
                return Err(format!("过滤规则第 {} 行不合法（{}）：{}", i + 1, line, e));
            }
        }
        let gi = builder
            .build()
            .map_err(|e| format!("过滤规则不合法：{}", e))?;
        Ok(Rules { inner: Some(gi) })
    }

    /// 目的：判定一条相对路径（`/` 分隔、相对规则根）是否被忽略；目录按目录语义匹配。
    /// 约束：祖先是忽略目录时，其下所有路径都算忽略（与 git 一致，遍历据此剪掉整棵子树）。
    pub fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        match &self.inner {
            None => false,
            Some(gi) => matches!(
                gi.matched_path_or_any_parents(Path::new(rel), is_dir),
                ignore::Match::Ignore(_)
            ),
        }
    }
}
