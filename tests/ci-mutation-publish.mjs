// 把突变测试的**小结果**发到 ci-mutation 滚动分支（同一键每次覆盖，只留最近一次）：
// 让没有 GitHub 凭据的一方也能 git show origin/ci-mutation:<key>/missed.txt 读到调查结果。
// 大的 mutants.out/ 与日志仍走 Actions 产物，不进 git。发布失败只是 ::warning::，不影响突变测试本身的成败。
// 分支模型与 ci-publish.mjs 同源，但 ci-mutation 由本工作流独占——与 test.yml 的 ci-report 各推各的。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const repo = process.env.GITHUB_REPOSITORY || "";
const token = process.env.GITHUB_TOKEN || "";
const runNumber = process.env.GITHUB_RUN_NUMBER || "0";
const sha = process.env.GITHUB_SHA || "";
const BRANCH = "ci-mutation";

const reportPath = path.join(ROOT, "target", "mutation-report.json");
if (!fs.existsSync(reportPath)) {
  console.log("::warning title=ci-mutation 未发布::找不到 target/mutation-report.json（突变步骤可能没跑到）");
  process.exit(0);
}
const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
const key = report.capability ? report.scope + "-" + report.capability : report.scope;

const headers = { Authorization: "Bearer " + token, "User-Agent": "solomni-ci", Accept: "application/vnd.github+json" };
const jsonHeaders = Object.assign({ "Content-Type": "application/json" }, headers);

async function apiJson(url) {
  const r = await fetch(url, { headers });
  if (!r.ok) throw new Error(url.replace("https://api.github.com", "") + " → HTTP " + r.status + " " + (await r.text()).slice(0, 160));
  return r.json();
}

// 首次运行时分支还不存在：Contents API 的 PUT 要求目标分支已存在，先按默认分支 HEAD 建出来。
async function ensureBranch() {
  const ref = await fetch("https://api.github.com/repos/" + repo + "/git/ref/heads/" + BRANCH, { headers });
  if (ref.status === 200) return;
  if (ref.status !== 404) throw new Error("查 " + BRANCH + " 分支 → HTTP " + ref.status);
  const info = await apiJson("https://api.github.com/repos/" + repo);
  const base = await apiJson("https://api.github.com/repos/" + repo + "/git/ref/heads/" + info.default_branch);
  const made = await fetch("https://api.github.com/repos/" + repo + "/git/refs", {
    method: "POST",
    headers: jsonHeaders,
    body: JSON.stringify({ ref: "refs/heads/" + BRANCH, sha: base.object.sha }),
  });
  if (!made.ok && made.status !== 422) throw new Error("建 " + BRANCH + " 分支 → HTTP " + made.status);
}

async function putFile(p, text) {
  const url = "https://api.github.com/repos/" + repo + "/contents/" + p;
  let existing = null;
  const g = await fetch(url + "?ref=" + BRANCH, { headers });
  if (g.status === 200) existing = (await g.json()).sha;
  const body = {
    message: "突变报告 " + key + " run " + runNumber,
    content: Buffer.from(text, "utf8").toString("base64"),
    branch: BRANCH,
  };
  if (existing) body.sha = existing;
  const r = await fetch(url, { method: "PUT", headers: jsonHeaders, body: JSON.stringify(body) });
  if (!r.ok) throw new Error(p + " → HTTP " + r.status + " " + (await r.text()).slice(0, 200));
}

const meta = {
  run: runNumber,
  sha,
  at: new Date().toISOString(),
  scope: report.scope,
  capability: report.capability,
  status: report.status,
  exitCode: report.exitCode,
  summary: report.summary,
  ms: report.ms,
};

if (!token) {
  console.log("::warning title=没有 GITHUB_TOKEN::跳过 ci-mutation 分支发布");
  process.exit(0);
}
try {
  await ensureBranch();
  const files = { "report.json": JSON.stringify(report, null, 2), "meta.json": JSON.stringify(meta, null, 2) };
  for (const name of ["missed.txt", "caught.txt", "timeout.txt", "unviable.txt", "outcomes.json"]) {
    const p = path.join(ROOT, "mutants.out", name);
    if (fs.existsSync(p)) files[name] = fs.readFileSync(p, "utf8");
  }
  for (const [name, text] of Object.entries(files)) await putFile(key + "/" + name, text);
  console.log("[ci-mutation-publish] 已写入 " + BRANCH + " 分支的 " + key + "/（" + Object.keys(files).join(", ") + "）");
} catch (e) {
  console.log("::warning title=ci-mutation 分支未发布::" + String(e.message || e).slice(0, 300));
}
