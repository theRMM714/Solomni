// 把失败报告与日志推到滚动分支 ci-report：这样"没有 GitHub 凭据的一方"（例如 AI 会话）也能读到失败详情。
// 只在 CI 失败时由工作流调用；每次覆盖同名目录，分支始终只有最近一次的产物。
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { execFileSync } from "node:child_process";

const ROOT = path.resolve(path.dirname(new URL(import.meta.url).pathname.replace(/^\/(?=[A-Za-z]:)/, "")), "..");
const runNumber = process.env.GITHUB_RUN_NUMBER || "0";
const osName = process.env.RUNNER_OS || "unknown";
const repo = process.env.GITHUB_REPOSITORY || "theRMM714/Solomni";
const token = process.env.GITHUB_TOKEN || "";

const stage = fs.mkdtempSync(path.join(os.tmpdir(), "ci-report-"));
const dest = path.join(stage, "runs", `${runNumber}-${osName}`);
fs.mkdirSync(dest, { recursive: true });

const report = path.join(ROOT, "target", "test-report.json");
if (fs.existsSync(report)) fs.copyFileSync(report, path.join(dest, "test-report.json"));
const logs = path.join(ROOT, "target", "test-logs");
if (fs.existsSync(logs)) fs.cpSync(logs, path.join(dest, "test-logs"), { recursive: true });
fs.writeFileSync(
  path.join(stage, "README.md"),
  [
    "# CI 失败报告（自动发布，滚动覆盖）",
    "",
    "每次任务失败时由 .github/workflows/test.yml 推到这条分支，内容是该平台的 target/test-report.json 与 target/test-logs/。",
    "分支只留最近一次：同名目录会被覆盖。",
    "",
    `最近一次：run ${runNumber} / ${osName}`,
    "",
  ].join("\n")
);

const git = (...args) => execFileSync("git", args, { cwd: stage, stdio: "inherit" });
git("init", "-q");
git("checkout", "-q", "-b", "ci-report");
git("config", "user.email", "ci@solomni");
git("config", "user.name", "ci");
if (token) {
  // 凭据写进临时仓库的本地配置，不出现在命令行里（避免被日志记下）。
  const basic = Buffer.from(`x-access-token:${token}`).toString("base64");
  git("config", "--local", "http.https://github.com/.extraheader", `AUTHORIZATION: basic ${basic}`);
}
git("add", "-A");
git("commit", "-q", "-m", `失败报告 run ${runNumber} (${osName})`);
git("push", "--force", `https://github.com/${repo}.git`, "ci-report");
console.log("已发布失败报告到 ci-report 分支");
