// CI 失败时的自助发布：① 打成 GitHub Actions 注释（匿名可读，不需要任何额外权限）
// ② 再尝试把报告与日志推到滚动分支 ci-report（需要 contents: write；失败只告警，不掩盖真正的失败）。
// 这样"没有 GitHub 凭据的一方"（例如 AI 会话）也能读到失败详情。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = path.resolve(path.dirname(process.argv[1]), "..");
const osName = process.env.RUNNER_OS || "unknown";
const repo = process.env.GITHUB_REPOSITORY || "theRMM714/Solomni";
const token = process.env.GITHUB_TOKEN || "";
const runNumber = process.env.GITHUB_RUN_NUMBER || "0";

const esc = (s) => String(s).replace(/%/g, "%25").replace(/\r/g, "%0D").replace(/\n/g, "%0A");
const annotate = (title, body) => console.log("::error title=" + esc(title) + "::" + esc(String(body).slice(0, 12000)));

// ① 报告要点 + 失败证据（挑一份提到失败的日志，带出其尾部）
let summary = "";
const reportPath = path.join(ROOT, "target", "test-report.json");
if (fs.existsSync(reportPath)) {
  const r = JSON.parse(fs.readFileSync(reportPath, "utf8"));
  summary += "安全模式=" + (r.fenceLive ? "真机(--fence-live)" : "安全") + " 平台=" + r.platform + " " + r.arch + "\n";
  for (const s of r.steps || []) summary += "  " + s.status + "  " + s.step + "  " + (s.detail || "") + "\n";
  if ((r.envSkips || []).length) summary += "环境跳过 " + r.envSkips.length + " 条\n";
  if ((r.gaps || []).length) summary += "缺口：" + r.gaps.join(", ") + "\n";
  if (r.doctor && r.doctor.fence) summary += "围栏能力：fs=" + r.doctor.fence.fs + " net=" + r.doctor.fence.net + " tree=" + r.doctor.fence.tree + "\n";
}
let evidence = "";
const logDir = path.join(ROOT, "target", "test-logs");
if (fs.existsSync(logDir)) {
  for (const f of fs.readdirSync(logDir)) {
    const t = fs.readFileSync(path.join(logDir, f), "utf8");
    if (/test result: FAILED|panicked at|error\[E\d+\]|失败详情/.test(t)) {
      evidence = "【日志 " + f + " 尾部】\n" + t.slice(-5000);
      break;
    }
  }
}
if (summary || evidence) {
  annotate("测试失败（" + osName + "）", (summary + "\n" + evidence).slice(0, 12000));
  console.log("[ci-publish] 已把失败要点打成注释");
}

// ② 滚动分支（可选通道：需要仓库允许 workflow 写内容）
try {
  const stage = fs.mkdtempSync(path.join(os.tmpdir(), "ci-report-"));
  const dest = path.join(stage, "runs", runNumber + "-" + osName);
  fs.mkdirSync(dest, { recursive: true });
  if (fs.existsSync(reportPath)) fs.copyFileSync(reportPath, path.join(dest, "test-report.json"));
  if (fs.existsSync(logDir)) fs.cpSync(logDir, path.join(dest, "test-logs"), { recursive: true });
  fs.writeFileSync(
    path.join(stage, "README.md"),
    "# CI 失败报告（自动发布，滚动覆盖）\n\n每次任务失败时由 .github/workflows/test.yml 发布：该平台的 target/test-report.json 与 target/test-logs/。\n"
  );
  const git = (...a) => execFileSync("git", a, { cwd: stage, stdio: "inherit" });
  git("init", "-q");
  git("checkout", "-q", "-b", "ci-report");
  git("config", "user.email", "ci@solomni");
  git("config", "user.name", "ci");
  if (token) {
    const basic = Buffer.from("x-access-token:" + token).toString("base64");
    git("config", "--local", "http.https://github.com/.extraheader", "AUTHORIZATION: basic " + basic);
  }
  git("add", "-A");
  git("commit", "-q", "-m", "失败报告 run " + runNumber + " (" + osName + ")");
  git("push", "--force", "https://github.com/" + repo + ".git", "ci-report");
  console.log("[ci-publish] 已发布到 ci-report 分支");
} catch (e) {
  console.log("::warning title=ci-report 分支未发布::" + esc(String(e.message || e).slice(0, 300)));
}
