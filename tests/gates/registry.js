/**
 * 门禁清单：**哪一项检查在哪、读哪些路径、属于哪条车道**只在这里登记。
 * 目的：--list 从这份清单读、--changed 从同一份 inputs 反查、自测按这份清单查完整性——
 *   影响面清单不另写一份，避免"第二权威"。
 * 纯文本 / 只读：gate 不起 cargo；docs 的两项会 spawn 已有的 docs-gov.mjs / docs-sync.mjs（判据不搬家）。
 */
"use strict";
const { execFileSync } = require("child_process");
const { matchAny } = require("../lib/fsutil.js");

const S = {
  targets: require("./structure/targets.js"),
  ledgers: require("./structure/ledgers.js"),
  map: require("./structure/map.js"),
  comments: require("./structure/comments.js"),
  platform: require("./structure/platform.js"),
  selftest: require("./structure/selftest.js"),
  docsTruth: require("./structure/docsTruth.js"),
  dependency: require("./structure/dependency.js"),
  capability: require("./structure/capability.js"),
  webLayers: require("./structure/webLayers.js"),
  size: require("./structure/size.js"),
};
const D = {
  portals: require("./docs/portals.js"),
  rootRouter: require("./docs/root-router.js"),
  templates: require("./docs/templates.js"),
  generated: require("./docs/generated.js"),
};

