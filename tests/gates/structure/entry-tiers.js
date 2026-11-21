/**
 * 结构审查·入口（entry）分层门禁：启动流程分四段——值层、判定层、装配层、编排层——只能依赖严格更低的段。
 * 登记表在 docs/entry/layers.yaml（唯一权威）：一行一条「模块: 层号」，模块名＝src/entry 下的文件名/目录名，
 * 编排根单列成 main（src/main.rs）；src/entry.rs 只是 mod 声明，不参与分层。
 * 判据：① src/entry 下的每个模块与 src/main.rs 必须在表里归层，表里的模块也必须在盘上——登记不许留静默空洞；
 *       ② 依赖边的目标段必须严格小于来源段（同段与向上都失败）——「同层禁止互相依赖」在这里落地，因此不可能成环；
 *       ③ 声明段号必须严格压住全部依赖（声明比最深依赖还低就失败）；
 *       ④ 成环照样显式检并报出环路径（②③ 都拦不住时才可能发生，报出来比装作没看见强）。
 * 与 kernel 机制表的差别：机制表的层号是「按依赖重算的最小层」这种事实缓存，写宽就失败；
 * 入口的段号表达**启动阶段职责**（值 → 判定 → 装配 → 编排），只要压得住依赖就合法，比最小值松不算错。
 * 只回执不判红：扇出（数字说明一处装配了多少模块）。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");
const { mask } = require("../../lib/rust-mask.js");
const { parseEntryTiersText, LABEL, TABLE: TABLE_NAME } = require("../../lib/entry-tiers.js");

const TABLE = "docs/entry/layers.yaml";
const ENTRY_DIR = path.join("src", "entry");
const BARREL = "src/entry.rs";
const MAIN = "main";

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

/** 磁盘上的入口模块 → 它的文件列表；编排根单列成 main。 */
function modulesOf(root) {
  const out = new Map();
  const dir = path.join(root, ENTRY_DIR);
  if (fs.existsSync(dir)) {
    for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
      const p = path.join(dir, e.name);
      if (e.isDirectory()) {
        const files = rsFilesOf(p);
        if (files.length) out.set(e.name, files);
      } else if (e.name.endsWith(".rs")) {
        out.set(e.name.replace(/\.rs$/, ""), [p]);
      }
    }
  }
  const main = path.join(root, "src", "main.rs");
  if (fs.existsSync(main)) out.set(MAIN, [main]);
  return out;
}

/** 读入口分层表并解析（解析本身在 tests/lib/entry-tiers.js，有自测）。 */
function parseEntryTiers(root) {
  const file = path.join(root, TABLE);
  if (!fs.existsSync(file)) {
    return { tiers: new Map(), problems: [LABEL + "：缺少 " + TABLE + "（入口分层的唯一权威）"] };
  }
  return parseEntryTiersText(fs.readFileSync(file, "utf8"));
}

/** 依赖边：模块 A 的文件引用了 crate::entry::<模块>（直接放在 src/entry 下的文件也认 super::<模块>）。
 *  剔除注释与测试模块；同一对模块只记首个命中文件与引用处数。 */
function edgesOf(root, modules) {
  const edges = new Map();
  const directDir = path.join(root, ENTRY_DIR);
  for (const [mod, files] of modules) {
    for (const f of files) {
      const text = mask(stripTestModules(fs.readFileSync(f, "utf8")));
      const direct = path.dirname(f) === directDir;
      const isMain = path.basename(f) === "main.rs" && path.dirname(f) === path.join(root, "src");
      const add = (to) => {
        if (to === mod || !modules.has(to)) return;
        const key = mod + ">" + to;
        if (edges.has(key)) edges.get(key).refs++;
        else edges.set(key, { file: f, refs: 1 });
      };
      for (const raw of text.split(/\r?\n/)) {
        if (!raw.trim() || raw.trim().startsWith("//")) continue;
        for (const m of raw.matchAll(/crate::entry::([a-z_][a-z0-9_]*)/g)) add(m[1]);
        // 花括号分组：crate::entry::{args, assembly}
        for (const m of raw.matchAll(/crate::entry::\{([^}]*)\}/g)) {
          for (const g of m[1].matchAll(/([a-z_][a-z0-9_]*)/g)) add(g[1]);
        }
        if (direct) for (const m of raw.matchAll(/\bsuper::([a-z_][a-z0-9_]*)/g)) add(m[1]);
        // 编排根在 src/main.rs 里写的是 mod entry; + entry::<模块>::…（没有 crate:: 前缀）
        if (isMain) for (const m of raw.matchAll(/(?<!:)entry::([a-z_][a-z0-9_]*)/g)) add(m[1]);
      }
    }
  }
  return edges;
}

