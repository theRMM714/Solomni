/* 目的：截图与审计用的合成数据——形状对齐 /api/state 与 /api/history 的真实契约，内容全部虚构。
 * 管：state / histories / work / files / tiers 五类响应，以及 empty 变体。
 * 不管：网络与浏览器（tests/screenshots/shots.cjs）；不读 .home、不读任何真实会话。
 * 联动：由 shots.cjs 的注入 shim 按 URL 路由；改了接口形状要同步前端的读法。 */
"use strict";

// 固定时钟：2026-11-11 02:00:00 +08:00。冻结后 timeAgo / 耗时都稳定，截图可比。
const NOW = 1762797600000;

const gateCard = {
  id: "card-7",
  type: "decision_card",
  envelope: { name: "builder" },
  message: {
    title: "要现在改写 26 个样式类，还是等令牌落地后一起删？",
    body: "这些类在 app/ 里没有任何引用，但 ui.css 里仍有定义。",
    detail: "先删会让同一份 CSS 搬两次；后删能一次改到位，但中间会多一个提交。",
  },
  options: [
    { id: "now", label: "现在就删" },
    { id: "later", label: "等令牌落地" },
  ],
  waiting: [
    { envelope: { name: "checker" }, title: "冒烟是否要覆盖 dead-class 检查" },
  ],
};

const state = {
  modules: [
    { id: "research", brief: "资料检索与出处核对", runtimes: ["python"] },
    { id: "release", brief: "构建、冒烟与发布检查", runtimes: ["node"] },
  ],
  providers: [
    { id: "anthropic", base_url: "https://api.anthropic.com" },
  ],
  models: [
    {
      id: "claude-sonnet-4-5", name: "Claude Sonnet 4.5", api_model: "claude-sonnet-4-5",
      provider: "anthropic", tools: "native", context: 200000, is_core: true,
      note: "日常编码与长文；原生工具调用。",
      thinking: { default: "high", levels: { low: "low", high: "high" } },
    },
    {
      id: "gpt-5.1", name: "GPT-5.1", api_model: "gpt-5.1",
      provider: "openai", tools: "native", context: 400000, is_core: false,
      note: "长上下文备选。",
      thinking: { default: null, levels: {} },
    },
  ],
  core: "claude-sonnet-4-5",
  rejected: [],
  agents: [
    { name: "builder", model: "claude-sonnet-4-5", modules: ["release"], prompt: "实现与自测" },
    { name: "checker", model: "", modules: ["research"], prompt: "验收与核对出处" },
  ],
  history: [
    { name: "web-art", mode: "single", done: false, run: "active", parent: "", running: true, tier: "host", tier_ready: true },
    { name: "web-art--tokens", mode: "single", done: true, run: "active", parent: "web-art", running: false, tier: "host", tier_ready: true },
    { name: "release-check", mode: "single", done: true, run: "active", parent: "", running: false, tier: "host", tier_ready: true },
    { name: "storage-cleanup", mode: "single", done: true, run: "stopped", parent: "", running: false, tier: "host", tier_ready: true },
  ],
  settings: {
    streaming: true, show_reasoning: true, llm_timeout_secs: 300,
    discuss_remind_cap: 3, compact_at_percent: 70, compact_keep_percent: 15, compact_model: "",
  },
  fence: { effective_fs: true, effective_net: true, sandbox_missing: [], sandbox_degraded: false },
  toolchains: { missing: [], fallback: {} },
  sessions: [
    {
      sid: "web-art", mode: "single", run: "active", running: true,
      pending: gateCard,
      context: {
        prompt_tokens: 38210, completion_tokens: 1204, total_tokens: 39414,
        window: 200000, exact: true, cache_read: 12000, cache_creation: 800,
      },
      shell: {
        command: "node run-tests.js", scope: "全量", cwd: "/demo/work",
        started_at: 1762797500, finished_at: 1762797580, elapsed_secs: 80.4,
        exit_code: 0, ok: true, status: "done",
        output: "FRONTEND-SMOKE-OK\nTOKENS-SMOKE-OK\nALL TESTS PASSED",
      },
      permission_label: "工作区", permission_profile: "workspace",
      thinking: {
        level: null, default_level: "high", label: "高",
        options: [{ id: "low", label: "低" }, { id: "high", label: "高" }], note: "",
      },
      tier: "host", tier_ready: true, can_update_task: true,
    },
  ],
};

