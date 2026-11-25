/**
 * 资源名门禁：一块盘一个主——介质名只在 `src/kernel/domain/layout.rs` 拼，别处只经它取路径。
 * 判据：除布局模块与测试之外，`src` 里不得出现 `.join("<介质名>")` 这种直接拼接；介质名清单从
 *   layout.rs 的 `pub const` 读，不在本文件另抄一份（第二权威一出现，加介质就会漏判）。
 * 歧义名不判：`bin` / `modules` / `state` 也是 guest 镜像（`usr/bin`、`lib/modules`）与运行包内的目录名，
 *   靠字面量判不出它指产品介质还是别人的目录；产品侧一律经 `layout::bin` / `layout::modules_src` / `layout::modules_data`。
 * 注释里提到 `root/logs/` 不算命中：先剥测试模块，再用 tests/lib/rust-mask.js 抹注释（保留字面量）。
 * 零豁免：命中即失败，没有基线文件可销账。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { maskComments } = require("../../lib/rust-mask.js");

/** 布局模块：唯一允许写介质名的地方。 */
const LAYOUT = "src/kernel/domain/layout.rs";
/** 不判的常量名：它们的值在别人的目录树里也是常见名（见文件头）。 */
const AMBIGUOUS = ["BIN", "MODULES", "STATE", "GUEST_SPEC"];
/** 测试自己的夹具树，不按产品盘面判：`src/tests/`、名为 tests 的目录、`tests.rs` / `*_tests.rs`。 */
function isTestFile(f) {
  if (f === "src/tests.rs" || f.startsWith("src/tests/")) return true;
  const segs = f.split("/");
  if (segs.includes("tests")) return true;
  const base = segs[segs.length - 1];
  return base === "tests.rs" || base.endsWith("_tests.rs");
}

/** 目的：从布局模块读介质名（常量名 → 目录名），去掉歧义名。 */
function mediaNames(root) {
  const text = fs.readFileSync(path.join(root, LAYOUT), "utf8");
  const out = new Map();
  for (const m of text.matchAll(/pub const ([A-Z][A-Z0-9_]*): &str = "([^"]+)";/g)) {
    if (AMBIGUOUS.includes(m[1])) continue;
    out.set(m[2], m[1]);
  }
  return out;
}

function walk(root) {
  const files = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) {
        const f = rel(root, p);
        if (!isTestFile(f)) files.push(f);
      }
    }
  };
  walkDir(path.join(root, "src"));
  return files;
}

/** 目的：扫一遍，返回问题清单与一行说明（介质名个数与扫描面）。 */
function resourceNames(root) {
  const names = mediaNames(root);
  const problems = [];
  let scanned = 0;
  for (const f of walk(root)) {
    if (f === LAYOUT) continue;
    scanned++;
    const prod = maskComments(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    prod.split("\n").forEach((line, i) => {
      for (const m of line.matchAll(/\.join\(\s*"([^"]+)"\s*\)/g)) {
        if (!names.has(m[1])) continue;
        problems.push(
          "资源名：" + f + ":" + (i + 1) + " 直接拼介质名「" + m[1] + "」（" + names.get(m[1]) + "）——改走 kernel::api::layout 的对应函数",
        );
      }
    });
  }
  return {
    problems,
    notes: ["资源名：扫了 " + scanned + " 个源文件，判 " + names.size + " 个介质名（清单读自 " + LAYOUT + "）"],
  };
}

module.exports = { resourceNames };
