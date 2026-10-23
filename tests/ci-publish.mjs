// CI 每次结束时的自助发布（成败都发），两条通道都需要零人工介入：
// ① GitHub Actions 注释：失败时逐条 ::error::（带断言原文与在场证据），通过时一条 ::notice:: 概览；
//    匿名可读，不依赖任何额外权限。
// ② Contents API 写进滚动分支（统一 ci-report）：同一路径每次覆盖（只留最近一次），不需要 git/ssh/管道。
// 目的：现场（含"这次绿了"的 steps / envSkips / 围栏能力）要让"没有 GitHub 凭据的一方"（例如 AI 会话）能自己读到。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
// 报告键默认取 runner OS；文档审查 / 结构审查不是平台报告，由调用方用 REPORT_OS=docs|structure 指定。
const osName = (process.env.REPORT_OS || process.env.RUNNER_OS || "unknown").toLowerCase();
// 发布目标分支：统一 ci-report（由工作流在本次运行前重建），三路各写自己的 runs/<key>/。
const reportBranch = process.env.REPORT_BRANCH || "ci-report";
const repo = process.env.GITHUB_REPOSITORY || "theRMM714/Solomni";
const token = process.env.GITHUB_TOKEN || "";
const runNumber = process.env.GITHUB_RUN_NUMBER || "0";
// PR 触发时 GITHUB_SHA 是 merge commit；报告用 PR head（workflow 的 SOURCE_SHA），没有就退回 GITHUB_SHA。
const sha = process.env.SOURCE_SHA || process.env.GITHUB_SHA || "";

const esc = (s) => String(s).replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
const annotate = (title, body) => console.log("::error title=" + esc(title) + "::" + esc(String(body).slice(0, 6000)));

// —— 收集现场：报告 + 每份失败日志的要点（从第一处失败标记开始截，保证断言原文在里面）
const reportPath = path.join(ROOT, "target", "test-report.json");
let report = null;
if (fs.existsSync(reportPath)) report = JSON.parse(fs.readFileSync(reportPath, "utf8"));

// 从一份日志里挑出"能看懂为什么失败"的证据：失败行本身 + 上下文（行号最近的几处），
// 挑不到失败行才退回"从第一处失败标记起截尾"——免得只捞到一片 PASS。
function evidence(text) {
  const lines = text.split("\n");
  const hits = [];
  for (let i = 0; i < lines.length && hits.length < 12; i++) {
    if (/^\s*(FAIL|FAILED)\b|panicked at|test result: FAILED|error\[E\d+\]|E2E-FAILED|assertion/i.test(lines[i])) hits.push(i);
  }
  if (!hits.length) {
    const m = text.search(/test result: FAILED|panicked at|失败详情|E2E-FAILED/);
    return m >= 0 ? text.slice(Math.max(0, m - 400), m + 3600) : "";
  }
  const out = [];
  let last = -9;
  for (const i of hits) {
    if (i - last < 3) continue;
    out.push(lines.slice(Math.max(0, i - 1), Math.min(lines.length, i + 3)).join("\n"));
    last = i;
  }
  return out.join("\n…\n").slice(0, 4200);
}

const logDir = path.join(ROOT, "target", "test-logs");
const failing = [];
if (fs.existsSync(logDir)) {
  for (const f of fs.readdirSync(logDir)) {
    const t = fs.readFileSync(path.join(logDir, f), "utf8");
    const ev = evidence(t);
    if (ev) failing.push({ name: f, text: ev });
  }
}

// 成败按报告自身判定：**硬失败与质量失败都算失败**——T0 的 quality-fail 会让入口返回非零，
// 报告这边也必须同样算失败，否则 CI 摘要会把红的说成绿的。
// 连报告都没有（更早的步骤就挂了，或没跑到）同样按失败处理。
const qualityFailed = (report && report.quality && report.quality.failed) || 0;
const failed = !report || (report.failed || 0) > 0 || qualityFailed > 0;
const modeLabel = report && report.mode === "docs"
  ? "文档审查"
  : report && report.mode === "structure"
    ? "结构审查"
    : report && report.fenceLive
      ? "真机(--fence-live)"
      : "安全";
