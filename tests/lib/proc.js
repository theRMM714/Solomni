/**
 * 子进程与环境：门禁里所有命令只从这里起，环境解析统一交给 env.js（与启动器同一份）。
 * 输出走**文件**而不是管道：受限环境里抓管道会 EPERM；落成日志还顺带留了档案与证据。
 * 每步都有墙钟上限：挂死的子进程必须被终止，不能把本地入口永久卡住。
 */
"use strict";
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const DEFAULT_TIMEOUT_MS = 15 * 60 * 1000;

function create(opts) {
  const root = opts.root;
  const fenceLive = !!opts.fenceLive;
  const logDir = path.join(root, "target", "test-logs");
  const timeouts = [];

  /** 子进程环境：env.js 解析 + 项目内工具区进 PATH + 围栏开关传给测试与产品。 */
  function buildEnv() {
    const envLayer = require(path.join(root, "env.js"));
    const resolved = envLayer.resolve();
    const e = Object.assign({}, resolved ? resolved.env : process.env);
    envLayer.pinTemp(e);
    const toolsBin = path.join(root, ".tools", "bin");
    if (fs.existsSync(toolsBin)) {
      const pk = Object.keys(e).find((k) => k.toUpperCase() === "PATH") || "PATH";
      e[pk] = toolsBin + path.delimiter + (e[pk] || "");
    }
    e.SOLOMNI_FENCE_LIVE = fenceLive ? "1" : "0";
    e.SOLOMNI_FENCE_WRITE = fenceLive ? "1" : "0";
    return e;
  }

  function sh(cmd, args, timeoutMs) {
    fs.mkdirSync(logDir, { recursive: true });
    const slug = (s) => String(s).replace(/[^\w.-]+/g, "_");
    const logFile = path.join(logDir, slug(path.basename(cmd)) + "-" + args.map(slug).join("_") + ".log");
    const fd = fs.openSync(logFile, "w");
    // 跑之前先报出这一步的实时日志落点：门禁卡住时直接读这个文件就知道停在哪（子进程边走边写）。
    process.stdout.write("    实时日志：" + path.relative(root, logFile) + "\n");
    const budget = timeoutMs || DEFAULT_TIMEOUT_MS;
    const r = spawnSync(cmd, args, { cwd: root, env: buildEnv(), stdio: ["ignore", fd, fd], timeout: budget, killSignal: "SIGKILL" });
    fs.closeSync(fd);
    const out = fs.readFileSync(logFile, "utf8");
    const timedOut = !!(r.error && (r.error.code === "ETIMEDOUT" || /timed?\s*out/i.test(String(r.error.message))));
    if (timedOut) timeouts.push("超时 " + Math.round(budget / 1000) + "s：" + path.basename(cmd) + " " + args.join(" "));
    return { code: r.error ? -1 : r.status, out, error: r.error ? String(r.error.message) : null, timedOut, log: path.relative(root, logFile) };
  }

  /** 轻量一次性取输出（不进日志、不进报告）：用于 toolchain 版本这类环境事实；管道不可用时如实返回失败。 */
  function capture(cmd, args) {
    try {
      const r = spawnSync(cmd, args, { cwd: root, env: buildEnv(), encoding: "utf8", timeout: 30000 });
      if (r.error) return { code: -1, out: "" };
      return { code: r.status === null ? -1 : r.status, out: String(r.stdout || "") };
    } catch (e) {
      return { code: -1, out: "" };
    }
  }

  /** 工具缺失 = env-skip（不静默算过）。 */
  function toolAvailable(sub) {
    return sh("cargo", [sub, "--version"]).code === 0;
  }

  return { root, fenceLive, logDir, timeouts, buildEnv, sh, capture, toolAvailable };
}

module.exports = { create, DEFAULT_TIMEOUT_MS };
