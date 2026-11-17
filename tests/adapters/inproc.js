//! 目的：把「同进程里跑 cargo 测试模块」这一种派发形态收在一处。
//! 管：按测试路径前缀跑 `cargo test --bin solomni <prefix>`，并把「0 个用例命中」判成失败。
//! 不管：cargo 引擎自身的构建与静态检查（在 tests/gates/steps.js）；选哪些模块（在 tests/lib/plan.js）。
//! 联动：tests/lib/plan.js 的 cargoFilters；run-tests.js 的子集执行。

const parsers = require("../lib/parsers.js");

module.exports = {
  name: "inproc",
  title: "进程内（cargo 测试模块）",
  /** 跑一组测试路径前缀；每次调用失败只影响自己那一条。 */
  run(targets, ctx, serial) {
    return targets.map((f) => {
      const r = ctx.proc.sh(
        "cargo",
        ["test", "--color", "never", "--bin", "solomni", f, "--", "--nocapture"].concat(serial)
      );
      const counts = parsers.cargoCounts(r.out);
      // 过滤器写错时 cargo 也退 0（只是 0 个用例）——那种绿必须判红，否则子集运行会假过。
      const ok = r.code === 0 && counts.passed > 0;
      const detail = ok ? counts.passed + " passed" : r.code === 0 ? "0 个用例命中——过滤器或模块路径写错了" : "日志 " + r.log;
      return { ok, label: f, detail, tail: r.out };
    });
  },
};