const webArtEvents = [
  {
    type: "transcript",
    lines: [
      { id: 1, kind: "user", speaker: "用户", line: "把设置页的报错改成一句话，原理和影响范围收进 ⓘ。" },
      {
        id: 2, kind: "msg", speaker: "Claude", verb: "say",
        line: "先确认现状。设置页有 12 处错误提示，其中 5 处把「为什么」和「怎么办」写在同一句里。\n\n- 首层保留：发生了什么、要不要做决定\n- 收进详情：原理、影响范围、回滚方式\n\n保存失败时先给一句人话，再把原因放进详情。",
      },
      {
        id: 3, kind: "tool", speaker: "Claude",
        tool: {
          speaker: "Claude", module: "work", name: "read", ok: true,
          args: "path: work/app/features/settings/pages.js\nlimit: 120",
          output: "settings/pages.js  1-120 已读\n发现 12 处 hint(...)",
        },
      },
      {
        id: 4, kind: "msg", speaker: "Claude", verb: "say",
        reasoning: "先看 hint 的调用点，再决定哪些句子属于「决定所需」，哪些只是解释。",
        line: "找到 3 处需要拆开的说明：删除会话、清空存储、切换权限档。其余 9 处只改措辞，不动结构。",
      },
      {
        id: 5, kind: "tool", speaker: "Claude",
        tool: {
          speaker: "Claude", module: "work", name: "shell", ok: false,
          args: "command: node run-tests.js --docs",
          output: "docs structure: 1 file failed\n  docs/presentation/web/README.md: missing anchor",
        },
      },
      {
        id: 6, kind: "system", speaker: "核心",
        line: "验收清单已生成：3 项通过、1 项待补（文档锚点）。",
      },
    ],
  },
  { type: "rewind", mode: "archive", label: "重构前" },
  { type: "plan", text: "1. 拆长文案，首层 ≤40 字\n2. 补 ⓘ 与「详情」\n3. 跑对比度门禁" },
  {
    type: "review",
    items: [
      { status: "pass", item: "首层文案 ≤40 字", note: "" },
      { status: "pass", item: "安全类提示保留", note: "" },
      { status: "fail", item: "文档锚点", note: "README.md 的路由表未同步" },
    ],
  },
  {
    type: "usage", prompt_tokens: 38210, completion_tokens: 1204,
    total_tokens: 39414, window: 200000, exact: true, cache_read: 12000, cache_creation: 800,
  },
];

const readonlyEvents = [
  {
    type: "transcript",
    lines: [
      { id: 1, kind: "user", speaker: "用户", line: "发布前把冒烟、对比度和截图三件事跑一遍。" },
      {
        id: 2, kind: "msg", speaker: "Claude", verb: "say",
        line: "三件事都过了：\n\n1. 冒烟全绿\n2. 对比度最低 5.32:1\n3. 截图在 .shots/after\n\n没有改路由和后端。",
      },
      { id: 3, kind: "notice", speaker: "", line: "本会话已收尾，不再被派发。" },
    ],
  },
];

const emptyState = Object.assign({}, state, {
  modules: [], providers: [], models: [], agents: [], history: [], sessions: [], core: null,
});

const histories = {
  "web-art": { meta: { name: "web-art", mode: "single", done: false }, events: webArtEvents, live: [], head: 1 },
  "web-art--tokens": {
    meta: { name: "web-art--tokens", mode: "single", done: true, parent: "web-art" },
    events: [{
      type: "transcript",
      lines: [
        { id: 1, kind: "user", speaker: "用户", line: "先只换令牌，不动布局。" },
        { id: 2, kind: "msg", speaker: "Claude", verb: "say", line: "令牌已换：底色、面、抬升、正文、次文、强调。布局没动。" },
      ],
    }],
    live: [], head: 1,
  },
  "release-check": { meta: { name: "release-check", mode: "single", done: true }, events: readonlyEvents, live: [], head: 1 },
  "storage-cleanup": {
    meta: { name: "storage-cleanup", mode: "single", done: true },
    events: [{ type: "transcript", lines: [{ id: 1, kind: "user", speaker: "用户", line: "清掉 30 天前的构建缓存。" }] }],
    live: [], head: 1,
  },
};

const work = {
  head: { id: "4f2a1c9", time: NOW - 4 * 60 * 1000, author: "builder", message: "转录页：状态轨与工具回执" },
  total: 4,
  commits: [
    { id: "4f2a1c9", author: "builder", time: NOW - 4 * 60 * 1000, message: "转录页：状态轨与工具回执", on_head: true },
    { id: "9c1e0b2", author: "checker", time: NOW - 26 * 60 * 1000, message: "验收：对比度与焦点环", on_head: false },
    { id: "17aa3d5", author: "builder", time: NOW - 58 * 60 * 1000, message: "令牌：中性色层级与字号阶梯", on_head: false },
    { id: "b0d4e71", author: "用户", time: NOW - 3 * 60 * 60 * 1000, message: "上传：设计稿截图三张", on_head: false },
  ],
  files: [
    { path: "work/app/features/settings/pages.js", change: "M", added: 18, deleted: 11 },
    { path: "work/app/features/transcript/stream.js", change: "M", added: 9, deleted: 2 },
  ],
};

const files = {
  work: ["README.md", "app/features/settings/pages.js", "app/features/transcript/stream.js"],
  agents: [],
  roots: { work: "/demo/work", agents: [] },
  usage: {
    files: 3, bytes: 48210,
    work: { files: 3, bytes: 48210 },
    agents: {},
  },
};

const tiers = {
  tiers: {
    default: "host", vm_available: false, vm_unavailable_reason: "本机未启用虚拟化平台",
    vm_requirements: [], guests: [], available: {},
  },
};

function byQuery(search) {
  const q = new URLSearchParams(search || "");
  return q.get("empty") === "1" ? emptyState : state;
}

module.exports = { NOW, state, emptyState, histories, work, files, tiers, byQuery, gateCard };
