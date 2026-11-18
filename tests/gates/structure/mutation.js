/**
 * 结构审查·突变范围：每个单元在 `tests/specs/<unit>.yaml` 的 `mutation` 块里自己声明"哪些文件值得做突变、是否进周检"。
 *
 * 为什么要有它：旧实现是手写 `tests/mutation-scope.json` + 散文双写，同一次搬迁只改一半就静默漂了
 * （`catalog/registry`、`conversation/slate` 已经改名，清单还指着旧路径；cargo-mutants 对不存在的 `-f` 不报错，范围静默缩小）。
 * 这里只判声明自身是否自洽：路径存在、落在本单元目录、不写 glob、同一文件只归一个单元、有 case 兜底。
 * 突变跑不跑由调度决定（`schedule: weekly` 进定时审计，`none` 只留给手动抽查），门禁不判"够不够大"。
 */
"use strict";

const { validate } = require("../../lib/mutation.js");

function mutationGate(root) {
  return validate(root);
}

module.exports = { mutationGate };
