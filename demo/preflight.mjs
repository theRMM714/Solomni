// 演示是**真机测试**：条件不齐就明说并拒绝跑，绝不用内置演示通道演一遍。
//
// 条件全部取自产品自己的能力面（`GET /api/state`），不猜：
//   ① 三个模块都在清单里：harvest / render / indexer；indexer 是 C++，要先编译出 build/ 里的可执行文件；
//   ② 登记处里至少有一个供应商（端点 + 密钥）与一个模型；
//   ③ 本次要用的模型定得下来：`SOLOMNI_DEMO_MODEL` 指定的在册，或有一个在册的核心默认模型。
// 三条全过 = 返回 null；否则返回 { reason, hints }，调用方打印后以退出码 2 收场（条件不足 ≠ 演示失败）。
//
// MCP 演示（`demo/run-demo-mcp.mjs`）的前置是 `mcpPreflight`：① mcp-toolbox 模块在清单里；② 同上面第 ②③ 条。
import { existsSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const NEEDED_MODULES = ["harvest", "render", "indexer"];
const MCP_MODULE = "mcp-toolbox";
const VM_MODULE = "harvest";

/** indexer 的构建产物在不在（两个平台的名字都认）。 */
function indexerBuilt() {
  const build = join(HERE, "..", "modules", "indexer", "build");
  return existsSync(join(build, "indexer.exe")) || existsSync(join(build, "indexer"));
}

/** 登记处与本次要用的模型：缺供应商 / 缺模型 / 定不下模型都如实拒绝。 */
export function registryPreflight(state, model) {
  const providers = (state && state.providers) || [];
  const models = (state && state.models) || [];
  const core = (state && state.core) || "";
  if (!providers.length) {
    return {
      reason: "登记处里没有供应商",
      hints: ["界面「设置 → 供应商」加一个端点 + 密钥（密钥只写进 .home/providers.yaml，不回显）"],
    };
  }
  if (!models.length) {
    return {
      reason: "登记处里没有模型",
      hints: ["界面「设置 → 模型」加一个模型（选供应商 + 写实际模型串）"],
    };
  }
  const known = models.map((m) => m && m.id).filter(Boolean);
  if (model) {
    if (!known.includes(model)) {
      return {
        reason: "SOLOMNI_DEMO_MODEL 指定的模型不在登记处：" + model,
        hints: ["在册的模型：" + known.join("、")],
      };
    }
    return null;
  }
  if (!core || !known.includes(core)) {
    return {
      reason: "没有可用的核心默认模型",
      hints: [
        "在界面「设置」里给核心选一个默认模型，或用 SOLOMNI_DEMO_MODEL=<模型 id> 指定",
        "在册的模型：" + known.join("、"),
      ],
    };
  }
  return null;
}

export function demoPreflight(state, model) {
  const modules = (state && state.modules) || [];
  const missing = NEEDED_MODULES.filter((id) => !modules.some((m) => m && m.id === id));
  if (missing.length) {
    return {
      reason: "模块清单里缺 " + missing.join("、"),
      hints: [
        "模块放进 modules/ 就出现（清单是目录扫描的纯函数）",
        "indexer 是 C++：按 modules/indexer/README.md 的命令编译一次",
      ],
    };
  }
  if (!indexerBuilt()) {
    return {
      reason: "indexer 还没构建（找不到 modules/indexer/build/ 下的可执行文件）",
      hints: ["按 modules/indexer/README.md 的命令编译一次（产物不入库，要本机自己建）"],
    };
  }
  return registryPreflight(state, model);
}

/**
 * MCP 演示（`demo/run-demo-mcp.mjs`）的前置：mcp-toolbox 模块在清单里，登记处有可用的模型。
 * 模块带 `services` 声明不需要预先构建；真正拉起服务在演示里做（enable → start），失败会如实报。
 */
export function mcpPreflight(state, model) {
  const modules = (state && state.modules) || [];
  if (!modules.some((m) => m && m.id === MCP_MODULE)) {
    return {
      reason: "模块清单里缺 " + MCP_MODULE,
      hints: [
        "模块放进 modules/ 就出现（清单是目录扫描的纯函数）",
        "它随仓库提供：确认 modules/mcp-toolbox/module.yaml 在（清单会重扫）",
      ],
    };
  }
  return registryPreflight(state, model);
}

/**
 * 代理演示（`demo/run-demo-proxy.mjs`）的前置：与前两个演示不同，这里**不指定模块**——
 * 挑人是核心自己的事（核心可以临时组装零模块的 agent：只用内建文件工具）。
 *
 * 代理会话跑在**核心默认模型**上（代理工具的执行者是核心自己）：没有核心默认模型就会回落到内置
 * 演示通道——那条通道不会原生调工具，验不出代理能力，所以这里与其余前置一样**如实拒跑**。
 * 也因此本演示**不认 `SOLOMNI_DEMO_MODEL`**（那是给 agent 指定模型用的）。
 */
export function proxyPreflight(state) {
  const providers = (state && state.providers) || [];
  const models = (state && state.models) || [];
  const core = (state && state.core) || "";
  if (!providers.length) {
    return {
      reason: "登记处里没有供应商",
      hints: ["界面「设置 → 供应商」加一个端点 + 密钥（密钥只写进 .home/providers.yaml，不回显）"],
    };
  }
  if (!models.length) {
    return {
      reason: "登记处里没有模型",
      hints: ["界面「设置 → 模型」加一个模型（选供应商 + 写实际模型串）"],
    };
  }
  const known = models.map((m) => m && m.id).filter(Boolean);
  if (!core || !known.includes(core)) {
    return {
      reason: "没有可用的核心默认模型（核心代理跑在核心通道上）：" + (core || "（没设）"),
      hints: [
        "在界面「设置」里给核心选一个默认模型——代理会话与它临时挑出来的子会话都走这条通道",
        "在册的模型：" + known.join("、"),
      ],
    };
  }
  return null;
}

/**
 * 虚拟机档演示（`demo/run-demo-vm.mjs`）的前置：① harvest 模块在清单里（它跑 python）；
 * ② 基础根在场，且 guest 画像**声明自带 python**（`guest.yaml` 的 `runtimes`）——虚拟机档的静态预检
 * 就按"包库 + guest 自带"两处判缺不缺包，演示也必须先有这一条；③ 同上面的供应商 / 模型。
 *
 * 缺包时核心会在**创建那一刻**拒绝（不拉一台跑不了工具的 guest）；这里把最容易缺的几项提前说清。
 */
export function vmPreflight(state, model, base) {
  const modules = (state && state.modules) || [];
  if (!modules.some((m) => m && m.id === VM_MODULE)) {
    return {
      reason: "模块清单里缺 " + VM_MODULE,
      hints: ["模块放进 modules/ 就出现（清单是目录扫描的纯函数）"],
    };
  }
  const dir = base && base.trim();
  if (!dir) {
    return {
      reason: "没有给虚拟机档的基础根",
      hints: ["用 SOLOMNI_DEMO_VM_BASE=<基础根目录> 指定（默认 .vm/guest/base）"],
    };
  }
  if (!existsSync(dir)) {
    return {
      reason: "基础根不在场：" + dir,
      hints: ["先补齐依赖：solomni --provision --ensure --yes（QEMU + guest 镜像 + guest-runtime）"],
    };
  }
  const yaml = join(dir, "guest.yaml");
  const text = existsSync(yaml) ? readFileSync(yaml, "utf8") : "";
  if (!/runtimes:\s*\[[^\]]*\bpython\b/.test(text)) {
    return {
      reason: "guest 画像没有声明自带 python：" + yaml,
      hints: [
        "让运行期编排把装好的能力写进 guest.yaml 的 runtimes（solomni --provision --ensure=vm.guest-runtime.windows --yes）",
        "或自己往 runtimes/ 放一个 python 包（虚拟机档的选型诊断两处都认）",
      ],
    };
  }
  return registryPreflight(state, model);
}

/** 打印条件不足的原因与出路；退出码 2 = 条件不足（不是演示失败）。 */
export function refuseDemo(block) {
  console.error("DEMO-SKIPPED：" + block.reason);
  for (const h of block.hints) console.error("  · " + h);
  console.error("  （演示走真实模型：条件不齐就不跑，不用演示通道凑一遍。）");
  return 2;
}
