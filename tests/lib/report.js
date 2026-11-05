/**
 * 测试报告：target/test-report.json 的**唯一写者**与字段定义处。
 * schema 只增不改（CI 的 ci-merge / ci-publish 读同一份字段）；模式靠 mode 区分。
 * 缺口账也在这里读：报告里的 gaps / globalGaps 是同一份账本的投影，不另存一份。
 */
"use strict";
const fs = require("fs");
const path = require("path");

const REPORT_REL = path.join("target", "test-report.json");

function reportPath(root) {
  return path.join(root, REPORT_REL);
}

/** 环境事实：rustc / cargo / git 的首行版本（取不到如实给 null，不猜）。 */
function detectToolchain(proc) {
  const one = (cmd) => {
    const r = proc.capture(cmd, ["--version"]);
    if (!r || r.code !== 0) return null;
    const line = String(r.out || "").split(/\r?\n/).map((s) => s.trim()).find(Boolean);
    return line || null;
  };
  return { rustc: one("rustc"), cargo: one("cargo"), git: one("git") };
}

/** 写报告（steps 统一裁成 5 个字段；raw / skips 不进文件，只留在控制台）。 */
function write(root, meta, steps) {
  const p = reportPath(root);
  const report = {
    mode: meta.mode,
    platform: process.platform,
    arch: process.arch,
    osKey: meta.osKey,
    profile: meta.profile,
    fenceLive: !!meta.fenceLive,
    doctor: meta.doctor === undefined ? null : meta.doctor,
    steps: (steps || []).map((s) => ({ step: s.step, status: s.status, detail: s.detail, ms: s.ms, slow: !!s.slow })),
    envSkips: meta.envSkips || [],
    quality: meta.quality || { failed: 0, steps: [] },
    globalGaps: meta.globalGaps || [],
    gaps: meta.gaps || [],
    timeouts: meta.timeouts || [],
    failed: meta.failed || 0,
    toolchain: meta.toolchain || { rustc: null, cargo: null, git: null },
  };
  fs.mkdirSync(path.dirname(p), { recursive: true });
  fs.writeFileSync(p, JSON.stringify(report, null, 2));
  return p;
}

/** 读一份缺口账里的 id（跨平台层 + 本平台层）：条目存在 = 这条测试还没有。 */
function readPlatformGaps(root, osKey) {
  const files = [path.join(root, "tests", "cross-platform", "gaps.yaml"), path.join(root, "tests", osKey, "gaps.yaml")];
  const ids = [];
  for (const f of files) {
    if (!fs.existsSync(f)) continue;
    for (const line of fs.readFileSync(f, "utf8").split(/\r?\n/)) {
      const m = line.match(/^\s*-\s*id:\s*(\S+)/);
      if (m) ids.push(m[1]);
    }
  }
  return ids;
}

/** 读全局缺口账（tests/gaps.yaml）：长期目标，每条都进报告，但不影响平台 ACCEPTED 判定。 */
function readGlobalGaps(root) {
  const ids = [];
  const f = path.join(root, "tests", "gaps.yaml");
  if (!fs.existsSync(f)) return ids;
  for (const line of fs.readFileSync(f, "utf8").split(/\r?\n/)) {
    const m = line.match(/^\s*-\s*id:\s*(\S+)/);
    if (m) ids.push(m[1]);
  }
  return ids;
}

module.exports = { reportPath, detectToolchain, write, readPlatformGaps, readGlobalGaps, REPORT_REL };
