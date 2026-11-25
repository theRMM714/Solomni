/**
 * 结构审查·依赖方向门禁：三条规则 + 归层不许有静默空洞。
 * 规则：① 业务/机制层不得反向依赖 adapters / presentation；② presentation 只经入站面（core::api）驱动；
 *       ③ 业务层内部不得成环（按强连通分量判定）。
 * 零豁免：七类违规集合与业务层依赖环必须为空——不读基线文件，没有先记账再治理的静默通道。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { unitOfPath, unitOfModule } = require("./unitpath.js");

// 模块级 #[cfg(test)] mod tests 不是生产依赖边，剔除；#[cfg(test)] 加在函数上的保留。
// 只剔**内联块**（`mod tests { … }`）：`mod tests;` 是另一个文件，块尾在别的文件里，
// 按「扫到下一个行首 }」会让它吞掉本文件后面所有生产代码，依赖边静默消失。
function stripTestModules(text) {
  const lines = text.split(/\r?\n/);
  const out = [];
  for (let i = 0; i < lines.length; i++) {
    if (/^\s*#\[cfg\(test\)\]\s*$/.test(lines[i])) {
      let j = i + 1;
      while (j < lines.length && lines[j].trim() === "") j++;
      if (j < lines.length && /^\s*(pub(\s*\([^)]*\))?\s+)?mod\s+\w+\s*\{\s*$/.test(lines[j])) {
        let k = j;
        while (k < lines.length && !/^\}/.test(lines[k])) k++;
        i = k;
        continue;
      }
    }
    out.push(lines[i]);
  }
  return out.join("\n");
}

function direction(root) {
  const problems = [];

  // 归一：单元路径截到「分区段」为止——::api / ::service / ::domain / ::detail / ::ports 是唯一合法入口与内部的判据段，
  // 分区段之前可以有任意层子业务（如 units/conversation/session/domain）。截断到分区段才能既认出 ::api，又认出 ::ports。
  const PARTITIONS = ["api", "service", "domain", "detail", "ports"];
  const normTarget = (full) => {
    const parts = full.split("::");
    if (parts[0] !== "units") return "crate::" + parts.slice(0, 2).join("::");
    const at = parts.findIndex((p, i) => i >= 2 && PARTITIONS.includes(p));
    const depth = at >= 0 ? at + 1 : 3;
    return "crate::" + parts.slice(0, Math.min(depth, parts.length)).join("::");
  };
  const LAYER_OF = (r) => {
    // 前端（交付机制）：各渠道一个顶层目录；不是业务单元。
    if (r === "src/presentation.rs" || r.startsWith("src/presentation/")) return "presentation";
    // 程序入口层：组合根 + 机器可读探针 + 围栏守门进程 + 入口层共用机制。它依赖所有人，任何人都不许依赖它。
    if (r === "src/main.rs" || r === "src/entry.rs" || r === "src/guard.rs" || r.startsWith("src/entry/") || r.startsWith("src/guard/")) return "entry";
    if (r === "src/kernel.rs" || r.startsWith("src/kernel/")) return "kernel";
    if (r === "src/units.rs" || r.startsWith("src/units/")) return "units";
    if (r === "src/tests.rs" || r.startsWith("src/tests/")) return "tests";
    if (r === "src/main.rs") return "main";
    return null;
  };
  // 层 → 它不得引用的层。端口由单元定义、它的 detail（与 kernel 的机制实现）实现，永不反向。
  const FORBIDDEN = {
    kernel: ["units", "presentation", "entry"],
    units: ["presentation", "entry"],
  };

  const rsFiles = [];
  const collectRs = (d) => {
    for (const e of fs.readdirSync(d, { withFileTypes: true })) {
      const p = path.join(d, e.name);
      if (e.isDirectory()) collectRs(p);
      else if (e.name.endsWith(".rs")) rsFiles.push(p);
    }
  };
  collectRs(path.join(root, "src"));

  // 测试专属模块：`#[cfg(test)] mod tests;` 指的兄弟文件是测试代码，不是生产依赖边。
  const testOnly = new Set();
  for (const abs of rsFiles) {
    const text = fs.readFileSync(abs, "utf8");
    for (const m of text.matchAll(/#\[cfg\(test\)\]\s*mod\s+(\w+)\s*;/g)) {
      const cand = path.join(abs.replace(/\.rs$/, ""), m[1] + ".rs");
      if (fs.existsSync(cand)) testOnly.add(cand);
    }
  }

  const reverse = new Set();
  const presentation = new Set();
  const apiOnly = new Set();
  const foreignImpl = new Set();
  const domainPorts = new Set();
  const apiPorts = new Set();
  const channelCross = new Set();
  const coreGraph = {};
  for (const abs of rsFiles) {
    const f = rel(root, abs);
    const layer = LAYER_OF(f);
    if (layer === null) {
      // 新分区建目录时必须先归层，否则它会绕过门禁——**不留静默空洞**。
      problems.push("依赖方向：未分类的源码目录（门禁需要先把它归层）：" + f);
      continue;
    }
    if (layer === "tests" || layer === "main") continue;
    if (testOnly.has(abs)) continue;
    const text = stripTestModules(fs.readFileSync(abs, "utf8"));
    for (const m of text.matchAll(/crate::([a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)*)/g)) {
      const t = normTarget(m[1]);
      const targetLayer = t.split("::")[1];
      // 渠道互不依赖：呈现层的文件不得引用**另一个渠道**（各自一个子目录，没有共享层）。
      if (layer === "presentation" && t.startsWith("crate::presentation::")) {
        const own = f.startsWith("src/presentation/cli/") ? "cli" : f.startsWith("src/presentation/web/") ? "web" : null;
        const other = t.split("::")[2];
        // 只判两个渠道之间：同一层根级的共享模块（如面袋 `faces`）不在此列。
        if (own && (other === "cli" || other === "web") && own !== other) channelCross.add(f + " -> " + t);
      }
      if ((FORBIDDEN[layer] || []).includes(targetLayer)) reverse.add(f + " -> " + t);
      // 呈现层只认入站面：某个单元的 ::api（顶层编排 runtime 也不例外）。
      const presOk = t.startsWith("crate::units::") && t.endsWith("::api");
      if (layer === "presentation" && !presOk && targetLayer !== "presentation") presentation.add(f + " -> " + t);
      // 业务之间只经对方的 api 交流（R1）；跨单元引用 ::ports / ::domain / ::detail 一律不许。
      if (layer === "units" && targetLayer === "units") {
        const selfCap = unitOfPath(f);
        const otherCap = unitOfModule(t);
        if (otherCap && otherCap !== selfCap && !t.endsWith("::api")) apiOnly.add(f + " -> " + t);
      }
      // domain 是纯逻辑：不得引用任何单元端口（自己的也不行，引了就不是纯的了）。
      if (f.includes("/domain/") && t.startsWith("crate::units::") && t.endsWith("::ports")) domainPorts.add(f + " -> " + t);
      // 端口只由定义它的单元持有（R12）：任何非入口层引用别的单元的 ports 都是违规。
      if (
        layer !== "entry" &&
        t.startsWith("crate::units::") &&
        t.endsWith("::ports") &&
        unitOfModule(t) !== (layer === "units" ? unitOfPath(f) : null)
      ) {
        apiOnly.add(f + " -> " + t);
      }
      // api.rs 是入站用例面：不得把单元端口再导出去。
      if (f.endsWith("/api.rs") && t.startsWith("crate::units::") && t.endsWith("::ports")) apiPorts.add(f + " -> " + t);
      // ::detail 是实现：跨单元引用一律不许，只有入口层的组合根能构造它。
      if (layer !== "entry" && t.startsWith("crate::units::") && t.endsWith("::detail") && unitOfModule(t) !== unitOfPath(f)) {
        apiOnly.add(f + " -> " + t);
      }
      // 环的节点：单元用 units/<名字>（协调业务与别的单元同处一张图）。
      const selfNode = "units/" + unitOfPath(f);
      const otherNode = targetLayer === "units" ? "units/" + unitOfModule(t) : null;
      if (otherNode && otherNode !== selfNode) {
        (coreGraph[selfNode] = coreGraph[selfNode] || new Set()).add(otherNode);
      }
    }
    // 不许给别的单元的类型写 impl（R1）：这是另一种"互相引入"，use 边看不见它。
    if (layer !== "entry") {
      const selfCap = layer === "units" ? unitOfPath(f) : null;
      const foreignPaths = (s) =>
        [...s.matchAll(/crate::([a-z_][a-z0-9_]*(?:::[a-z_][a-z0-9_]*)*)/g)]
          .map((m) => normTarget(m[1]))
          .filter((t2) => t2.startsWith("crate::units::") && unitOfModule(t2) !== selfCap);
      for (const line of text.split("\n")) {
        if (!/^impl\b/.test(line)) continue;
        // impl Trait for Type → 只看 Type（实现别人的 api trait 是正当的队列代理）；impl Type → 看 Type。
        const forIdx = line.indexOf(" for ");
        const traitPart = forIdx >= 0 ? line.slice(4, forIdx) : "";
        const typePart = forIdx >= 0 ? line.slice(forIdx + 5) : line.slice(4);
        for (const t2 of foreignPaths(typePart)) foreignImpl.add(f + " -> " + t2);
        // trait 侧只在显式引用了别人的 ports 时才算违规（那是替别人实现端口，R12 不许）。
        for (const t2 of foreignPaths(traitPart)) {
          if (t2.endsWith("::ports")) foreignImpl.add(f + " -> " + t2);
        }
      }
    }
  }

  // 强连通分量：> 1 个模块的分量 = 一个环。
  const coreSccs = [];
  {
    const idx = {}, low = {}, on = {}, stack = [];
    let counter = 0;
    const visit = (v) => {
      idx[v] = low[v] = counter++;
      stack.push(v);
      on[v] = true;
      for (const w of coreGraph[v] || []) {
        if (!(w in idx)) { visit(w); low[v] = Math.min(low[v], low[w]); }
        else if (on[w]) low[v] = Math.min(low[v], idx[w]);
      }
      if (low[v] === idx[v]) {
        const comp = [];
        let w;
        do { w = stack.pop(); on[w] = false; comp.push(w); } while (w !== v);
        if (comp.length > 1) coreSccs.push(comp.sort());
      }
    };
    for (const v of Object.keys(coreGraph)) if (!(v in idx)) visit(v);
  }
  const sortSccs = (list) => list.map((s) => s.slice().sort()).sort((a, b) => a.join(",").localeCompare(b.join(",")));

  // 零豁免：每类违规逐条列出，集合非空即失败。
  const expectEmpty = (items, hint) => {
    for (const item of items) problems.push("依赖方向：" + hint + "：" + item);
  };
  expectEmpty([...reverse].sort(), "业务层不得反向依赖呈现层或入口层");
  expectEmpty([...presentation].sort(), "presentation 只能经各单元的 ::api 驱动");
  expectEmpty([...apiOnly].sort(), "业务之间只能经对方的 ::api（R1）；::domain / ::detail 是实现，跨单元一律不许碰");
  expectEmpty([...foreignImpl].sort(), "不得给别的单元的类型写 impl（R1：另一种互相引入）");
  expectEmpty([...domainPorts].sort(), "domain 是纯逻辑：不得引用任何 ports");
  expectEmpty([...apiPorts].sort(), "api 是入站用例面：不得把本单元的 ports 再导出去（R12）");
  expectEmpty([...channelCross].sort(), "呈现层的两个渠道（cli / web）互不依赖：不得引用对方");

  for (const scc of sortSccs(coreSccs)) {
    problems.push("依赖方向：业务层内部出现依赖环（R1：只能经 ::api 单向协作）：" + scc.join(" -> ") + " -> " + scc[0]);
  }
  return problems;
}

module.exports = { direction, stripTestModules };
