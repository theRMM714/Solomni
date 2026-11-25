/**
 * 结构审查·kernel 机制分层门禁：一个机制管一件事，只能依赖严格更低层的机制。
 * 登记表在 docs/kernel/mechanisms.yaml（唯一权威）：一行一条「机制: 层号」，机制名写成分区/名字
 * （contracts/note、domain/path、detail/procs），共享端口面单列成 ports。
 * 判据：① src/kernel 下的每个机制必须在表里归层，表里的机制也必须在盘上——登记不许留静默空洞；
 *       ② 依赖边的目标层必须严格小于来源层（同层与向上都失败）——「同层禁止互相依赖」在这里落地；
 *       ③ 声明层号必须等于按现有依赖重算的层号——表是事实的缓存，不是可以写宽的意图；
 *       ④ 机制之间不许成环——重算前先找环并报出环路径（成环时不再谈层号）；
 *       ⑤ 内核内部只走机制路径——src/kernel 下（除 api/ 面自身）不许出现 `crate::kernel::api`，
 *          对外面只留给单元与装配层；内核绕道面会让面与实现互相牵制。
 * ③ 的含义：动了一条边就重算全表（node tests/gates/structure/kernel-mechanisms.js --write 重算写回），
 * 让「这个机制为什么抬高」留在 diff 里；不想抬层就只能消掉那条边。
 * 只回执不判红：扇出（数字说明一个机制在替多少个机制装配）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");
const { parseMechanismsText } = require("../../lib/mechanisms.js");

const TABLE = "docs/kernel/mechanisms.yaml";
const AREAS = ["contracts", "domain", "detail"];
const PORTS = "ports";

/** 递归收集 .rs 文件（绝对路径）。 */
const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

/** 一个文件属于哪个机制：分区目录下的第一段（文件 = 机制本体，子目录同属该机制）。 */
function mechanismOf(file) {
  const parts = file.split(/[\\/]/);
  const at = parts.lastIndexOf("src");
  if (at < 0) return null;
  const area = parts[at + 2];
  if (!AREAS.includes(area) || parts[at + 3] === undefined) return null;
  return area + "/" + parts[at + 3].replace(/\.rs$/, "");
}

/** 磁盘上的机制 → 它的文件列表。 */
function mechanismsOf(root) {
  const out = new Map();
  const base = path.join(root, "src", "kernel");
  for (const area of AREAS) {
    const dir = path.join(base, area);
    if (!fs.existsSync(dir)) continue;
    for (const f of rsFilesOf(dir)) {
      const mech = mechanismOf(f);
      if (!mech) continue;
      if (!out.has(mech)) out.set(mech, []);
      out.get(mech).push(f);
    }
  }
  const ports = path.join(base, "ports.rs");
  if (fs.existsSync(ports)) out.set(PORTS, [ports]);
  return out;
}

/** 读机制表并解析（解析本身在 tests/lib/mechanisms.js，有自测）。 */
function parseMechanisms(root) {
  const file = path.join(root, TABLE);
  if (!fs.existsSync(file)) return { tiers: new Map(), problems: ["机制分层：缺少 " + TABLE + "（机制分层的唯一权威）"] };
  return parseMechanismsText(fs.readFileSync(file, "utf8"));
}

