/**
 * 盘主门禁：一块盘一个主。`layout::<介质>()` 只许主人单元、组合根（entry / main）与内核机制调用；
 *   别的单元想要这块盘，走主人单元的 api 或端口，别自己拼路径。
 * 判据一：每个访问器都在主人表里（`src/kernel/domain/layout.rs` 新增访问器而不登记 → 红，防止门禁静默失效）。
 * 判据二：调用点所属单元 ∈ 该介质的允许集合（主人 / 组合根 / 内核；少数横向只读消费者逐个写明用途）。
 * 判据三：主人表里的访问器必须真的还在 `layout.rs`（改名后旧表会让门禁空转）。
 * 不判测试：`src/tests/` 的替身与夹具按 R10 直接造盘，不参与产品盘面归属。
 * 零豁免：命中即失败，没有基线文件可销账。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { stripTestModules } = require("./dependency.js");
const { maskComments } = require("../../lib/rust-mask.js");

const LAYOUT = "src/kernel/domain/layout.rs";

/** 主人表：访问器 → 主人单元 + 允许的其它调用单元（组合根 entry / main、内核机制、横向只读消费者）。 */
const OWNERS = {
  data: { owner: "layout", allow: ["kernel", "entry", "main"] },
  home: { owner: "home", allow: ["kernel", "entry", "main"] },
  state: { owner: "kernel", allow: ["kernel", "entry", "main", "system", "provision", "maintenance", "workspace"] },
  locks: { owner: "kernel", allow: ["kernel", "entry", "main", "workspace"] },
  logs: { owner: "kernel", allow: ["kernel", "entry", "main", "maintenance"] },
  session: { owner: "sessions", allow: ["kernel", "entry", "main", "maintenance"] },
  vm: { owner: "vmstore", allow: ["kernel", "entry", "main"] },
  modules_data: { owner: "content", allow: ["kernel", "entry", "main"] },
  runtimes: { owner: "content", allow: ["kernel", "entry", "main"] },
  modules_src: { owner: "catalog", allow: ["kernel", "entry", "main"] },
  bin: { owner: "system", allow: ["kernel", "entry", "main"] },
  prompts: { owner: "assets", allow: ["kernel", "entry", "main"] },
  systools: { owner: "assets", allow: ["kernel", "entry", "main"] },
  locales: { owner: "assets", allow: ["kernel", "entry", "main"] },
  guest_spec: { owner: "provision", allow: ["vm", "entry", "main"] },
};

/** 测试自己的夹具树，不按产品盘面判。 */
function isTestFile(f) {
  if (f === "src/tests.rs" || f.startsWith("src/tests/")) return true;
  const segs = f.split("/");
  return segs.includes("tests") || segs[segs.length - 1].endsWith("_tests.rs");
}

/** 目的：调用点属于哪个单元（`src/units/<单元>/…` / `kernel` / `entry` / `main`）。 */
function unitOf(file) {
  const segs = file.split("/");
  if (segs[0] !== "src") return null;
  if (segs[1] === "units" && segs.length >= 3) return segs[2].replace(/\.rs$/, "");
  if (segs[1] === "kernel") return "kernel";
  if (segs[1] === "entry" || segs[1] === "entry.rs") return "entry";
  return "main";
}

function walk(root) {
  const files = [];
  const walkDir = (d) => {
    if (!fs.existsSync(d)) return;
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walkDir(p);
      else if (e.name.endsWith(".rs")) files.push(rel(root, p));
    }
  };
  walkDir(path.join(root, "src"));
  return files;
}

/** 目的：扫一遍调用点，返回问题清单与一行说明。 */
function diskOwners(root) {
  const problems = [];
  const layoutText = fs.readFileSync(path.join(root, LAYOUT), "utf8");
  const declared = new Set([...layoutText.matchAll(/^pub fn ([a-z_][a-z0-9_]*)\(/gm)].map((m) => m[1]));
  for (const [acc, rule] of Object.entries(OWNERS)) {
    if (!declared.has(acc)) {
      problems.push("盘主：" + LAYOUT + " 里没有 " + acc + "()——本门禁的主人表过期了");
    }
  }
  let scanned = 0;
  for (const f of walk(root)) {
    if (isTestFile(f)) continue;
    scanned++;
    const prod = maskComments(stripTestModules(fs.readFileSync(path.join(root, f), "utf8")));
    const unit = unitOf(f);
    prod.split("\n").forEach((line, i) => {
      for (const m of line.matchAll(/layout::([a-z_][a-z0-9_]*)\s*\(/g)) {
        const acc = m[1];
        const rule = OWNERS[acc];
        if (!rule) {
          problems.push(
            "盘主：" + f + ":" + (i + 1) + " 用了没登记的访问器 layout::" + acc + "()——先在主人表登记它归谁",
          );
          continue;
        }
        if (unit !== rule.owner && !rule.allow.includes(unit)) {
          problems.push(
            "盘主：" +
              f +
              ":" +
              (i + 1) +
              " 用 layout::" +
              acc +
              "() 碰这块盘，但它不属于主人单元「" +
              rule.owner +
              "」——走主人单元的 api / 端口",
          );
        }
      }
    });
  }
  return {
    problems,
    notes: ["盘主：扫了 " + scanned + " 个源文件，判 " + Object.keys(OWNERS).length + " 块盘（主人表读自本文件）"],
  };
}

module.exports = { diskOwners };
