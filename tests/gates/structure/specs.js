/**
 * 结构审查·测试声明：`tests/specs/*.yaml` 是测试清单的**唯一声明处**，这里做三方对账。
 *
 * 为什么要有它：用例散在 Rust 文件与 e2e 里时，"某单元有哪些用例"只能靠文件名猜；
 * 重构搬目录会让清单静默失真（删掉的用例没人发现、声明的用例早就搬走）。
 * 对账方向（任一处不一致都判红）：
 *   1. 声明 → 实现：cargo 目标文件存在、旅程在 `tests/journeys/index.js` 里、平台目标目录存在；
 *   2. 实现 → 声明：`src/tests` 下带测试的文件、旅程清单里的每一段，都必须被某份 spec 声明；
 *   3. 驱动 → 旅程清单：驱动里 `await journey('x'` 的 id 必须就是清单里的 id（声明了不跑 = 空转）。
 */
"use strict";

const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const plan = require("../../lib/plan.js");
const { splitCaseId } = require("../../lib/spec.js");
const journeys = require("../../journeys/index.js");

const HAS_TEST = /#\[(?:tokio::)?test\]/;

/** src/tests 下带测试的文件（模块路径形态：tests::api::session）。 */
function testModules(root) {
  const base = path.join(root, "src", "tests");
  const out = [];
  const walk = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) {
        walk(p);
        continue;
      }
      if (!e.name.endsWith(".rs")) continue;
      if (!HAS_TEST.test(fs.readFileSync(p, "utf8"))) continue;
      const relPath = rel(root, p).replace(/^src[\\/]/, "").replace(/\.rs$/, "");
      out.push(relPath.split(/[\\/]/).join("::"));
    }
  };
  if (fs.existsSync(base)) walk(base);
  return out;
}

/** 驱动里真的存在的旅程段（`await journey('id', …`）。 */
function driverJourneys(root) {
  const file = path.join(root, "tests", "cross-platform", "e2e", "driver.js");
  if (!fs.existsSync(file)) return [];
  const out = [];
  for (const m of fs.readFileSync(file, "utf8").matchAll(/await journey\('([a-z0-9-]+)'/g)) out.push(m[1]);
  return out;
}

function specsGate(root) {
  const problems = [];
  const { entries, problems: parseProblems } = plan.collect(root);
  problems.push(...parseProblems);

  // 同单元内 id 重复：两条声明指向同一实现，必有其一过期。
  const seen = new Set();
  for (const e of entries) {
    const key = e.unit + "|" + e.id;
    if (seen.has(key)) problems.push("测试声明：" + e.spec + " 里 " + e.id + " 重复");
    seen.add(key);
  }
  problems.push(...plan.verify(root, entries));

  // 反向：带测试的文件必须被声明（否则删/搬了就没人知道）。
  const declared = [];
  for (const e of entries) {
    const s = splitCaseId(e.id);
    if (s && s.kind === "cargo") declared.push(s.target);
  }
  for (const mod of testModules(root)) {
    const full = mod; // testModules 已给出 tests:: 前缀的完整模块路径
    const covered = declared.some((t) => full === t || full.startsWith(t + "::"));
    if (!covered) {
      problems.push(
        "测试声明：src/" + mod.split("::").join("/") + ".rs 有用例但没有声明——在对应单元的 tests/specs/<单元>.yaml 里加一条 cargo/" + full
      );
    }
  }

  // 旅程双向对账：清单 ↔ 声明 ↔ 驱动。
  const declaredJourneys = new Set();
  for (const e of entries) {
    const s = splitCaseId(e.id);
    if (s && s.kind === "e2e") declaredJourneys.add(s.target);
  }
  for (const j of journeys.JOURNEYS) {
    if (!declaredJourneys.has(j.id)) problems.push("测试声明：旅程 " + j.id + " 没有出现在任何 tests/specs/*.yaml 里");
  }
  for (const id of declaredJourneys) {
    if (!journeys.BY_ID.has(id)) problems.push("测试声明：specs 里的旅程 " + id + " 不在 tests/journeys/index.js 里");
  }
  const inDriver = new Set(driverJourneys(root));
  for (const j of journeys.JOURNEYS) {
    if (!inDriver.has(j.id)) problems.push("测试声明：旅程 " + j.id + " 在驱动里没有对应的段（await journey('" + j.id + "', …)）——声明空转");
  }
  for (const id of inDriver) {
    if (!journeys.BY_ID.has(id)) problems.push("测试声明：驱动里的段 " + id + " 没登记进 tests/journeys/index.js");
  }
  return problems;
}

module.exports = { specsGate, testModules, driverJourneys };
