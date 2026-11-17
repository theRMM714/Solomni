/**
 * 结构审查·用例不碰实现：src/tests 的用例只经**入站面**（`::api`）、**出站端口**（`::ports`）与组合根
 * （`crate::entry::assembly`）装配被测对象；不得直接引用 `::service` / `::domain` / `::detail`。
 * 目的：实现会频繁搬家，用例只钉**行为与旅程**；装配细节收敛到替身与组合根两处，搬家不再改用例。
 * 豁免：替身与装配脚手架（doubles / builders / prelude / fakes / detail 契约本身）——它们就是"怎么装"的那一层。
 * 判据来源：TESTING.md「测试只钉行为」与 ARCHITECTURE.md R10。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

// 替身与装配层：按能力装配被测对象本就是它们的职责；用例文件不在豁免内。
const SCAFFOLD = [
  "src/tests/doubles.rs",
  "src/tests/doubles/",
  "src/tests/builders/",
  "src/tests/fakes.rs",
  "src/tests/prelude.rs",
  "src/tests/detail.rs",
  "src/tests/helpers/",
];

const IMPL_USE = /crate::capabilities::[a-z_]+::[a-z_]+::(service|domain|detail)\b/;

function testsApiOnly(root) {
  const problems = [];
  const base = path.join(root, "src", "tests");
  if (!fs.existsSync(base)) return problems;
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) {
        walk(p);
        continue;
      }
      if (!e.name.endsWith(".rs")) continue;
      const r = rel(root, p);
      if (SCAFFOLD.some((s) => (s.endsWith("/") ? r.startsWith(s) : r === s))) continue;
      const lines = mask(fs.readFileSync(p, "utf8")).split(/\r?\n/);
      for (let i = 0; i < lines.length; i++) {
        const m = IMPL_USE.exec(lines[i]);
        if (!m) continue;
        problems.push(
          `${r}:${i + 1} 用例不得直接引用能力的实现分区（::${m[1]}::）——改走 ::api / ::ports 或组合根 crate::entry::assembly，装配细节放替身层：${lines[i].trim()}`
        );
      }
    }
  };
  walk(base);
  return problems;
}

module.exports = { testsApiOnly };
