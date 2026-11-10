/* 目的：HTTP 调用的唯一封装（JSON 进出 + 统一错误形状）。
 * 管：发请求、解析 JSON、非 2xx 抛出带 `status` / `data` 的 Error（文案取后端 `{error}`）。
 * 不管：路由含义、重试、长轮询（`app/core/events.js`）；不读任何界面状态。
 * 联动：`fetch` 由调用方注入（`app/main.js`），冒烟可换桩；路由目录见 `src/presentation/web/routes.rs`。 */

/**
 * @param {(url: string, init?: object) => Promise<{ok: boolean, status?: number, json: () => Promise<any>}>} fetchImpl
 * @returns {(method: string, url: string, body?: any) => Promise<any>}
 */
export function createApi(fetchImpl) {
  return async function api(method, url, body) {
    const res = await fetchImpl(url, {
      method,
      headers: body ? { 'Content-Type': 'application/json' } : undefined,
      body: body ? JSON.stringify(body) : undefined,
    });
    const data = await res.json().catch(() => ({}));
    if (!res.ok) {
      const err = new Error(data.error || ('HTTP ' + res.status));
      err.status = res.status;
      err.data = data;
      throw err;
    }
    return data;
  };
}
