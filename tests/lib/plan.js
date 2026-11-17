/**
 * 测试计划的收集与过滤（编排器只做这一件事：**声明 → 选择 → 派发**）。
 *
 * 收集：`tests/specs/*.yaml` 是唯一声明处，一条用例一行（id / tier / note）。
 * 过滤：`--unit` / `--tier` / `--journey` 都是"选子集"，选完由调用方派发。
 * 派发不看实现细节：cargo 用例按**测试路径前缀**成组跑，旅程按 id 走 e2e。
 */
"use strict";

const { loadSpecs, splitCaseId, cargoTargetExists, platformTargetExists, TIERS } = require("./spec");
const journeys = require("../journeys/index.js");

/** 读全部 spec 声明（附带的解析问题原样带回，由调用方决定怎么报）。 */
function collect(root) {
  const { specs, problems } = loadSpecs(root);
  const entries = [];
  for (const s of specs) {
    for (const c of s.cases) entries.push({ unit: s.unit, spec: s.file, id: c.id, tier: c.tier, note: c.note || "" });
  }
  return { entries, specs, problems };
}

/** 过滤器：三段都是逗号列表，空 = 不筛选。 */
function parseFilters(opts) {
  const list = (v) => String(v || "").split(",").map((x) => x.trim()).filter(Boolean);
  return { units: list(opts.unit), tiers: list(opts.tier), journeys: list(opts.journey) };
}

function isEmpty(f) {
  return !f.units.length && !f.tiers.length && !f.journeys.length;
}

/** 选子集：三个维度是"与"（都要满足），维度内部是"或"。 */
function select(entries, f) {
  return entries.filter((e) => {
    if (f.units.length && !f.units.includes(e.unit)) return false;
    if (f.tiers.length && !f.tiers.includes(e.tier)) return false;
    if (f.journeys.length) {
      const { kind, target } = splitCaseId(e.id);
      if (kind !== "e2e" || !f.journeys.includes(target)) return false;
    }
    return true;
  });
}

/** 声明能不能落地：id 可解析、目标存在、旅程在清单里。返回问题列表。 */
function verify(root, entries) {
  const problems = [];
  for (const e of entries) {
    const { kind, target } = splitCaseId(e.id);
    if (!kind) {
      problems.push("用例 " + e.id + "：id 只认 <cargo|platform|e2e>/<目标>");
      continue;
    }
    if (!TIERS[kind].includes(e.tier)) {
      problems.push("用例 " + e.id + "：" + kind + " 的层级只能是 " + TIERS[kind].join(" / ") + "，现在是 " + e.tier);
    }
    if (kind === "e2e") {
      if (!journeys.BY_ID.has(target)) problems.push("用例 " + e.id + "：旅程清单里没有 " + target);
      continue;
    }
    if (kind === "cargo" && !cargoTargetExists(root, target)) {
      problems.push("用例 " + e.id + "：找不到测试文件 " + target.replace(/^tests::/, "src/tests/").replace(/::/g, "/"));
    }
    if (kind === "platform" && !platformTargetExists(root, target)) {
      problems.push("用例 " + e.id + "：找不到 tests/" + target + "/");
    }
  }
  return problems;
}

/** cargo 用例按测试路径前缀成组（一次 cargo 调用跑一棵子树）。 */
function cargoFilters(entries) {
  const out = [];
  for (const e of entries) {
    const { kind, target } = splitCaseId(e.id);
    if (kind === "cargo" && !out.includes(target)) out.push(target);
  }
  return out;
}

/** 平台目标（cargo --test <name>）。 */
function platformTargets(entries) {
  const out = [];
  for (const e of entries) {
    const { kind, target } = splitCaseId(e.id);
    if (kind === "platform" && !out.includes(target)) out.push(target);
  }
  return out;
}

/** 旅程 id（e2e 驱动按 id 分段跑）。 */
function journeyIds(entries) {
  const out = [];
  for (const e of entries) {
    const { kind, target } = splitCaseId(e.id);
    if (kind === "e2e" && !out.includes(target)) out.push(target);
  }
  return out;
}

/** 一行摘要（--plan / --list 打印用）。 */
function summary(entries) {
  const byTier = new Map();
  const byUnit = new Map();
  for (const e of entries) {
    byTier.set(e.tier, (byTier.get(e.tier) || 0) + 1);
    byUnit.set(e.unit, (byUnit.get(e.unit) || 0) + 1);
  }
  return {
    total: entries.length,
    tiers: [...byTier.entries()].sort().map(([t, n]) => ({ tier: t, count: n })),
    units: [...byUnit.entries()].sort().map(([u, n]) => ({ unit: u, count: n })),
  };
}

module.exports = { collect, parseFilters, isEmpty, select, verify, cargoFilters, platformTargets, journeyIds, summary };