/** 每条记录：id（--only 用）/ lane / 所属报告步骤 / 输入（--changed 用）/ 执行函数。 */
const GATES = [
  { id: "structure/targets", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["Cargo.toml", "tests/**/*.rs"], run: (root, ctx) => S.targets.registration(root, ctx) },
  { id: "structure/orphans", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["Cargo.toml", "tests/**/*.rs"], run: (root, ctx) => S.targets.orphans(root, ctx) },
  { id: "structure/gap-format", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["tests/**/gaps.yaml"], run: (root) => S.ledgers.gapFormat(root) },
  { id: "structure/unit-map", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["docs/**/unit-map.md", "src/**/*.rs"], run: (root) => S.map.unitMap(root) },
  { id: "structure/doc-refs", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs", "docs/**/*.md"], run: (root) => S.map.docRefs(root) },
  { id: "structure/comment-contract", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs", "tests/**/*.rs"], run: (root) => S.comments.contract(root).problems },
  { id: "structure/comment-hygiene-tests", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs", "tests/**/*.rs"], run: (root) => S.comments.hygieneSelfTests(root) },
  { id: "structure/fence-literals", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs"], run: (root) => S.platform.fenceLiterals(root) },
  { id: "structure/batch-ascii", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["**/*.cmd", "**/*.bat"], run: (root) => S.platform.batchAscii(root) },
  { id: "structure/cross-marks", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["tests/**/*.rs"], run: (root) => S.platform.crossMarks(root) },
  { id: "structure/selftest/parsers", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["tests/lib/**/*.js"], run: (root) => S.selftest.parsers(root) },
  { id: "structure/selftest/outside", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["tests/lib/**/*.js"], run: (root) => S.selftest.outside(root) },
  { id: "structure/ignore-ban", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs", "tests/**/*.rs"], run: (root) => S.platform.ignoreBan(root) },
  { id: "structure/prompt-keys", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["docs/prompt/prompts.md", "prompts/**/*.yaml"], run: (root) => S.docsTruth.promptKeys(root) },
  { id: "structure/systools", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["systools/**/*.yaml", "SYSTOOL.md", "docs/tools/tools-and-roles.md"], run: (root) => S.docsTruth.systools(root) },
  { id: "structure/links", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["links.yaml", "dependencies.yaml", "env.js", "src/**/*.rs"], run: (root) => S.ledgers.links(root) },
  { id: "structure/deps-sha256", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["dependencies.yaml"], run: (root) => S.ledgers.depsSha256(root) },
  { id: "structure/dependency", lane: "structure", step: "T0 结构审查", kind: "text", inputs: ["src/**/*.rs"], run: (root) => S.dependency.direction(root) },
  {
    id: "structure/web-layers",
    lane: "structure",
    step: "T0 结构审查",
    kind: "text",
    inputs: ["src/presentation/web/assets/**/*.js", "src/presentation/web/assets/index.html"],
    run: (root, ctx) => {
      const r = S.webLayers.webLayers(root);
      ctx.stats.notes = (ctx.stats.notes || []).concat(r.notes);
      return r.problems;
    },
  },
  {
    id: "structure/file-size",
    lane: "structure",
    step: "T0 结构审查",
    kind: "text",
    inputs: ["src/**/*.rs"],
    run: (root, ctx) => {
      const r = S.size.fileSize(root);
      ctx.stats.notes = (ctx.stats.notes || []).concat(r.notes);
      return r.problems;
    },
  },
  {
    id: "structure/capability-internal",
    lane: "structure",
    step: "T0 结构审查",
    kind: "text",
    inputs: ["src/capabilities/**/*.rs"],
    run: (root, ctx) => {
      const r = S.capability.internal(root);
      ctx.stats.notes = (ctx.stats.notes || []).concat(r.notes);
      return r.problems;
    },
  },
  { id: "docs/portals", lane: "docs", step: "T0 文档审查", kind: "text", inputs: ["*.md", "docs/**/*.md"], run: (root) => D.portals.portals(root) },
  { id: "docs/root-router", lane: "docs", step: "T0 文档审查", kind: "text", inputs: ["*.md", "AGENTS.md"], run: (root) => D.rootRouter.rootRouter(root) },
  { id: "docs/templates", lane: "docs", step: "T0 文档审查", kind: "spawn", inputs: ["docs/templates/**", "docs/**/*.md"], run: (root) => D.templates.templates(root) },
  { id: "docs/generated", lane: "docs", step: "T0 文档审查", kind: "spawn", inputs: ["docs/**/*.md", "src/capabilities/vm/domain/exec.rs", "tests/**/gaps.yaml"], run: (root) => D.generated.generated(root) },
];

/** 车道 = 有序 id 列表（顺序即报告里问题的顺序；改动顺序会让 detail 字符串变化）。 */
const LANE_ORDER = {
  docs: ["docs/portals", "docs/root-router", "docs/templates", "docs/generated"],
  structure: [
    "structure/targets",
    "structure/orphans",
    "structure/gap-format",
    "structure/unit-map",
    "structure/doc-refs",
    "structure/comment-contract",
    "structure/comment-hygiene-tests",
    "structure/fence-literals",
    "structure/batch-ascii",
    "structure/cross-marks",
    "structure/selftest/parsers",
    "structure/selftest/outside",
    "structure/ignore-ban",
    "structure/prompt-keys",
    "structure/systools",
    "structure/links",
    "structure/deps-sha256",
    "structure/dependency",
    "structure/web-layers",
    "structure/file-size",
    "structure/capability-internal",
  ],
};

/** --docs / 全量 T0：文档审查 + 结构审查（顺序与既有报告一致）。 */
const LANE_ORDER_ALL = LANE_ORDER.docs.concat(LANE_ORDER.structure);

/** 自测车道：门禁自身的可信度（解析器 / 项目外写 / 注释契约三项 / 清单自检由调用方补）。 */
const LANE_ORDER_SELFTEST = ["structure/selftest/parsers", "structure/selftest/outside", "structure/comment-hygiene-tests"];

const BY_ID = new Map(GATES.map((g) => [g.id, g]));

function laneIds(name) {
  if (name === "docs") return LANE_ORDER.docs.slice();
  if (name === "structure") return LANE_ORDER.structure.slice();
  if (name === "all") return LANE_ORDER_ALL.slice();
  if (name === "selftest") return LANE_ORDER_SELFTEST.slice();
  return null;
}

function find(id) {
  return BY_ID.get(id) || null;
}

/** 跑一组 gate：问题按清单顺序拼接（报告 detail 依赖这个顺序）。 */
function runGates(root, ids) {
  const ctx = { stats: {} };
  const problems = [];
  for (const id of ids) {
    const g = BY_ID.get(id);
    if (!g) throw new Error("未知 gate：" + id);
    for (const p of g.run(root, ctx) || []) problems.push(p);
  }
  return {
    problems,
    notes: ctx.stats.notes || [],
    targets: ctx.stats.targets || [],
    testFiles: ctx.stats.testFiles || 0,
  };
}

function runLane(root, name) {
  const ids = laneIds(name);
  if (!ids) throw new Error("未知车道：" + name);
  return runGates(root, ids);
}

/** 清单自检：id 唯一、车道无悬空引用、字段齐全、inputs 至少命中一个受版本管理的文件（防漂移）。 */
function selfTest(root) {
  const problems = [];
  const bad = (msg) => problems.push("门禁清单自检失败：" + msg);
  const seen = new Set();
  for (const g of GATES) {
    if (!g.id) bad("有 gate 缺 id");
    if (seen.has(g.id)) bad("gate id 重复：" + g.id);
    seen.add(g.id);
    if (g.lane !== "structure" && g.lane !== "docs") bad(g.id + " 的 lane 只能是 structure / docs");
    if (!g.step) bad(g.id + " 缺所属步骤名");
    if (!g.kind) bad(g.id + " 缺 kind");
    if (typeof g.run !== "function") bad(g.id + " 缺 run");
    if (!Array.isArray(g.inputs) || !g.inputs.length) bad(g.id + " 缺 inputs（--changed 依赖它反查）");
  }
  for (const [lane, ids] of Object.entries(LANE_ORDER)) {
    const dup = new Set();
    for (const id of ids) {
      if (!BY_ID.has(id)) bad(lane + " 车道引用了不存在的 gate：" + id);
      if (dup.has(id)) bad(lane + " 车道有重复 gate：" + id);
      dup.add(id);
    }
    for (const g of GATES) {
      if (g.lane === lane && !ids.includes(g.id)) bad(lane + " 车道的 gate 没进顺序表：" + g.id);
    }
  }
  // inputs 至少命中一个 git 跟踪文件：glob 写错时 --changed 会静默失效。
  let files = null;
  try {
    // 含未跟踪但未被忽略的文件：新增 gate / lib 在提交前也要能自检通过。
    files = execFileSync("git", ["ls-files", "--cached", "--others", "--exclude-standard"], { cwd: root, encoding: "utf8", maxBuffer: 32 * 1024 * 1024 }).split(/\r?\n/).filter(Boolean);
  } catch (e) {
    files = null; // 没有 git（导出包 / 压缩探针）：跳过这一条，不假装验过
  }
  if (files) {
    for (const g of GATES) {
      for (const p of g.inputs) {
        if (!files.some((f) => matchAny([p], f))) bad(g.id + " 的 inputs 没命中任何受版本管理的文件：" + p);
      }
    }
  }
  return problems;
}

/** --list / --changed 共用的只读视图（不含函数，方便 JSON 化）。 */
function listModel() {
  return GATES.map((g) => ({ id: g.id, lane: g.lane, step: g.step, kind: g.kind, inputs: g.inputs.slice() }));
}

module.exports = { GATES, LANE_ORDER, LANE_ORDER_ALL, LANE_ORDER_SELFTEST, laneIds, find, runGates, runLane, selfTest, listModel };
