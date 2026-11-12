// 浏览器 E2E（L4，由 orchestrator.js 用 E2E_DRIVER=browser.cjs 调起）：
// 复用同一套夹具 / 假供应商 / 端口，只把驱动换成真实浏览器里的关键旅程。
// 判据是可见行为：建会话、流式回复、工具裁决卡、设置保存、存储回收、上传。
const fs = require("fs");
const path = require("path");
const { spawnSync } = require("child_process");

const BASE = process.env.E2E_BASE || "http://127.0.0.1:3099";
const MOCK_BASE = process.env.E2E_MOCK_BASE || "http://127.0.0.1:8397";
const ROOT = path.resolve(__dirname, "..", "..", "..");
const ARTIFACT_ROOT = path.join(ROOT, "target", "e2e-browser");
const BROWSERS_PATH = path.join(ROOT, "target", "playwright");
// 系统浏览器优先；只有都起不来才下载。路径在 require playwright 之前就定好。
process.env.PLAYWRIGHT_BROWSERS_PATH = process.env.PLAYWRIGHT_BROWSERS_PATH || BROWSERS_PATH;
const { chromium } = require("playwright");

let failed = 0;
const consoleErrors = [];
const httpErrors = [];
function log(ok, label, extra) {
  console.log((ok ? "PASS " : "FAIL ") + label + (ok || extra === undefined ? "" : " :: " + extra));
  if (!ok) failed++;
}
function assert(cond, label, extra) {
  log(!!cond, label, extra);
  if (!cond) throw new Error(label);
}