/** 按现有依赖重算最小段号：A 依赖 B 则 A 至少比 B 高一段；叶子为 0。
 *  成环时把环路径收进 cycles 并停止抬段。 */
function minLayersOf(modules, edges) {
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
  const visit = (mod) => {
    const st = state.get(mod);
    if (st === 2) return layers.get(mod);
    if (st === 1) {
      cycles.push(stack.slice(stack.indexOf(mod)).concat(mod));
      return 0;
    }
    state.set(mod, 1);
    stack.push(mod);
    let top = -1;
    for (const d of deps.get(mod) || []) {
      if (!modules.has(d)) continue;
      const n = visit(d);
      if (n > top) top = n;
    }
    stack.pop();
    state.set(mod, 2);
    layers.set(mod, top + 1);
    return top + 1;
  };
  for (const mod of [...modules.keys()].sort()) visit(mod);
  return { layers, cycles };
}

/** src/entry.rs 只许放 mod 声明：入口模块清单一旦混进代码，就会绕开分层（它是 barrel，不参与分层）。 */
function barrelProblems(root) {
  const file = path.join(root, BARREL);
  if (!fs.existsSync(file)) return [];
  const problems = [];
  fs.readFileSync(file, "utf8")
    .split(/\r?\n/)
    .forEach((raw, i) => {
      const line = raw.trim();
      if (!line || line.startsWith("//") || /^pub mod [a-z_][a-z0-9_]*;$/.test(line)) return;
      problems.push(LABEL + "：" + BARREL + " 第 " + (i + 1) + " 行只能是 mod 声明或注释（入口模块清单不参与分层，别把代码放这里）");
    });
  return problems;
}

/** 入口分层判据：返回 problems（失败）与 notes（只报不拦）。 */
function entryTiers(root) {
  const { tiers, problems: tableProblems } = parseEntryTiers(root);
  const modules = modulesOf(root);
  const problems = tableProblems.concat(barrelProblems(root));
  for (const mod of [...modules.keys()].sort()) if (!tiers.has(mod)) problems.push(LABEL + "：" + ENTRY_DIR.split(path.sep).join("/") + " 的模块 " + mod + " 没有在 " + TABLE + " 里归层");
  for (const mod of [...tiers.keys()].sort()) if (!modules.has(mod)) problems.push(LABEL + "：" + TABLE + " 里的 " + mod + " 在盘上没有对应文件");
  if (problems.length) return { problems, notes: [] };

  const edges = edgesOf(root, modules);
  const { layers: need, cycles } = minLayersOf(modules, edges);
  if (cycles.length) {
    const seen = new Set();
    for (const c of cycles) {
      const key = c.slice().sort().join("|");
      if (seen.has(key)) continue;
      seen.add(key);
      problems.push(LABEL + "：模块成环 " + c.join(" → ") + "——同段不许互相依赖，只能向下；把其中一条边的一方挪到更低的段或断开它");
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
      const rel = b === a ? "同段互依赖" : "向上依赖";
      problems.push(
        LABEL + "：模块 " + from + "（第 " + a + " 段）" + rel + " " + to + "（第 " + b + " 段），见 " +
          info.file.slice(root.length + 1).replace(/\\/g, "/"),
      );
    }
  }
  for (const [mod, tier] of [...tiers].sort()) {
    const want = need.get(mod);
    if (tier < want) {
      problems.push(
        LABEL + "：" + mod + " 在 " + TABLE + " 声明第 " + tier + " 段，压不住它的依赖（至少要第 " + want + " 段）",
      );
    }
  }

  const notes = [];
  const rows = [...fanout].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
  if (rows.length) notes.push(LABEL + "：扇出（只回执，不设预算）：" + rows.map(([m, n]) => m + " " + n).join("、"));
  return { problems, notes };
}

module.exports = { entryTiers, modulesOf, edgesOf, minLayersOf, parseEntryTiers };

/** 命令行：打印入口分层表与按依赖重算的最小段号。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  const { tiers } = parseEntryTiers(root);
  const modules = modulesOf(root);
  const edges = edgesOf(root, modules);
  const { layers: need, cycles } = minLayersOf(modules, edges);
  console.log("段号  模块            依赖数  最小段号");
  for (const [mod, tier] of [...tiers].sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]))) {
    const n = [...edges.keys()].filter((k) => k.startsWith(mod + ">")).length;
    const want = need.get(mod);
    console.log(String(tier).padEnd(6) + mod.padEnd(16) + String(n).padEnd(8) + want + (want > tier ? "  ← 压不住依赖" : ""));
  }
  for (const c of cycles) console.log("环：" + c.join(" → "));
  const r = entryTiers(root);
  for (const p of r.problems) console.log("失败：" + p);
  for (const n of r.notes) console.log("回执：" + n);
  process.exit(r.problems.length ? 1 : 0);
}
