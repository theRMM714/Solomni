// 合并突变测试的分片结果（每个分片一个 job，见 .github/workflows/mutants.yml）：
// 把 target/shards/**/mutation-report*.json 折成一份 target/mutation-report.json，并把各片的
// mutants.out/*.txt 与 outcomes.json 拼到仓库根，供 ci-mutation-publish.mjs 照原样发布。
// 本入口只做调查结果的合并：分片缺失或状态不一都如实写进报告，不改变工作流的红绿（红由各分片自己给出）。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SHARDS_DIR = path.join(ROOT, "target", "shards");
const REPORT = path.join(ROOT, "target", "mutation-report.json");
const OUT_DIR = path.join(ROOT, "mutants.out");

function walk(dir) {
  const out = [];
  if (!fs.existsSync(dir)) return out;
  for (const e of fs.readdirSync(dir, { withFileTypes: true })) {
    const p = path.join(dir, e.name);
    if (e.isDirectory()) out.push(...walk(p));
    else out.push(p);
  }
  return out;
}

const reports = walk(SHARDS_DIR)
  .filter((f) => /mutation-report.*\.json$/.test(path.basename(f)))
  .map((f) => {
    try {
      return { file: f, body: JSON.parse(fs.readFileSync(f, "utf8")) };
    } catch (e) {
      console.log("::warning title=分片报告读不懂::" + path.relative(ROOT, f) + "：" + String(e.message || e));
      return null;
    }
  })
  .filter(Boolean);

if (!reports.length) {
  console.log("[突变合并] 没有分片报告（target/shards/**）；已退回上次的 target/mutation-report.json");
  process.exit(0);
}

// 状态优先级：基线挂 > 报错 > 有漏网 > 有超时 > 全捕获——合并后按最坏的那片说话。
const ORDER = ["all-caught", "timeout", "missed", "error", "baseline-failed"];
const EXIT = { "all-caught": 0, timeout: 3, missed: 2, error: 1, "baseline-failed": 4 };
const worst = reports
  .map((r) => String(r.body.status || "error"))
  .sort((a, b) => ORDER.indexOf(b) - ORDER.indexOf(a))[0] || "error";
const status = ORDER.includes(worst) ? worst : "error";

const shards = reports.map((r) => ({
  shard: r.body.shard || null,
  status: r.body.status || "error",
  summary: r.body.summary || "",
  ms: r.body.ms || 0,
}));
const first = reports[0].body;
const report = {
  scope: first.scope,
  capability: first.capability || null,
  shard: null,
  shards: reports.length,
  shardsExpected: Number(first.shards) || reports.length,
  shardReports: shards,
  files: [...new Set(reports.flatMap((r) => r.body.files || []))],
  timeoutSecs: first.timeoutSecs,
  exitCode: EXIT[status],
  status,
  summary: reports.map((r) => (r.body.shard || "1/1") + " " + (r.body.summary || r.body.status)).join("；"),
  ms: reports.reduce((n, r) => Math.max(n, r.body.ms || 0), 0),
  log: reports.map((r) => r.body.log).filter(Boolean),
  mergedAt: new Date().toISOString(),
};
fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));

fs.mkdirSync(OUT_DIR, { recursive: true });
for (const name of ["missed.txt", "caught.txt", "timeout.txt", "unviable.txt"]) {
  const parts = walk(SHARDS_DIR)
    .filter((f) => path.basename(f) === name)
    .map((f) => {
      const t = fs.readFileSync(f, "utf8").trim();
      return t ? "# " + path.relative(SHARDS_DIR, path.dirname(f)) + "\n" + t + "\n" : "";
    })
    .filter(Boolean);
  if (parts.length) fs.writeFileSync(path.join(OUT_DIR, name), parts.join(""));
}
const outcomes = walk(SHARDS_DIR)
  .filter((f) => path.basename(f) === "outcomes.json")
  .map((f) => fs.readFileSync(f, "utf8").trim())
  .filter(Boolean);
if (outcomes.length) fs.writeFileSync(path.join(OUT_DIR, "outcomes.json"), outcomes.join("\n") + "\n");

console.log("[突变合并] " + shards.length + " 个分片 → " + status);
for (const s of shards) console.log("  " + (s.shard || "1/1") + " " + s.status + "：" + (s.summary || "无摘要"));
console.log("[突变合并] 报告 " + path.relative(ROOT, REPORT));