async function api(method, p, body) {
  const r = await fetch(BASE + p, {
    method,
    headers: { "Content-Type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(30000),
  });
  const text = await r.text();
  let json = null;
  try { json = JSON.parse(text); } catch {}
  return { status: r.status, json, text };
}
const reg = (id, body) => api("POST", "/api/actions/" + id, body || {});
const act = (sid, id, body) => api("POST", "/api/actions/" + id, Object.assign({ session_id: sid }, body || {}));

async function waitFor(fn, ms, label) {
  const deadline = Date.now() + (ms || 20000);
  for (;;) {
    const v = await fn();
    if (v) return v;
    if (Date.now() > deadline) throw new Error("超时：" + label);
    await new Promise((r) => setTimeout(r, 100));
  }
}

async function findSession(name) {
  const s = await api("GET", "/api/state");
  return (((s.json && s.json.sessions) || []).find((x) => x.sid === name)) || null;
}

async function registerFixture() {
  assert((await reg("upsert_provider", { id: "mock", base_url: MOCK_BASE + "/v1", api_key: "k" })).status === 200, "登记假供应商");
  assert((await reg("upsert_model", { id: "m1", name: "M1", api_model: "m1", provider: "mock", note: "", tools: "envelope" })).status === 200, "登记模型 m1");
  assert((await reg("upsert_model", { id: "m2", name: "M2", api_model: "m2", provider: "mock", note: "", tools: "envelope" })).status === 200, "登记模型 m2");
  assert((await reg("set_core_model", { id: "m1" })).status === 200, "设核心默认 m1");
  assert((await reg("upsert_agent", { name: "单兵", modules: ["summarizer"], model: "m1", note: "e2e" })).status === 200, "登记 agent 单兵");
  assert((await reg("upsert_agent", { name: "双子", modules: ["research", "reviewer"], model: "m2", note: "e2e 多模块" })).status === 200, "登记 agent 双子");
  await waitFor(async () => {
    const s = await api("GET", "/api/state");
    const names = ((s.json && s.json.agents) || []).map((a) => a.name);
    return names.indexOf("单兵") >= 0 && names.indexOf("双子") >= 0;
  }, 10000, "登记处刷新");
}

async function launchBrowser() {
  const attempts = [];
  for (const channel of ["msedge", "chrome"]) {
    try {
      const b = await chromium.launch({ channel, headless: true });
      console.log("[browser] 使用系统浏览器：" + channel);
      return b;
    } catch (e) {
      attempts.push(channel + ": " + String(e.message || e).split("\n")[0]);
    }
  }
  try {
    const b = await chromium.launch({ headless: true });
    console.log("[browser] 使用 Playwright 自带 Chromium");
    return b;
  } catch (e) {
    attempts.push("bundled: " + String(e.message || e).split("\n")[0]);
  }
  console.log("[browser] 系统浏览器不可用，下载 Chromium 到 target/playwright");
  const cli = path.join(ROOT, "node_modules", "playwright", "cli.js");
  assert(fs.existsSync(cli), "Playwright CLI 存在（先 npm ci）", cli);
  const install = spawnSync(process.execPath, [cli, "install", "chromium"], {
    cwd: ROOT,
    stdio: "inherit",
    env: Object.assign({}, process.env, { PLAYWRIGHT_BROWSERS_PATH: BROWSERS_PATH }),
  });
  assert(install.status === 0, "下载 Playwright Chromium", "exit " + install.status);
  const b = await chromium.launch({ headless: true });
  console.log("[browser] 使用下载的 Chromium");
  return b;
}

async function saveArtifacts(context, page, stamp) {
  const dir = path.join(ARTIFACT_ROOT, stamp);
  fs.mkdirSync(dir, { recursive: true });
  try { await page.screenshot({ path: path.join(dir, "failure.png"), fullPage: true }); } catch {}
  try { fs.writeFileSync(path.join(dir, "failure.html"), await page.content()); } catch {}
  try { fs.writeFileSync(path.join(dir, "console.log"), consoleErrors.concat(httpErrors).join("\n") + "\n"); } catch {}
  try { await context.tracing.stop({ path: path.join(dir, "trace.zip") }); } catch {}
  console.log("[browser] 失败现场：" + path.relative(ROOT, dir));
  return dir;
}

(async () => {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  fs.mkdirSync(ARTIFACT_ROOT, { recursive: true });
  await registerFixture();

  const browser = await launchBrowser();
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 }, locale: "zh-CN" });
  await context.tracing.start({ screenshots: true, snapshots: true });
  const page = await context.newPage();
  page.setDefaultTimeout(20000);
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    const where = (m.location && m.location().url) || "";
    if (/favicon\.ico/.test(m.text() + where)) return;
    consoleErrors.push("console: " + m.text() + (where ? " @ " + where : ""));
  });
  page.on("pageerror", (e) => consoleErrors.push("pageerror: " + (e.message || String(e))));
  // 非 2xx 的 API 回包留原文：失败现场要能直接看出后端拒绝的理由。
  page.on("response", async (r) => {
    if (r.status() < 400 || /favicon\.ico/.test(r.url())) return;
    let body = "";
    try { body = (await r.text()).slice(0, 800); } catch {}
    httpErrors.push("http " + r.status() + " " + r.url() + " " + body);
  });
  console.log("[browser] browser.version = " + browser.version());

  let workName = "";
  try {
    // ① 首页连接真实服务：应用就绪看连接状态；DOMContentLoaded 会被任一 defer 资源（如 md.js）拖住。
    await page.goto(BASE, { waitUntil: "commit" });
    await page.locator("#conn-text").waitFor({ state: "visible" });
    await waitFor(async () => (await page.locator("#conn-text").textContent()) === "已连接", 20000, "连接状态");
    console.log("PASS 首页显示已连接");

    // ② 向导建会话：名称 → 从登记处选 agent → 创建
    workName = "ui-browser-" + Date.now();
    await page.click("#btn-new-work");
    await page.locator(".modal-title", { hasText: "新建工作" }).waitFor();
    await page.fill('input[placeholder^="工作名称"]', workName);
    await page.click('button:has-text("从登记处加入")');
    await page.locator(".modal-title", { hasText: "选择 agent" }).waitFor();
    await page.locator('.reg-item:has-text("单兵") button:has-text("选择")').click();
    await page.locator(".modal-title", { hasText: "新建工作" }).waitFor();
    await page.click('button:has-text("创建并开始")');
    await page.locator(".modal-title", { hasText: "新建工作" }).waitFor({ state: "detached" });
    const created = await waitFor(() => findSession(workName), 30000, "新会话落盘");
    console.log("PASS 向导创建会话：" + workName + "（" + created.sid + "）");
    const sid = created.sid;
    await page.locator("#tabs .tab", { hasText: workName }).waitFor();

    // ③ 发送并看到流式回复（默认档位允许 write：假供应商先调工具、拿到结果后收尾）
    await page.fill("#input", "你好，先说一句");
    await page.click("#btn-send");
    await page.locator("#stream .turn-text").getByText("收到", { exact: false }).first().waitFor({ timeout: 30000 });
    console.log("PASS 发送消息并收到回复");

    // 把 write 设成逐工具确认：下面要验真实裁决卡。
    assert((await act(sid, "edit_session", {
      agents: [{ name: "单兵", modules: ["summarizer"], model: "m1" }],
      permission: { tools: { write: "ask" } },
      tier: "host",
    })).status === 200, "设置 write 需确认");

    // ④ 工具裁决卡：真停下、真提交 allow、卡消失
    await page.fill("#input", "工具确认：放行");
    await page.click("#btn-send");
    const card = page.locator(".gate-card").first();
    await card.waitFor({ timeout: 30000 });
    await card.locator("button", { hasText: /放行/ }).first().click();
    await card.waitFor({ state: "detached", timeout: 30000 });
    await page.locator("#stream .turn-text").getByText("收到", { exact: false }).first().waitFor({ timeout: 30000 });
    console.log("PASS 裁决卡提交后继续生成");

    // ⑤ 设置保存
    await page.click("#btn-settings");
    await page.locator(".modal-title", { hasText: "设置" }).waitFor();
    await page.locator('.settings-foot button:has-text("保存")').click();
    await page.locator(".toast", { hasText: /设置已保存|已保存/ }).first().waitFor({ timeout: 20000 });
    console.log("PASS 设置保存");

    // ⑥ 存储回收：有可回收项就确认，没有就接受空提示；两种都不能静默无反馈
    await page.locator('.settings-nav button[data-id="storage"]').click();
    await page.locator(".storage-report .storage-actions").waitFor();
    await page.locator('.storage-report button:has-text("按策略回收")').click();
    let storage = "";
    await waitFor(async () => {
      if (await page.locator(".notice-title", { hasText: "回收预览" }).isVisible().catch(() => false)) {
        await page.locator('.notice-panel .notice-actions button:has-text("回收")').click();
        storage = "回收";
        return true;
      }
      if (await page.locator(".toast", { hasText: "没有达到策略条件" }).isVisible().catch(() => false)) {
        storage = "空";
        return true;
      }
      return false;
    }, 20000, "存储回收反馈");
    if (storage === "回收") {
      await page.locator(".toast", { hasText: "已回收" }).first().waitFor({ timeout: 20000 });
      console.log("PASS 存储回收（有可回收项，确认后完成）");
    } else {
      console.log("PASS 存储回收（当前没有到策略条件的项，给出明确提示）");
    }

    // ⑦ 上传：文件选择器 → work/
    await page.keyboard.press("Escape");
    await page.locator(".modal-title", { hasText: "设置" }).waitFor({ state: "detached" });
    const [chooser] = await Promise.all([
      page.waitForEvent("filechooser"),
      page.click("#btn-upload"),
    ]);
    await chooser.setFiles({ name: "e2e-upload.txt", mimeType: "text/plain", buffer: Buffer.from("来自浏览器 E2E\n") });
    await page.locator(".toast", { hasText: "已上传到 work/" }).first().waitFor({ timeout: 20000 });
    console.log("PASS 浏览器上传文件到 work/");

    if (consoleErrors.length) {
      log(false, "浏览器控制台无错误", consoleErrors.slice(0, 5).join(" | "));
    } else {
      console.log("PASS 浏览器控制台无错误");
    }
  } catch (e) {
    log(false, "浏览器旅程", String(e && e.message ? e.message : e) + (httpErrors.length ? " | " + httpErrors.slice(-3).join(" | ") : ""));
    await saveArtifacts(context, page, stamp);
  } finally {
    if (failed === 0) {
      try { await context.tracing.stop(); } catch {}
    } else {
      try { await context.tracing.stop({ path: path.join(ARTIFACT_ROOT, stamp, "trace.zip") }); } catch {}
    }
    await context.close().catch(() => {});
    await browser.close().catch(() => {});
  }

  console.log(failed === 0 ? "BROWSER-E2E-OK" : "BROWSER-E2E-FAILED " + failed);
  process.exit(failed === 0 ? 0 : 1);
})().catch((e) => {
  console.error("BROWSER-E2E-FAILED " + (e && e.stack ? e.stack : e));
  process.exit(1);
});
