/**
 * 结构审查·单元桶门禁：`src/units/` 下只许三个角色桶，每个单元恰好住一个桶。
 * 判据：① 顶层只许 `pure` / `resource` / `business` 三个目录，且各自有同名 `.rs`（`src/units/<桶>.rs`）；
 *       ② 桶内只许单元目录，每个单元目录有同名 `.rs`，且桶的 `.rs` 正好声明这些单元的 `pub mod`；
 *       ③ 每个单元恰好出现在一个桶里，且与 `layers.yaml` 的单元清单双向一致（表里有、盘上没／盘上有、表里没都失败）；
 *       ④ `src/units.rs` 只声明三个桶。
 * 为什么：桶是「角色」不是环号——环是派生属性写在 `layers.yaml`，路径换环就会全量改名并与表形成两个权威。
 *   反向防的是「悄悄在顶层新加一个平铺单元」或「同一个单元copy到两个桶」，否则风格与唯一性只靠人盯。
 * 零豁免：不读基线文件。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { rel } = require("../../lib/fsutil.js");
const { BUCKETS } = require("./unitpath.js");
const { parseLayers } = require("./layers.js");

const DECL = /^\s*pub mod ([a-z_][a-z0-9_]*);/;

/** 目的：读一个模块根文件里声明的子模块名（只认 `pub mod x;`）。 */
function declared(file) {
  if (!fs.existsSync(file)) return [];
  const out = [];
  for (const line of fs.readFileSync(file, "utf8").split(/\r?\n/)) {
    const m = DECL.exec(line);
    if (m) out.push(m[1]);
  }
  return out;
}

/** 目的：扫一遍，返回问题清单与一行说明（几个桶、几个单元）。 */
function unitBuckets(root) {
  const problems = [];
  const base = path.join(root, "src", "units");
  if (!fs.existsSync(base)) return { problems: ["单元桶：缺少 src/units/"], notes: [] };
  const entries = fs.readdirSync(base, { withFileTypes: true });
  const homes = new Map(); // 单元名 → [桶…]
  const bucketDirs = [];

  for (const e of entries) {
    if (e.isDirectory()) {
      if (!BUCKETS.has(e.name)) {
        problems.push("单元桶：" + rel(root, path.join(base, e.name)) + " 是顶层单元目录——单元必须住进角色桶（" + [...BUCKETS].join(" / ") + "）");
        continue;
      }
      bucketDirs.push(e.name);
      continue;
    }
    const stem = e.name.replace(/\.rs$/, "");
    if (!e.name.endsWith(".rs") || !BUCKETS.has(stem)) {
      problems.push("单元桶：" + rel(root, path.join(base, e.name)) + " 不是桶的模块根——src/units/ 下只许三个桶目录与同名 .rs");
    }
  }

  for (const bucket of BUCKETS) {
    const dir = path.join(base, bucket);
    const modFile = path.join(base, bucket + ".rs");
    if (!fs.existsSync(dir)) {
      problems.push("单元桶：缺少桶目录 src/units/" + bucket + "/（三角色桶必须都在）");
      continue;
    }
    if (!fs.existsSync(modFile)) {
      problems.push("单元桶：缺少桶的模块根 src/units/" + bucket + ".rs");
      continue;
    }
    const kids = fs.readdirSync(dir, { withFileTypes: true });
    const dirs = kids.filter((k) => k.isDirectory()).map((k) => k.name).sort();
    const mods = kids.filter((k) => !k.isDirectory()).map((k) => k.name).sort();
    for (const k of kids) {
      if (k.isDirectory()) {
        if (!homes.has(k.name)) homes.set(k.name, []);
        homes.get(k.name).push(bucket);
        if (!fs.existsSync(path.join(dir, k.name + ".rs"))) {
          problems.push("单元桶：src/units/" + bucket + "/" + k.name + "/ 缺少同名模块根 " + k.name + ".rs（目录模块必须有同名文件）");
        }
      } else if (!k.name.endsWith(".rs")) {
        problems.push("单元桶：src/units/" + bucket + "/" + k.name + " 不是单元目录");
      }
    }
    const stray = mods.filter((m) => !dirs.includes(m.replace(/\.rs$/, "")));
    for (const s of stray) problems.push("单元桶：src/units/" + bucket + "/" + s + " 声明不了任何单元目录");
    const want = dirs.slice();
    const have = declared(modFile).sort();
    for (const d of want) if (!have.includes(d)) problems.push("单元桶：src/units/" + bucket + ".rs 缺少 'pub mod " + d + ";'");
    for (const h of have) if (!want.includes(h)) problems.push("单元桶：src/units/" + bucket + ".rs 里的 'pub mod " + h + ";' 在盘上没有对应目录");
  }

  for (const [unit, list] of homes) {
    if (list.length > 1) problems.push("单元桶：" + unit + " 同时住进了 " + list.join(" / ") + "——每个单元恰好住一个桶");
  }

  const parsed = parseLayers(root);
  problems.push(...parsed.problems);
  for (const unit of parsed.tiers.keys()) {
    if (unit === "kernel") continue;
    if (!homes.has(unit)) problems.push("单元桶：layers.yaml 里的 " + unit + " 没有住在任何桶里");
  }
  for (const unit of homes.keys()) {
    if (!parsed.tiers.has(unit)) problems.push("单元桶：src/units/ 下的 " + unit + " 没有在 layers.yaml 登记");
  }

  const rootMods = declared(path.join(root, "src", "units.rs")).sort();
  const wantRoot = [...BUCKETS].sort();
  for (const b of wantRoot) if (!rootMods.includes(b)) problems.push("单元桶：src/units.rs 缺少 'pub mod " + b + ";'");
  for (const m of rootMods) if (!wantRoot.includes(m)) problems.push("单元桶：src/units.rs 里的 'pub mod " + m + ";' 不是角色桶");

  return {
    problems,
    notes: ["单元桶：" + bucketDirs.length + " 个桶、盘上 " + homes.size + " 个单元、layers.yaml " + parsed.tiers.size + " 条"],
  };
}

module.exports = { unitBuckets };
