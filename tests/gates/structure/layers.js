/**
 * 结构审查·能力分层门禁：能力只能依赖严格更低层，同层不得互相依赖。
 * 分层表在 layers.yaml（唯一权威）：kernel 是 0，能力从 1 起，层号连续。
 * 判据：① src/capabilities/<名字>/ 必须归层，表里的名字也必须有目录——归层不许留静默空洞；
 *       ② 依赖边的目标层必须严格小于来源层（向上与同层都失败）——这是「业务保持向下依赖」的机器判定；
 *       ③ 声明层号必须等于按现有依赖重算的层号——表是分层事实的缓存，不是可以写宽的意图；
 *       ④ 层号从 1 起连续、kernel 为 0，不许用 T37 这类号把单元藏到没人注意的位置。
 * ③ 的含义：动了一条边就重算全表（node tests/gates/structure/layers.js --write 重算写回），
 * 让「这一层为什么抬高」留在 diff 里；不想重算就只能消掉那条边。
 * 只回执不判红的两类：纯转发边（只 pub use 再导出，不是真依赖）、扇出（不设预算，数字说明它在替多少个单元装配）。
 * 改表（合并 / 插入层）时用 node tests/gates/structure/layers.js 看当前表、边与重算结果。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { stripTestModules } = require("./dependency.js");

const TABLE = "layers.yaml";

const dirsOf = (root) => fs
  .readdirSync(path.join(root, "src", "capabilities"), { withFileTypes: true })
  .filter((e) => e.isDirectory())
  .map((e) => e.name)
  .sort();

const rsFilesOf = (dir) => {
  const out = [];
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...rsFilesOf(p));
    else if (e.name.endsWith(".rs")) out.push(p);
  }
  return out;
};

/** 解析分层表：一行一条「名字: 层号」，井号之后是注释。返回 Map 与表自身的格式问题。 */
function parseLayers(root) {
  const tiers = new Map();
  const problems = [];
  const file = path.join(root, TABLE);
  if (!fs.existsSync(file)) return { tiers, problems: ["分层：缺少 " + TABLE + "（能力分层的唯一权威）"] };
  let lineNo = 0;
  for (const raw of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    lineNo++;
    const line = raw.replace(/#.*$/, "").trim();
    if (!line) continue;
    const m = line.match(/^([a-z_][a-z0-9_]*):\s*(\d+)$/);
    if (!m) { problems.push("分层：" + TABLE + " 第 " + lineNo + " 行不是「名字: 层号」：" + raw.trim()); continue; }
    if (tiers.has(m[1])) { problems.push("分层：" + TABLE + " 里 " + m[1] + " 归了两次层"); continue; }
    tiers.set(m[1], Number(m[2]));
  }
  return { tiers, problems };
}

/** 层号形状：kernel 必须 0；能力层从 1 起连续。 */
function shapeProblems(tiers, dirs) {
  const problems = [];
  if (!tiers.has("kernel")) problems.push("分层：" + TABLE + " 必须给 kernel 归层 0");
  else if (tiers.get("kernel") !== 0) problems.push("分层：kernel 必须是第 0 层，实际 " + tiers.get("kernel"));
  const caps = [...tiers.keys()].filter((k) => k !== "kernel");
  for (const d of dirs) if (!tiers.has(d)) problems.push("分层：src/capabilities/" + d + "/ 没有在 " + TABLE + " 里归层");
  for (const c of caps) if (!dirs.includes(c)) problems.push("分层：" + TABLE + " 里的 " + c + " 在 src/capabilities/ 下没有目录");
  const nums = caps.map((c) => tiers.get(c)).filter((n) => n > 0).sort((a, b) => a - b);
  const max = nums.length ? nums[nums.length - 1] : 0;
  for (let n = 1; n <= max; n++) if (!nums.includes(n)) problems.push("分层：能力层号必须从 1 起连续，缺第 " + n + " 层");
  for (const c of caps) if (tiers.get(c) < 1) problems.push("分层：能力 " + c + " 的层号必须 ≥ 1，实际 " + tiers.get(c));
  return problems;
}

/** 依赖边：能力 A 的文件引用了 crate::capabilities::B（剔除注释与测试模块）。
 *  同一对边只记一处首个命中文件与引用处数（次数给结构图当粗细，判据不看它）；
 *  forward 记「这条边全是 pub use 再导出」——纯转发只回执，不当真依赖。 */
function edgesOf(root, dirs) {
  const edges = new Map();
  for (const cap of dirs) {
    for (const f of rsFilesOf(path.join(root, "src", "capabilities", cap))) {
      const text = stripTestModules(fs.readFileSync(f, "utf8"));
      for (const raw of text.split(/\r?\n/)) {
        if (!raw.trim() || raw.trim().startsWith("//")) continue;
        const forward = /^\s*pub\s+use\b/.test(raw);
        for (const m of raw.matchAll(/crate::capabilities::([a-z_][a-z0-9_]*)/g)) {
          const to = m[1];
          if (to === cap) continue;
          const key = cap + ">" + to;
          const hit = edges.get(key);
          if (hit) {
            hit.refs++;
            if (!forward) hit.forward = false;
          } else edges.set(key, { file: f, refs: 1, forward: forward });
        }
      }
    }
  }
  return edges;
}

/** 按当前依赖重算的最小层号：A 依赖 B 则 A 至少比 B 高一层。声明层号必须与它相等。 */
function minimalLayers(tiers, edges) {
  const deps = new Map();
  for (const key of edges.keys()) {
    const [from, to] = key.split(">");
    if (!deps.has(from)) deps.set(from, []);
    deps.get(from).push(to);
  }
  const memo = new Map();
  const open = new Set();
  const of = (cap) => {
    if (memo.has(cap)) return memo.get(cap);
    if (!tiers.has(cap) || cap === "kernel") return 0;
    if (open.has(cap)) return 0;
    open.add(cap);
    let top = 0;
    for (const d of deps.get(cap) || []) top = Math.max(top, of(d));
    open.delete(cap);
    memo.set(cap, top + 1);
    return top + 1;
  };
  const out = new Map();
  for (const cap of tiers.keys()) if (cap !== "kernel") out.set(cap, of(cap));
  return out;
}

/** 重算整表：给每个现有目录算最小可行层号（与表里写过什么无关）。 */
function recompute(root) {
  const dirs = dirsOf(root);
  const seed = new Map([["kernel", 0]]);
  for (const d of dirs) seed.set(d, 1);
  return minimalLayers(seed, edgesOf(root, dirs));
}

/** 把重算结果写回 layers.yaml：保留表头注释，按层号分组，每层一组 名字: 层号。 */
function writeTable(root) {
  const minimal = recompute(root);
  const file = path.join(root, TABLE);
  const head = fs.existsSync(file)
    ? fs.readFileSync(file, "utf8").split(/\r?\n/).filter((l) => /^#/.test(l))
    : [];
  const rows = [...minimal].sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]));
  const body = ["kernel: 0"];
  let tier = 0;
  for (const [cap, n] of rows) {
    if (n !== tier) {
      tier = n;
      body.push("", "# 第 " + tier + " 层");
    }
    body.push(cap + ": " + n);
  }
  fs.writeFileSync(file, head.concat(body).join("\n") + "\n");
  return minimal;
}