let summary = "平台=" + osName + " 模式=" + modeLabel + " 结果=" + (failed ? "失败" : "通过") + "\n";
if (report) {
  for (const s of report.steps || []) {
    if (s.status !== "pass" && s.status !== "skip-platform") summary += "  " + s.status + "  " + s.step + "  " + (s.detail || "") + "\n";
  }
  // 围栏的"环境性跳过"必须留在现场：绿了但探针跳过 = 这条围栏没验收，读的人要能一眼看到。
  if ((report.envSkips || []).length) summary += "环境跳过 " + report.envSkips.length + " 条：" + report.envSkips.join(" / ") + "\n";
  if ((report.gaps || []).length) summary += "缺口：" + report.gaps.join(", ") + "\n";
  if (report.doctor && report.doctor.fence) summary += "围栏能力：fs=" + report.doctor.fence.fs + " net=" + report.doctor.fence.net + " tree=" + report.doctor.fence.tree + "\n";
}
if (failed) {
  if (!failing.length) summary += "（没有匹配到失败日志，见产物）\n";
  annotate("测试失败（" + osName + "）", summary);
  for (const f of failing.slice(0, 3)) annotate("失败详情：" + f.name, f.text);
} else {
  console.log("::notice title=测试通过（" + osName + "）::" + esc(summary));
}

// —— 滚动分支（Contents API：同一路径覆盖，只留最近一次）
const apiHeaders = { Authorization: "Bearer " + token, "User-Agent": "solomni-ci", Accept: "application/vnd.github+json" };
const jsonHeaders = Object.assign({ "Content-Type": "application/json" }, apiHeaders);

// 首次运行时目标分支还不存在：Contents API 的 PUT 要求目标分支已存在，
// 所以先按默认分支的 HEAD 建出这个滚动分支，否则第一条失败报告就永远发不出去。
async function apiJson(url) {
  const r = await fetch(url, { headers: apiHeaders });
  if (!r.ok) throw new Error(url.replace("https://api.github.com", "") + " → HTTP " + r.status + " " + (await r.text()).slice(0, 160));
  return r.json();
}

async function ensureReportBranch() {
  const refUrl = "https://api.github.com/repos/" + repo + "/git/ref/heads/" + reportBranch;
  const ref = await fetch(refUrl, { headers: apiHeaders });
  if (ref.status === 200) return;
  // 404 才是"还没有这个分支"；其它状态（401/403）说明凭据或权限不对，如实报出来而不是继续往下撞。
  if (ref.status !== 404) throw new Error("查 " + reportBranch + " 分支 → HTTP " + ref.status + " " + (await ref.text()).slice(0, 160));
  const info = await apiJson("https://api.github.com/repos/" + repo);
  const baseRef = await apiJson("https://api.github.com/repos/" + repo + "/git/ref/heads/" + info.default_branch);
  const made = await fetch("https://api.github.com/repos/" + repo + "/git/refs", {
    method: "POST",
    headers: jsonHeaders,
    body: JSON.stringify({ ref: "refs/heads/" + reportBranch, sha: baseRef.object.sha }),
  });
  // 422 = 已被并发建好，当作成功。
  if (!made.ok && made.status !== 422) throw new Error("建 " + reportBranch + " 分支 → HTTP " + made.status + " " + (await made.text()).slice(0, 160));
}

async function putFile(pathInRepo, text) {
  const url = "https://api.github.com/repos/" + repo + "/contents/" + pathInRepo;
  let shaExisting = null;
  const g = await fetch(url + "?ref=" + reportBranch, { headers: apiHeaders });
  if (g.status === 200) shaExisting = (await g.json()).sha;
  const body = { message: (failed ? "失败报告" : "通过报告") + " run " + runNumber + " (" + osName + ")", content: Buffer.from(text, "utf8").toString("base64"), branch: reportBranch };
  if (shaExisting) body.sha = shaExisting;
  const r = await fetch(url, { method: "PUT", headers: jsonHeaders, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(pathInRepo + " → HTTP " + r.status + " " + (await r.text()).slice(0, 200));
}

if (token) {
  try {
    await ensureReportBranch();
    await putFile(
      "runs/" + osName + "/meta.json",
      JSON.stringify(
        { run: runNumber, sha, os: osName, mode: report ? report.mode || "full" : "none", at: new Date().toISOString(), failed: failed, qualityFailed: qualityFailed },
        null,
        2,
      ),
    );
    if (report) await putFile("runs/" + osName + "/test-report.json", JSON.stringify(report, null, 2));
    if (fs.existsSync(logDir)) {
      for (const f of fs.readdirSync(logDir)) await putFile("runs/" + osName + "/logs/" + f, fs.readFileSync(path.join(logDir, f), "utf8"));
    }
    console.log("[ci-publish] 已写入 " + reportBranch + " 分支的 runs/" + osName + "/");
  } catch (e) {
    console.log("::warning title=" + reportBranch + " 分支未发布::" + esc(String(e.message || e).slice(0, 300)));
  }
} else {
  console.log("::warning title=没有 GITHUB_TOKEN::跳过 " + reportBranch + " 分支发布");
}
