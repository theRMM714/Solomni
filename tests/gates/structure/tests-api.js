/**
 * 结构审查·用例不碰实现：src/tests 的用例只经**入站面**（`::api`）、**出站端口**（`::ports`）与组合根
 * （`crate::entry::assembly`）装配被测对象；不得直接引用 `::service` / `::domain` / `::detail`。
 * 目的：实现会频繁搬家，用例只钉**行为与旅程**；装配细节收敛到替身与组合根两处，搬家不再改用例。
 * 豁免：替身与装配脚手架（doubles / builders / prelude / fakes / detail 契约本身）——它们就是"怎么装"的那一层。
 * 另有 8 个**适配器边界用例**直接构造真实适配器（HttpFetch / SetupHost / FsWorkspace …），性质同 `src/tests/detail/`：
 * 它们触到实现分区是既有事实，收敛另行记账（`tests/gaps.yaml` 的 `tests.impl-partition-imports`），名单只减不增。
 * 判据来源：TESTING.md「测试只钉行为」与 ARCHITECTURE.md R10。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { mask } = require("../../lib/rust-mask.js");

// 替身与装配层：按单元装配被测对象本就是它们的职责；用例文件不在豁免内。
const SCAFFOLD = [
  "src/tests/doubles.rs",
  "src/tests/doubles/",
  "src/tests/builders/",
  "src/tests/fakes.rs",
  "src/tests/prelude.rs",
  "src/tests/detail.rs",
  "src/tests/detail/",
  "src/tests/helpers/",
];

// 记账在 tests/gaps.yaml 的 tests.impl-partition-imports：清掉一条就从这里删一行，新增文件不得进这张表。
const ADAPTER_CASES = [
  "src/tests/api/guest.rs",
  "src/tests/mcp.rs",
  "src/tests/permission.rs",
  "src/tests/provision/ports.rs",
  "src/tests/provision/service.rs",
  "src/tests/routes.rs",
  "src/tests/runtime.rs",
  "src/tests/setup.rs",
];

const IMPL_USE = /crate::units::(?:(?:pure|resource|business)::[a-z_]+|[a-z_]+::[a-z_]+)::(service|domain|detail)\b/;

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
      if (ADAPTER_CASES.includes(r)) continue;
      const lines = mask(fs.readFileSync(p, "utf8")).split(/\r?\n/);
      for (let i = 0; i < lines.length; i++) {
        const m = IMPL_USE.exec(lines[i]);
        if (!m) continue;
        problems.push(
          `${r}:${i + 1} 用例不得直接引用单元的实现分区（::${m[1]}::）——改走 ::api / ::ports 或组合根 crate::entry::assembly，装配细节放替身层：${lines[i].trim()}`
        );
      }
    }
  };
  walk(base);
  return problems;
}

module.exports = { testsApiOnly };