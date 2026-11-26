/**
 * 结构审查·呈现层入站面门禁：每屏只许取清单里登记过的单元，且同层不许互相依赖。
 * 规则：① 清单（docs/presentation/inbound.yaml）是每屏入站面的唯一权威，源码与清单**双向比对**：
 *        源码取了清单外的单元＝入站面外溢（要放行就改清单），清单登记了但源码没取＝清单过期；
 *       ② 呈现层内部只许向上取根（presentation.rs / faces.rs / signal.rs / cli.rs / web.rs），
 *        兄弟文件之间不得互相引用（同层互依赖＝将来成环的入口）。
 * 零豁免：不读基线文件，违规集合非空即失败。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");
const { parseInboundText } = require("../../lib/presentation.js");
const { BUCKETS } = require("./unitpath.js");

const MANIFEST = "docs/presentation/inbound.yaml";
const SRC = "src/presentation";
// 呈现层内部允许被任何一屏取用的根文件：渠道入口、面袋，与两渠道共用的进程级 Ctrl+C。
const ROOTS = new Set([
  "src/presentation.rs",
  "src/presentation/faces.rs",
  "src/presentation/signal.rs",
  "src/presentation/cli.rs",
  "src/presentation/web.rs",
]);
// 分区段：单元路径截到它为止才是「单元」——::api 之前那段才是子单元名。
const PARTITIONS = ["api", "service", "domain", "detail", "ports"];

/** 目的：把 use 路径折成入站面单元名（\`work\` / \`catalog/registry\`）；桶那层不算单元名（桶名以 unitpath.js 为准）。 */
function unitOf(rest) {
  const seg = rest.split("::");
  if (BUCKETS.has(seg[0])) seg.shift();
  if (seg.length >= 2 && !PARTITIONS.includes(seg[1])) return seg[0] + "/" + seg[1];
  return seg[0];
}

/** 目的：把 \`presentation::cli::width::cols\` 这样的一串模块段解析回磁盘上的文件。 */
function fileOfModule(root, segs) {
  for (let n = segs.length; n >= 1; n--) {
    const cand = path.join(root, "src", segs.slice(0, n).join(path.sep) + ".rs");
    if (fs.existsSync(cand)) return path.relative(root, cand).split(path.sep).join("/");
  }
  return null;
}

function presentationInbound(root) {
  const problems = [];
  const notes = [];
  const manifestPath = path.join(root, MANIFEST);
  if (!fs.existsSync(manifestPath)) {
    return { problems: ["呈现入站面：缺少 " + MANIFEST + "（呈现层每屏入站面的唯一权威）"], notes };
  }
  const parsed = parseInboundText(fs.readFileSync(manifestPath, "utf8"));
  problems.push(...parsed.problems);

  // 盘上的呈现层源码 → 去掉测试专属文件（\`#[cfg(test)] mod tests;\` 指的是测试代码，没有生产入站面）。
  const files = [];
  (function walk(d) {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) walk(p);
      else if (e.name.endsWith(".rs")) files.push(path.relative(root, p).split(path.sep).join("/"));
    }
  })(path.join(root, SRC));
  const testOnly = new Set();
  for (const f of files) {
    const text = fs.readFileSync(path.join(root, f), "utf8");
    for (const m of text.matchAll(/#\[cfg\(test\)\]\s*mod\s+(\w+)\s*;/g)) {
      const cand = f.replace(/\.rs$/, "") + "/" + m[1] + ".rs";
      if (files.includes(cand)) testOnly.add(cand);
    }
  }

  const used = new Map();
  for (const f of files) {
    if (testOnly.has(f)) continue;
    const text = stripTestModules(fs.readFileSync(path.join(root, f), "utf8"));
    const units = new Set();
    for (const m of text.matchAll(/crate::units::([a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)*)/g)) {
      units.add(unitOf(m[1]));
    }
    if (/crate::kernel::api/.test(text)) units.add("kernel");
    // 同层互依赖：只许取根文件，兄弟文件之间的引用一律不认。
    for (const m of text.matchAll(/crate::presentation(?:::[a-z_][a-z0-9_]*)*/g)) {
      const segs = m[0].split("::").slice(1);
      const target = fileOfModule(root, segs);
      if (target === null || target === f) continue;
      if (ROOTS.has(target)) continue;
      problems.push("呈现入站面：同层互依赖（" + f + " → " + m[0] + "）：呈现层内部只能取根 " + [...ROOTS].join(" / "));
    }
    if (units.size > 0) used.set(f, [...units].sort());
  }

  // 双向比对：外溢与过期都算问题——清单必须是每屏入站面的当前实况。
  for (const [f, units] of [...used].sort()) {
    const allowed = parsed.faces.get(f) || [];
    for (const u of units) {
      if (!allowed.includes(u)) {
        problems.push("呈现入站面：" + f + " 取了清单外的单元 " + u + "（要放行先改 " + MANIFEST + "）");
      }
    }
    for (const u of allowed) {
      if (!units.includes(u)) {
        problems.push("呈现入站面：" + f + " 在清单里登记了 " + u + "，但源码没有取（清单过期）");
      }
    }
  }
  for (const f of [...parsed.faces.keys()].sort()) {
    if (!used.has(f)) problems.push("呈现入站面：清单登记的 " + f + " 在盘上不存在或不再取任何单元（删掉这一行）");
  }

  // 回执：清单里有几屏、最宽的一屏取几个单元——宽就是「这屏知道得太多」，只报不拦。
  const widest = [...used].sort((a, b) => b[1].length - a[1].length).slice(0, 3);
  notes.push(
    "呈现入站面：清单 " +
      parsed.faces.size +
      " 屏 / 实际取单元的 " +
      used.size +
      " 屏；最宽：" +
      widest.map(([f, u]) => f.replace("src/presentation/", "") + " " + u.length).join("、")
  );
  return { problems, notes };
}

module.exports = { presentationInbound, unitOf, fileOfModule };
