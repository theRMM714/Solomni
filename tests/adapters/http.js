//! 目的：把「经 HTTP 驱动真二进制」这一种派发形态收在一处。
//! 管：把选中的旅程 id 交给 e2e 编排器（它起夹具与真二进制），按 E2E-OK 判成败。
//! 不管：旅程里的断言（在 tests/cross-platform/e2e/driver.js）；夹具准备（在 orchestrator.js）。
//! 联动：tests/journeys/index.js 的 id；run-tests.js 的子集执行。

const path = require("path");
const markers = require("../lib/markers.js");

module.exports = {
  name: "http",
  title: "经 HTTP 驱动真二进制（e2e 旅程）",
  run(ids, ctx, serial, root) {
    // 段选择走环境变量：编排器只负责起夹具与真二进制，不关心选哪几段。
    process.env.E2E_JOURNEY = ids.join(",");
    const e2e = path.join(root, "tests", "cross-platform", "e2e", "orchestrator.js");
    const r = ctx.proc.sh(process.execPath, [e2e], require("../gates/steps.js").E2E_TIMEOUT_MS);
    const ok = r.code === 0 && r.out.includes(markers.TEXT.E2E_OK);
    return [{ ok, label: "旅程 " + ids.join(","), detail: ok ? "E2E-OK" : "日志 " + r.log, tail: r.out }];
  },
};
