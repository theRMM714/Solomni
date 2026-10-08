// CI 发布入口的合并步骤（契约见 docs/testing/execution-ci.md）：每个平台由 quality 与 e2e 两个 job
// 各交一份报告，这里并成该平台唯一的 target/test-report.json，再交给 tests/ci-publish.mjs 发布。
// 任何一半缺报告都补一条 fail 步骤——"没跑到"不能读成"通过"。
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const REPORT = path.join(ROOT, "target", "test-report.json");
const E2E = path.join(ROOT, "target", "e2e-report.json");

// 报告读不动（缺文件 / JSON 坏了）一律当"这一半没产出"：合并补 fail 步骤，绝不让发布整段中断。
const read = (p) => {
  if (!fs.existsSync(p)) return null;
  try {
    return JSON.parse(fs.readFileSync(p, "utf8"));
  } catch (e) {
    console.log("::warning title=报告不可读::" + path.relative(ROOT, p) + "：" + e.message);
    return null;
  }
};

function main() {
  const quality = read(REPORT);
  const e2e = read(E2E);
  if (!quality && !e2e) {
    console.log("::warning title=没有可合并的报告::quality 与 e2e 两个 job 都没产出报告");
    return;
  }

  const steps = [];
  // quality 那一半按契约不带 L4（run-tests.js --skip-e2e）；万一漏带也在这里剔除，避免与 e2e 的 L4 重复。
  if (quality) steps.push(...(quality.steps || []).filter((s) => s.step !== "L4 端到端"));
  else steps.push({ step: "质量门禁 + 单元（quality job）", status: "fail", detail: "quality job 未产出 test-report.json", ms: 0 });
  if (e2e) steps.push(...(e2e.steps || []));
  else steps.push({ step: "L4 端到端", status: "fail", detail: "e2e job 未产出 e2e-report.json", ms: 0 });

  const base = quality || e2e;
  const failed = steps.filter((s) => s.status === "fail");
  const qualityFailed = steps.filter((s) => s.status === "quality-fail");
  const report = {
    platform: base.platform,
    arch: base.arch,
    osKey: base.osKey,
    profile: base.profile || "debug",
    fenceLive: base.fenceLive === undefined ? true : base.fenceLive,
    doctor: quality ? quality.doctor : null,
    steps: steps.map((s) => ({ step: s.step, status: s.status, detail: s.detail, ms: s.ms })),
    envSkips: [].concat(quality ? quality.envSkips || [] : [], e2e ? e2e.envSkips || [] : []),
    quality: { failed: qualityFailed.length, steps: qualityFailed.map((s) => ({ step: s.step, detail: s.detail })) },
    globalGaps: base.globalGaps || [],
    gaps: base.gaps || [],
    failed: failed.length,
  };
  fs.writeFileSync(REPORT, JSON.stringify(report, null, 2));
  console.log(
    "[ci-merge] 已合并 " + (quality ? "quality" : "（缺 quality）") + " + " + (e2e ? "e2e" : "（缺 e2e）") +
    " → " + path.relative(ROOT, REPORT) + "：steps=" + report.steps.length +
    " failed=" + report.failed + " qualityFailed=" + report.quality.failed
  );
}

main();
