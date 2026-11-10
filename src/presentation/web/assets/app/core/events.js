/* 目的：事件台长轮询的传输层（取批次 + 游标 + 连接状态）。
 * 管：按 `since` 取 `/api/events`；只把「取不到」报成断线（3 秒后重试）；把裁剪起点与批次交给钩子；
 *   钩子抛出的异常报给 `onError`（界面自己的问题，不是断线），轮询照常继续。
 * 不管：事件含义、应用游标 `appliedSeq`、会话状态与渲染——都在钩子另一侧。
 * 联动：契约见 `docs/presentation/contracts.md` 的「传输选择：长轮询」；`fetch` / `setTimeout` 由
 *   `app/bridge.js` 注入。 */

const RETRY_MS = 3000;

/**
 * @param {{
 *   fetch: (url: string) => Promise<{ok: boolean, status?: number, json: () => Promise<any>}>,
 *   setTimeout: (fn: () => void, ms: number) => any,
 *   onConn: (ok: boolean, err?: any) => void,
 *   onTrim: (oldest: number) => Promise<void> | void,
 *   onLines: (lines: Array<{seq: number, sid: string, events: any[]}>) => Promise<void> | void,
 *   onError: (err: any) => void,
 * }} deps
 */
export function createEventStream(deps) {
  let since = 0;
  let active = false;
  const wait = (ms) => new Promise((res) => deps.setTimeout(res, ms));

  async function run() {
    if (active) return;
    active = true;
    while (true) {
      let data;
      try {
        const r = await deps.fetch('/api/events?sid=&since=' + since);
        if (!r.ok) throw new Error('轮询失败 ' + r.status);
        data = await r.json();
      } catch (err) {
        deps.onConn(false, err);
        await wait(RETRY_MS);
        continue;
      }
      deps.onConn(true);
      try {
        const head = data.head != null ? data.head : since;
        const oldest = typeof data.oldest === 'number' ? data.oldest : 0;
        // 先对齐裁剪、再推进游标、最后交批次：顺序与契约的「顺序与去重」一致。
        await deps.onTrim(oldest);
        since = head;
        await deps.onLines(data.lines);
      } catch (err) {
        deps.onError(err);
      }
    }
  }

  return { run, cursor: () => since };
}
