/* 冒烟：app/core 的 HTTP 封装与事件流传输层（桩 fetch / 桩定时器，不起浏览器）。
 * 用法：node src/presentation/web/assets/app/core/core.smoke.cjs   （输出 CORE-SMOKE-OK / FAIL）
 * 盯的契约：错误形状 {status,data}；只有「取不到」才算断线；先对齐裁剪、再推进游标、最后交批次；
 *   钩子异常走 onError 且轮询继续。 */
const { createApi } = require("./http.js");
const { createEventStream } = require("./events.js");

const fails = [];
const check = (ok, name) => { if (!ok) fails.push(name); };

async function apiCases() {
  const calls = [];
  const api = createApi(async (url, init) => {
    calls.push({ url, init });
    if (url === "/bad") return { ok: false, status: 400, json: async () => ({ error: "坏请求", extra: 1 }) };
    if (url === "/nojson") return { ok: false, status: 502, json: async () => { throw new Error("x"); } };
    return { ok: true, json: async () => ({ ok: true }) };
  });
  check((await api("GET", "/a")).ok === true, "成功回包原样返回");
  check(calls[0].init.headers === undefined && calls[0].init.body === undefined, "无 body 不带头与体");
  await api("POST", "/b", { x: 1 });
  check(calls[1].init.headers["Content-Type"] === "application/json" && calls[1].init.body === '{"x":1}', "有 body 发 JSON");
  try { await api("GET", "/bad"); check(false, "非 2xx 必须抛"); } catch (e) {
    check(e.message === "坏请求" && e.status === 400 && e.data.extra === 1, "错误带后端文案 / status / data");
  }
  try { await api("GET", "/nojson"); check(false, "非 JSON 错误也要抛"); } catch (e) {
    check(e.message === "HTTP 502" && e.status === 502, "无 JSON 时文案退回 HTTP 码");
  }
}

async function streamCases() {
  const log = [];
  const urls = [];
  const replies = [
    () => { throw new Error("断了"); },
    () => ({ ok: false, status: 502, json: async () => ({}) }),
    () => ({ ok: true, json: async () => ({ lines: [{ seq: 1, sid: "a", events: [] }], head: 1, oldest: 1 }) }),
    () => ({ ok: true, json: async () => ({ lines: "boom", head: 5, oldest: 4 }) }),
    () => ({ ok: true, json: async () => ({ lines: [], oldest: 4 }) }),
  ];
  let done;
  const finished = new Promise((r) => { done = r; });
  const stream = createEventStream({
    fetch: async (url) => {
      urls.push(url);
      const next = replies.shift();
      if (!next) { done(); return new Promise(() => {}); }
      return next();
    },
    setTimeout: (fn, ms) => { log.push("wait:" + ms); fn(); },
    onConn: (ok) => log.push(ok ? "conn:ok" : "conn:bad"),
    onTrim: (oldest) => log.push("trim:" + oldest),
    onLines: (lines) => {
      if (lines === "boom") throw new Error("应用出错");
      log.push("lines:" + lines.length);
    },
    onError: (e) => log.push("err:" + e.message),
  });
  stream.run();
  stream.run();
  await finished;
  check(JSON.stringify(log) === JSON.stringify([
    "conn:bad", "wait:3000", "conn:bad", "wait:3000",
    "conn:ok", "trim:1", "lines:1",
    "conn:ok", "trim:4", "err:应用出错",
    "conn:ok", "trim:4", "lines:0",
  ]), "事件顺序：断线重试 → 对齐 → 批次；应用出错不算断线 · 实际 " + JSON.stringify(log));
  check(urls[0] === "/api/events?sid=&since=0", "从 0 开始取");
  check(urls[3] === "/api/events?sid=&since=1", "按 head 推进游标");
  check(urls[4] === "/api/events?sid=&since=5", "批次应用出错也推进游标（与契约一致：游标在交批次前推进）");
  check(urls[5] === "/api/events?sid=&since=5", "缺 head 时游标不动");
  check(stream.cursor() === 5, "cursor() 读到当前游标");
  check(urls.length === 6, "重复 run() 不起第二条轮询");
}

(async () => {
  try { await apiCases(); } catch (e) { fails.push("api 用例异常：" + e.message); }
  try { await streamCases(); } catch (e) { fails.push("事件流用例异常：" + e.message); }
  if (fails.length) {
    for (const f of fails) console.log("FAIL " + f);
    console.log("CORE-SMOKE-FAIL");
    process.exit(1);
  }
  console.log("CORE-SMOKE-OK");
})();
