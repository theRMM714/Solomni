// 突变测试入口（手动抽查 + 每周一定时；契约见 docs/testing/execution-ci.md 的「突变测试工作流」）。
// 只圈 tests/mutation-scope.json 里的高价值纯逻辑文件；测试命令固定「只跑 bin + 串行」——
// 串行便于逐条变异单独复现，与门禁的默认并行无关。
// 本入口只作调查：不写 TEST-REPORT-*，不参与验收。
import fs from "node:fs";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const require = createRequire(import.meta.url);
const envLayer = require("../env.js");

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SCOPE_FILE = path.join(ROOT, "tests", "mutation-scope.json");
const LOG_DIR = path.join(ROOT, "target", "logs");
const REPORT = path.join(ROOT, "target", "mutation-report.json");
fs.mkdirSync(LOG_DIR, { recursive: true });

const scope = process.env.MUTATION_SCOPE || "core";
// 只有 scope=capability 才记能力名：其余范围不带，键才是范围本身（而不是 core-repair）。
const capabilityName = scope === "capability" ? process.env.MUTATION_CAPABILITY || "" : "";
const timeout = process.env.MUTATION_TIMEOUT_SECS || "60";

const die = (m) => {
  console.error("[突变] " + m);
  process.exit(2);
};
// 分片：k/n，把同一范围的变异体均分到 n 个 job 上并行（单个范围已超过 GitHub 的 6 小时 job 上限）。
const shard = process.env.MUTATION_SHARD || "1/1";
if (!/^\d+\/\d+$/.test(shard)) die("MUTATION_SHARD 只认 k/n 形式（如 2/4），收到：" + shard);
const [shardIndex, shardCount] = shard.split("/").map(Number);
if (!(shardCount >= 1 && shardIndex >= 1 && shardIndex <= shardCount)) die("MUTATION_SHARD 越界：" + shard);
const sharded = shardCount > 1;
if (!fs.existsSync(SCOPE_FILE)) die("缺范围文件：" + path.relative(ROOT, SCOPE_FILE));
const def = JSON.parse(fs.readFileSync(SCOPE_FILE, "utf8"));
let files;
if (scope === "core") {
  files = def.core || [];
} else if (scope === "weekly") {
  files = def.weekly || [];
} else if (scope === "capability") {
  if (!capabilityName) die("scope=capability 需要 MUTATION_CAPABILITY（可选：" + Object.keys(def.capabilities || {}).join(" / ") + "）");
  files = (def.capabilities || {})[capabilityName];
  if (!files) die("能力不在 tests/mutation-scope.json 的 capabilities 里：" + capabilityName);
} else {
  die("MUTATION_SCOPE 只认 core / weekly / capability（all 刻意不做，理由见 docs/testing/execution-ci.md 的「突变测试工作流」）");
}
if (!files.length) die("范围为空：" + scope + (capabilityName ? ":" + capabilityName : ""));

const args = ["mutants", "--in-place", "--timeout", timeout, "--build-timeout", String(Number(timeout) * 2)];
for (const f of files) args.push("-f", f);
if (sharded) args.push("--shard", shard);
// 只跑 bin（不把 T3/T4 真进程用例拖进每个变异体）+ 串行（便于逐条变异单独复现）。
args.push("--", "--bin", "solomni", "--", "--test-threads=1");

const base = "mutation-" + scope + (capabilityName ? "-" + capabilityName : "");
const logFile = path.join(LOG_DIR, base + (sharded ? "-shard-" + shardIndex + "-of-" + shardCount : "") + ".log");
const fd = fs.openSync(logFile, "w");
process.stdout.write("[突变] 范围 " + scope + (capabilityName ? ":" + capabilityName : "") + "，分片 " + shard + "，文件 " + files.length + " 个\n");
process.stdout.write("[突变] 命令：cargo " + args.join(" ") + "\n");
const t0 = Date.now();
// 与门禁同一份环境解析：借用系统工具链可以，但 CARGO_HOME/临时目录仍钉在项目内（不许写它的 home）。
const resolvedEnv = envLayer.resolve();
const childEnv = Object.assign({}, resolvedEnv ? resolvedEnv.env : process.env);
envLayer.pinTemp(childEnv);
const r = spawnSync("cargo", args, { cwd: ROOT, env: childEnv, stdio: ["ignore", fd, fd] });
fs.closeSync(fd);
const out = fs.readFileSync(logFile, "utf8");
const exitCode = r.error ? -1 : r.status;
const summary = (out.split(/\r?\n/).find((l) => /mutants? tested in/.test(l)) || "").trim();
const status =
  exitCode === 0 ? "all-caught" :
  exitCode === 2 ? "missed" :
  exitCode === 3 ? "timeout" :
  exitCode === 4 ? "baseline-failed" :
  "error";
const report = {
  scope,
  capability: capabilityName || null,
  shard,
  shards: shardCount,
  files,
  timeoutSecs: Number(timeout),
  exitCode,
  status,
  summary,
  ms: Date.now() - t0,
  log: path.relative(ROOT, logFile),
};
fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));
console.log("[突变] " + status + "（exit " + exitCode + "）：" + (summary || "见日志 " + report.log));
console.log("[突变] 报告 " + path.relative(ROOT, REPORT) + "；结果目录 mutants.out/；本次只作调查，不参与 TEST-REPORT-ACCEPTED");
if (exitCode === 0) {
  console.log("MUTATION-OK");
  process.exit(0);
}
console.log("MUTATION-FOUND");
process.exit(1);
