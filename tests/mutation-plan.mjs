// 突变分片计划：先数「这次要跑多少个变异体」，再把 `--shard k/n` 切成 n 片，输出 GitHub Actions 的 matrix。
// 为什么不在 workflow 里写死 4 片：片数应当由变异体总数现算（单变异体约 29s，350 个≈3h），
// 范围变大时自动加片，范围变小时不浪费 job。
// 只读：不动源码、不写 TEST-REPORT-*。--list 不可用（本机没装 cargo-mutants）时退回固定 --fallback 片并显式告警。
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const envLayer = require("../env.js");
const mutation = require("./lib/mutation.js");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.join(ROOT, "target", "mutation-plan.json");

const argOf = (name) => {
  const hit = process.argv.slice(2).find((a) => a.startsWith("--" + name + "="));
  return hit ? hit.slice(name.length + 3) : "";
};
const scope = (argOf("scope") || process.env.MUTATION_SCOPE || "weekly").trim();
const unit = scope === "unit" ? (argOf("unit") || process.env.MUTATION_UNIT || "").trim() : "";
const perShard = Math.max(1, Number(argOf("max-per-shard") || process.env.MUTATION_MAX_PER_SHARD || 350));
const fallback = Number(argOf("fallback") || 4);
const asJson = process.argv.slice(2).includes("--json");

const warn = (m) => process.stderr.write("[突变计划] " + m + "\n");

const plan = mutation.collect(ROOT, { scope, unit });
if (plan.problems.length) {
  warn("突变声明有问题：" + plan.problems.join("；"));
  process.exit(2);
}

/** 数 --list 输出的变异体：优先整段 JSON，其次逐行 JSONL，最后退回纯文本「file:line:col: …」。 */
function countMutants(text) {
  const t = String(text).trim();
  if (!t) return 0;
  try {
    const parsed = JSON.parse(t);
    if (Array.isArray(parsed)) return parsed.length;
    if (parsed && Array.isArray(parsed.mutants)) return parsed.mutants.length;
  } catch {
    // 不是整段 JSON，继续按行解析
  }
  const lines = t.split(/\r?\n/).filter((l) => l.trim());
  if (lines.every((l) => l.trim().startsWith("{"))) {
    let n = 0;
    for (const l of lines) {
      try { JSON.parse(l); n++; } catch { /* 表头或告警行，跳过 */ }
    }
    return n;
  }
  return lines.filter((l) => /\.rs:\d+:\d+:/.test(l)).length;
}

function listMutants() {
  const resolvedEnv = envLayer.resolve();
  const childEnv = Object.assign({}, resolvedEnv ? resolvedEnv.env : process.env);
  envLayer.pinTemp(childEnv);
  const run = (withJson) => {
    const args = ["mutants", "--list"];
    if (withJson) args.push("--json");
    for (const f of plan.files) args.push("-f", f);
    return spawnSync("cargo", args, { cwd: ROOT, env: childEnv, encoding: "utf8", timeout: 15 * 60 * 1000 });
  };
  let r = run(true);
  // 老版本 cargo-mutants 没有 --list --json：退回纯文本，靠「file:line:col:」行数。
  if (r.status !== 0 && /unexpected argument|unrecognized|unknown argument/i.test(String(r.stderr || ""))) {
    r = run(false);
  }
  if (r.error) return { total: 0, why: String(r.error.message || r.error) };
  if (r.status !== 0) return { total: 0, why: "cargo mutants --list 退出码 " + r.status + "：" + String(r.stderr || "").split(/\r?\n/).slice(-3).join(" ") };
  const total = countMutants(r.stdout);
  return total ? { total, why: "" } : { total: 0, why: "--list 输出里认不出变异体（格式变了？）" };
}

let total = 0;
let why = "";
try {
  const listed = listMutants();
  total = listed.total;
  why = listed.why;
} catch (e) {
  why = String(e && e.message ? e.message : e);
}

let shards;
if (total > 0) {
  const n = Math.ceil(total / perShard);
  shards = Array.from({ length: n }, (_, i) => i + 1 + "/" + n);
} else if (fallback > 0) {
  warn("数不出变异体，退回固定 " + fallback + " 片" + (why ? "（原因：" + why + "）" : ""));
  shards = Array.from({ length: fallback }, (_, i) => i + 1 + "/" + fallback);
} else {
  warn("数不出变异体且禁用回退：" + why);
  process.exit(2);
}

const out = {
  scope,
  unit: unit || null,
  units: plan.units.map((u) => u.unit),
  files: plan.files,
  mutants: total || null,
  maxPerShard: perShard,
  shards,
  include: shards.map((s) => ({ shard: s })),
};
fs.mkdirSync(path.dirname(OUT), { recursive: true });
fs.writeFileSync(OUT, JSON.stringify(out, null, 2));
if (asJson) {
  console.log(JSON.stringify(out, null, 2));
} else {
  console.log("[突变计划] 范围 " + scope + (unit ? ":" + unit : "") + "，文件 " + plan.files.length + " 个，变异体 " + (total || "未知") + "，分 " + shards.length + " 片");
  console.log("[突变计划] 分片：" + shards.join(" "));
  console.log("PLAN-MUTANTS=" + (total || 0));
  console.log("PLAN-SHARDS=" + shards.length);
  console.log("[突变计划] 计划写 " + path.relative(ROOT, OUT));
}