/** 测试专属模块：`#[cfg(test)] mod x;` 指的兄弟文件是测试代码，不是生产依赖边。 */
function testOnlyFiles(files) {
  const out = new Set();
  for (const f of files) {
    const text = fs.readFileSync(f, "utf8");
    for (const m of text.matchAll(/#\[cfg\(test\)\]\s*mod\s+(\w+)\s*;/g)) {
      const cand = path.join(f.replace(/\.rs$/, ""), m[1] + ".rs");
      if (fs.existsSync(cand)) out.add(cand);
    }
  }
  return out;
}

/** 依赖边：机制 A 的文件引用了 crate::kernel::<分区>::<机制>（剔除注释与测试模块）。
 *  同一对机制只记首个命中文件与引用处数；跟 kernel 其它机制的边才算机制边。 */
function edgesOf(root, mechs) {
  const edges = new Map();
  const testOnly = testOnlyFiles([...mechs.values()].flat());
  for (const [mech, files] of mechs) {
    for (const f of files) {
      if (testOnly.has(f)) continue;
      const text = mask(stripTestModules(fs.readFileSync(f, "utf8")));
      const add = (to) => {
        if (to === mech || !mechs.has(to)) return;
        const key = mech + ">" + to;
        if (edges.has(key)) edges.get(key).refs++;
        else edges.set(key, { file: f, refs: 1 });
      };
      for (const raw of text.split(/\r?\n/)) {
        if (!raw.trim() || raw.trim().startsWith("//")) continue;
        for (const m of raw.matchAll(/crate::kernel::(contracts|domain|detail)::([a-z_][a-z0-9_]*)/g)) add(m[1] + "/" + m[2]);
        // 花括号分组：crate::kernel::{contracts::note::X, ports::Log}
        for (const m of raw.matchAll(/crate::kernel::\{([^}]*)\}/g)) {
          for (const g of m[1].matchAll(/(contracts|domain|detail)::([a-z_][a-z0-9_]*)/g)) add(g[1] + "/" + g[2]);
          if (/(^|[\s{,])ports::/.test(m[1])) add(PORTS);
        }
        if (/crate::kernel::ports::/.test(raw)) add(PORTS);
      }
    }
  }
  return edges;
}

/** 内核内部绕道对外面的地方：src/kernel 下（除 api/ 面自身）出现 kernel::api。
 *  对外面是给单元与装配层用的；内核内部直取所属机制，面才不会被实现反向绑住。 */
function apiFaceRefs(root, mechs) {
  const hits = [];
  const files = [...new Set([...mechs.values()].flat())];
  const barrel = path.join(root, "src", "kernel.rs");
  if (fs.existsSync(barrel)) files.push(barrel);
  for (const f of files) {
    const text = mask(fs.readFileSync(f, "utf8"));
    text.split(/\r?\n/).forEach((raw, i) => {
      if (/kernel::api\b/.test(raw)) hits.push({ file: f, line: i + 1 });
    });
  }
  return hits;
}

/** 按当前依赖重算层号：A 依赖 B 则 A 至少比 B 高一层；叶子为 0。
 *  成环时把环路径收进 cycles 并停止抬层——环不是层号问题，不能靠数字糊过去。 */
function layersOf(mechs, edges) {
  const deps = new Map();
  for (const key of edges.keys()) {
    const [from, to] = key.split(">");
    if (!deps.has(from)) deps.set(from, []);
    deps.get(from).push(to);
  }
  const layers = new Map();
  const state = new Map();
  const stack = [];
  const cycles = [];
  const visit = (mech) => {
    const st = state.get(mech);
    if (st === 2) return layers.get(mech);
    if (st === 1) {
      cycles.push(stack.slice(stack.indexOf(mech)).concat(mech));
      return 0;
    }
    state.set(mech, 1);
    stack.push(mech);
    let top = -1;
    for (const d of deps.get(mech) || []) {
      if (!mechs.has(d)) continue;
      const n = visit(d);
      if (n > top) top = n;
    }
    stack.pop();
    state.set(mech, 2);
    layers.set(mech, top + 1);
    return top + 1;
  };
  for (const mech of [...mechs.keys()].sort()) visit(mech);
  return { layers, cycles };
}

/** 重算整张机制表：只按磁盘与边，与表里写过什么无关。 */
function recompute(root) {
  const mechs = mechanismsOf(root);
  return layersOf(mechs, edgesOf(root, mechs)).layers;
}

/** 把重算结果写回机制表：保留表头注释，按层号分组，每层一组「机制: 层号」。 */
function writeTable(root) {
  const layers = recompute(root);
  const file = path.join(root, TABLE);
  const head = fs.existsSync(file)
    ? fs.readFileSync(file, "utf8").split(/\r?\n/).filter((l) => /^#/.test(l) && !/^#\s*第\s*\d+\s*层/.test(l))
    : [];
  const rows = [...layers].sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]));
  const body = [];
  let tier = -1;
  for (const [mech, n] of rows) {
    if (n !== tier) {
      tier = n;
      body.push((body.length ? "\n" : "") + "# 第 " + tier + " 层");
    }
    body.push(mech + ": " + n);
  }
  fs.writeFileSync(file, head.concat(body).join("\n") + "\n");
  return layers;
}