/** 分层判据：返回 problems（失败）与 notes（只报不拦）。 */
function layers(root) {
  const { tiers, problems: tableProblems } = parseLayers(root);
  const dirs = dirsOf(root);
  const problems = tableProblems.concat(shapeProblems(tiers, dirs));
  if (problems.length) return { problems, notes: [] };

  const edges = edgesOf(root, dirs);
  const minimal = minimalLayers(tiers, edges);
  const fanout = new Map();
  for (const [key, info] of [...edges].sort()) {
    const file = info.file;
    const [from, to] = key.split(">");
    fanout.set(from, (fanout.get(from) || 0) + 1);
    const a = tiers.get(from);
    const b = tiers.get(to);
    if (b >= a) {
      const rel = b === a ? "同层互依赖" : "向上依赖";
      problems.push("分层：能力 " + from + "（第 " + a + " 层）" + rel + " " + to + "（第 " + b + " 层），见 " + file.slice(root.length + 1).replace(/\\/g, "/"));
    }
  }
  // 声明层号必须等于重算层号：表是事实的缓存，写宽一层就等于让门禁放水。
  for (const [cap, tier] of [...tiers].filter(([k]) => k !== "kernel").sort()) {
    const need = minimal.get(cap);
    if (need !== tier) {
      problems.push(
        "分层：" + cap + " 在 " + TABLE + " 声明第 " + tier + " 层，按现有依赖重算应为第 " + need + " 层" +
          "——动过边就重算全表：node tests/gates/structure/layers.js --write；不想抬层就消掉那条边",
      );
    }
  }

  const notes = [];
  const forward = [...edges].filter(([, info]) => info.forward).map(([key]) => key.replace(">", " → ")).sort();
  if (forward.length) notes.push("分层：纯转发边（只 pub use 再导出，不当真依赖，只回执）：" + forward.join("、"));
  const rows = [...fanout].sort((x, y) => y[1] - x[1] || x[0].localeCompare(y[0]));
  if (rows.length) notes.push("分层：扇出（只回执，不设预算）：" + rows.map(([c, n]) => c + " " + n).join("、"));
  return { problems, notes };
}

module.exports = { layers, minimalLayers, recompute, writeTable, parseLayers, edgesOf, dirsOf, rsFilesOf };

/** 命令行：打印当前分层表与按依赖重算的层号；--write 把重算结果写回 layers.yaml。 */
if (require.main === module) {
  const root = path.resolve(__dirname, "..", "..", "..");
  if (process.argv.includes("--write")) {
    const minimal = writeTable(root);
    console.log("已按现有依赖重算 " + TABLE + "：" + minimal.size + " 个单元，" + Math.max(...minimal.values()) + " 层");
  }
  const { tiers } = parseLayers(root);
  const dirs = dirsOf(root);
  const edges = edgesOf(root, dirs);
  const minimal = minimalLayers(tiers, edges);
  const rows = [...tiers].filter(([k]) => k !== "kernel").sort((x, y) => x[1] - y[1] || x[0].localeCompare(y[0]));
  console.log("层号  单元          依赖数  重算层号");
  for (const [cap, tier] of rows) {
    const n = [...edges.keys()].filter((k) => k.startsWith(cap + ">")).length;
    const need = minimal.get(cap);
    console.log(String(tier).padEnd(6) + cap.padEnd(14) + String(n).padEnd(8) + need + (need !== tier ? "  ← 表与重算不一致" : ""));
  }
  const r = layers(root);
  for (const p of r.problems) console.log("失败：" + p);
  for (const n of r.notes) console.log("回执：" + n);
  process.exit(r.problems.length ? 1 : 0);
}
