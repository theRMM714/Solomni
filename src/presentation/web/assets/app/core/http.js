/* 目的：HTTP 调用的唯一封装（JSON 进出 + 统一错误形状 + 有界等待）。
 * 管：发请求、解析 JSON、非 2xx 抛出带 `status` / `data` 的 Error（文案取后端 `{error}`）；
 *   调用方给 `timeoutMs` 时按 AbortSignal 中止，抛带 `timeout: true` 的 Error。
 * 不管：路由含义、重试、长轮询（`app/core/events.js`）；不读任何界面状态。
 * 联动：`fetch` 与定时器由调用方注入（`app/main.js`），冒烟可换桩；路由目录见 `src/presentation/web/routes.rs`。 */

const DEFAULT_TIMEOUT_MS = 0;

/**
 * @param {(url: string, init?: object) => Promise<{ok: boolean, status?: number, json: () => Promise<any>}>} fetchImpl
 * @param {{ setTimeout?: (fn: () => void, ms: number) => any, clearTimeout?: (t: any) => void }} [timers]
 * @returns {(method: string, url: string, body?: any, opts?: {timeoutMs?: number}) => Promise<any>}
 */
export function createApi(fetchImpl, timers) {
  const setTimer = (timers && timers.setTimeout) || ((fn, ms) => setTimeout(fn, ms));
  const clearTimer = (timers && timers.clearTimeout) || ((t) => clearTimeout(t));

  /** 一次调用；`timeoutMs` > 0 时到点中止并抛 `timeout: true`（默认不设上限：建会话、补编、上传都可能是长动作）。 */
  return async function api(method, url, body, opts) {
    const ms = (opts && opts.timeoutMs > 0) ? opts.timeoutMs : DEFAULT_TIMEOUT_MS;
    const ctrl = (ms > 0 && typeof AbortController === "function") ? new AbortController() : null;
    const work = (async () => {
      const res = await fetchImpl(url, {
        method,
        headers: body ? { 'Content-Type': 'application/json' } : undefined,
        body: body ? JSON.stringify(body) : undefined,
        signal: ctrl ? ctrl.signal : undefined,
      });
      const data = await res.json().catch(() => ({}));
      if (!res.ok) {
        throw Object.assign(new Error(data.error || ('HTTP ' + res.status)), {
          status: res.status,
          data,
        });
      }
      return data;
    })();
    if (!ms) return work;
    let timer = null;
    const expired = new Promise((_, reject) => {
      timer = setTimer(() => {
        if (ctrl) { try { ctrl.abort(); } catch { /* 已经结束 */ } }
        reject(Object.assign(new Error('请求超时（' + ms + ' 毫秒）：' + method + ' ' + url), {
          timeout: true,
          status: 0,
          data: {},
        }));
      }, ms);
    });
    // 超时之后这次调用仍在飞：接住它的结果，免得变成未处理的 rejection。
    work.catch(() => {});
    try {
      return await Promise.race([work, expired]);
    } finally {
      if (timer !== null) clearTimer(timer);
    }
  };
}