/** 机制分层判据：返回 problems（失败）与 notes（只报不拦）。 */
function kernelMechanisms(root) {
  const { tiers, problems: tableProblems } = parseMechanisms(root);
  const mechs = mechanismsOf(root);
  const problems = tableProblems.slice();
  for (const mech of [...mechs.keys()].sort()) if (!tiers.has(mech)) problems.push("机制分层：src/kernel 的机制 " + mech + " 没有在 " + TABLE + " 里归层");
  for (const mech of [...tiers.keys()].sort()) if (!mechs.has(mech)) problems.push("机制分层：" + TABLE + " 里的 " + mech + " 在 src/kernel 下没有对应文件");
  if (problems.length) return { problems, notes: [] };

  for (const hit of apiFaceRefs(root, mechs)) {
    problems.push(
      "机制分层：" + hit.file.slice(root.length + 1).replace(/\\/g, "/") + " 第 " + hit.line +
        " 行绕道内核对外面 kernel::api——内核内部直取所属机制（crate::kernel::contracts|domain|detail::<机制>）",
    );
  }

  const edges = edgesOf(root, mechs);
  const { layers: need, cycles } = layersOf(mechs, edges);
  if (cycles.length) {
    const seen = new Set();
    for (const c of cycles) {
      const key = c.slice().sort().join("|");
      if (seen.has(key)) continue;
      seen.add(key);
      problems.push("机制分层：机制成环 " + c.join(" → ") + "——同层不许互相依赖，只能向下；把其中一条边的一方挪到更低层或断开它");
    }
    return { problems, notes: [] };
  }

  const fanout = new Map();
  for (const [key, info] of [...edges].sort()) {
    const [from, to] = key.split(">");
    fanout.set(from, (fanout.get(from) || 0) + 1);
    const a = tiers.get(from);
    const b = tiers.get(to);
    if (b >= a) {
      const rel = b === a ? "同层互依赖" : "向上依赖";
      problems.push(
        "机制分层：机制 " + from + "（第 " + a + " 层）" + rel + " " + to + "（第 " + b + " 层），见 " +
          info.file.slice(root.length + 1).replace(/\\/g, "/"),
      );
    }
  }
  for (const [mech, tier] of [...tiers].sort()) {
    const want = need.get(mech);
    if (want !== tier) {
      problems.push(
        "机制分层：" + mech + " 在 " + TABLE + " 声明第 " + tier + " 层，按现有依赖重算应为第 " + want + " 层" +
          "——动过边就重算全表：node tests/gates/structure/kernel-mechanisms.js --write；不想抬层就消掉那条边",
      );
    }
  }

  const notes = [];
  const rows = [...fanout].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
  if (rows.length) notes.push("机制分层：扇出（只回执，不设预算）：" + rows.map(([m, n]) => m + " " + n).join("、"));
  return { problems, notes };
}

module.exports = { kernelMechanisms, layersOf, mechanismsOf, edgesOf, apiFaceRefs, parseMechanisms, recompute, writeTable, mechanismOf };

/** 命令行：打印机制表与按依赖重算的层号；--write 把重算结果写回机制表。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  if (process.argv.includes("--write")) {
    const layers = writeTable(root);
    console.log("已按现有依赖重算 " + TABLE + "：" + layers.size + " 个机制，" + (Math.max(...layers.values()) + 1) + " 层");
  }
  const { tiers } = parseMechanisms(root);
  const mechs = mechanismsOf(root);
  const edges = edgesOf(root, mechs);
  const { layers: need, cycles } = layersOf(mechs, edges);
  const rows = [...tiers].sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]));
  console.log("层号  机制                          依赖数  重算层号");
  for (const [mech, tier] of rows) {
    const n = [...edges.keys()].filter((k) => k.startsWith(mech + ">")).length;
    const want = need.get(mech);
    console.log(String(tier).padEnd(6) + mech.padEnd(30) + String(n).padEnd(8) + want + (want !== tier ? "  ← 表与重算不一致" : ""));
  }
  for (const c of cycles) console.log("环：" + c.join(" → "));
  const r = kernelMechanisms(root);
  for (const p of r.problems) console.log("失败：" + p);
  for (const n of r.notes) console.log("回执：" + n);
  process.exit(r.problems.length ? 1 : 0);
}
