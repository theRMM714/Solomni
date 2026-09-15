// CI 失败时的自助发布，两条通道都需要零人工介入：
// ① GitHub Actions 注释（::error::）：匿名可读，不依赖任何额外权限；每个失败日志一条，都带断言原文与在场证据。
// ② Contents API 写进 ci-report 滚动分支：同一路径每次覆盖（只留最近一次），不需要 git/ssh/管道。
// 目的：失败详情要让"没有 GitHub 凭据的一方"（例如 AI 会话）能自己读到。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const osName = (process.env.RUNNER_OS || "unknown").toLowerCase();
const repo = process.env.GITHUB_REPOSITORY || "theRMM714/Solomni";
const token = process.env.GITHUB_TOKEN || "";
const runNumber = process.env.GITHUB_RUN_NUMBER || "0";
const sha = process.env.GITHUB_SHA || "";

const esc = (s) => String(s).replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
const annotate = (title, body) => console.log("::error title=" + esc(title) + "::" + esc(String(body).slice(0, 6000)));

// —— 收集现场：报告 + 每份失败日志的要点（从第一处失败标记开始截，保证断言原文在里面）
const reportPath = path.join(ROOT, "target", "test-report.json");
let report = null;
if (fs.existsSync(reportPath)) report = JSON.parse(fs.readFileSync(reportPath, "utf8"));

const logDir = path.join(ROOT, "target", "test-logs");
const failing = [];
if (fs.existsSync(logDir)) {
  for (const f of fs.readdirSync(logDir)) {
    const t = fs.readFileSync(path.join(logDir, f), "utf8");
    const m = t.search(/test result: FAILED|panicked at|error\[E\d+\]|失败详情|E2E-FAILED/);
    if (m >= 0) failing.push({ name: f, text: t.slice(Math.max(0, m - 400), m + 3600) });
  }
}

let summary = "平台=" + osName + " 模式=" + (report && report.fenceLive ? "真机(--fence-live)" : "安全") + "\n";
if (report) {
  for (const s of report.steps || []) {
    if (s.status !== "pass" && s.status !== "skip-platform") summary += "  " + s.status + "  " + s.step + "  " + (s.detail || "") + "\n";
  }
  if ((report.gaps || []).length) summary += "缺口：" + report.gaps.join(", ") + "\n";
  if (report.doctor && report.doctor.fence) summary += "围栏能力：fs=" + report.doctor.fence.fs + " net=" + report.doctor.fence.net + " tree=" + report.doctor.fence.tree + "\n";
}
if (!failing.length) summary += "（没有匹配到失败日志，见产物）\n";
annotate("测试失败（" + osName + "）", summary);
for (const f of failing.slice(0, 3)) annotate("失败详情：" + f.name, f.text);

// —— ci-report 分支（Contents API：同一路径覆盖，只留最近一次）
async function putFile(pathInRepo, text) {
  const url = "https://api.github.com/repos/" + repo + "/contents/" + pathInRepo;
  const headers = { Authorization: "Bearer " + token, "User-Agent": "solomni-ci", Accept: "application/vnd.github+json" };
  let shaExisting = null;
  const g = await fetch(url + "?ref=ci-report", { headers });
  if (g.status === 200) shaExisting = (await g.json()).sha;
  const body = { message: "失败报告 run " + runNumber + " (" + osName + ")", content: Buffer.from(text, "utf8").toString("base64"), branch: "ci-report" };
  if (shaExisting) body.sha = shaExisting;
  const r = await fetch(url, { method: "PUT", headers: Object.assign({ "Content-Type": "application/json" }, headers), body: JSON.stringify(body) });
  if (!r.ok) throw new Error(pathInRepo + " → HTTP " + r.status + " " + (await r.text()).slice(0, 200));
}

if (token) {
  try {
    await putFile("runs/" + osName + "/meta.json", JSON.stringify({ run: runNumber, sha, os: osName, at: new Date().toISOString() }, null, 2));
    if (report) await putFile("runs/" + osName + "/test-report.json", JSON.stringify(report, null, 2));
    if (fs.existsSync(logDir)) {
      for (const f of fs.readdirSync(logDir)) await putFile("runs/" + osName + "/logs/" + f, fs.readFileSync(path.join(logDir, f), "utf8"));
    }
    console.log("[ci-publish] 已写入 ci-report 分支的 runs/" + osName + "/");
  } catch (e) {
    console.log("::warning title=ci-report 分支未发布::" + esc(String(e.message || e).slice(0, 300)));
  }
} else {
  console.log("::warning title=没有 GITHUB_TOKEN::跳过 ci-report 分支发布");
}
