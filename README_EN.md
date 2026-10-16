# Solomni

> **中文:** [README.md](README.md) —— the same overview in Chinese.

Solomni is an **agent runtime environment (AgentOS) that runs on your own machine**: one executable plus a
`modules/` folder is the whole product. No installer, no server, no account.

## Why you need it

**① You have a tool or a workflow, and you want AI to use it.**
Wrapping it as a plugin for one specific product ties it to that product: switch agents or platforms and you wrap it
again, while the tool itself grows a shell coupled to the host. Here you **adapt it to the contract once**: a folder
plus a `module.yaml` (responsibility prompt + tool declarations), with the command it already had. The change stays on
the module's own side (for example, putting an executable inside the module folder, or agreeing on how arguments are
passed and how the receipt looks) — no restructuring of this repository, no SDK. The result is that **outside the
contract the two sides depend on nothing from each other**: take the platform away and the tool still runs as it did;
take the tool away and the platform keeps running. The same capability is used by people through the UI and by agents
through the contract, from a single declaration — no two entry points to maintain.

**② Several agents working on one job.**
A shared workspace has concrete problems by default: they overwrite each other, ownership of artifacts is unclear, and
someone builds on another's unfinished output as if it were fact. Here every agent has **its own sandbox**
(`session/<name>/<agent-instance>/`), which is also its **working copy** of the shared area; the shared area `work/`
is the **main copy and read-only for agents**. Pull what you need with `work_pull`, edit in the sandbox, then commit
with `work_commit`. Commits use a file-level three-way comparison, so a conflict rejects the whole commit and names
each path instead of silently overwriting; any commit point can be restored. The only two roads across agents are that
shared area and the transcript that enters the context: artifacts land in their owner's cell, so rework can find the
person, and a module's directory belongs only to the agent that holds it. Finer-grained "who may write which part" is
enforced by session permissions: per-agent read and commit allow-/block-lists, with module directories read-only by
default (see [docs/permission/README.md](docs/permission/README.md)).

**③ When something goes wrong, you need to know what it saw and why it decided that.**
The transcript is **append-only**; delete and restore are explicit actions that really rewrite the log. On any line,
"rewind to here" offers: **archive** — append a marker and collapse the earlier lines (all bytes kept, recoverable);
**delete** — really truncate to that point; **restore** — truncate at the marker, discarding work done since. Line ids
are monotonic and never reused, and replay is reproducible, so **a rebuild matches the live run line by line**. The
shared area comes back to that moment too, via the `(agent, line)` anchor on each commit, with the whole subtree
synchronized. A delivery can be replayed from disk — what the AI did is an auditable, reproducible engineering
artifact, not a chat that is gone.

## What it is made of

| Thing | What it is |
| --- | --- |
| **Core** | The only program in the product (Rust). It provides the mechanisms: sessions and transcripts, task orchestration and acceptance, tool execution and fencing, the registry, rewind and compaction |
| **Module** | A folder `modules/<id>/`: a `module.yaml` (responsibility prompt + tool declarations, optionally resident services and secret fields) plus the tools themselves — any language, anything that runs |
| **Agent** | A name + the modules it holds + a model + its own sandbox. Whichever agent a module is loaded into, it speaks in that agent's name |
| **Users** | People and agents. People use the interfaces (terminal / local web UI), agents use the module contract — the same modules, two doors |

## What works today

- **Three forms** (chosen when you create a piece of work):
  - **single agent**: one AI holding several module capabilities, working directly;
  - **group collaboration**: several agents each holding one capability — form-up → discussion → a task chain
    is drafted → **review gate** (nothing starts until you approve) → execution per stage with concurrency →
    one acceptance pass per stage → final acceptance → delivery; on failure only the named nodes go back;
  - **proxy**: hand the whole decision to the core, which picks people, creates child sessions and answers the gates for you.
- **Modules are drop-in**: a folder with a valid `module.yaml` under `modules/` appears; remove it and it is
  gone — no registration, no build, no restart.
- **Sessions and history**: one directory per piece of work (`session/<name>/`); "rewind to here" on any line —
  **archive** (recoverable) / **delete** (real truncation) / **restore** — and the shared area comes back with it,
  anchored by commit line; transcripts are replayable, context is compactable; child sessions live under the
  parent's `children/` (nesting is allowed) and the whole tree shares **one** work area (versioned: `work_pull` / `work_commit`).
- **Three isolation layers**: path checks inside the built-in tools → every tool process runs behind the gate
  process (environment allowlist / process tree / timeout kills the whole tree) → platform fences
  (Windows AppContainer, Linux Landlock, macOS seatbelt). If a mechanism cannot be installed, the capability
  grade is reported honestly instead of pretending. Fence grant targets are split into **required** and
  **optional**: an optional target that cannot be granted is only recorded, while a **required** one is never
  degraded silently — it raises a card offering "run this once unfenced / drop the call", and with no front end
  able to answer, the call is refused (the command is never started).
- **Two execution tiers**: the **host tier** (default — scripts run directly on the host) and the **vm tier**
  (module tools run inside a guest, isolated from host processes). The vm tier is pluggable along
  **host × engine × guest × channel × share**: Linux (QEMU/KVM + virtiofs, falling back to 9p when virtiofsd is absent) and Windows (QEMU/WHPX + SMB over guestfwd, falling back to QGA copy) are already
  verified on real machines, macOS is still missing (see [tests/gaps.yaml](tests/gaps.yaml)). `node start.js --provision --ensure --yes`
  installs QEMU and a Debian guest into the project's `.vm/` (`--yes` confirms the download); host paths and guest paths are converted
  by **one bridge**, in both directions (identity when both sides share a dialect, a mirror root otherwise), and the working-environment
  prompt states **one spelling per environment** — system tools translate a guest spelling back to a host path before checking it. Details in [docs/vm/README.md](docs/vm/README.md); runtime package
  contract in [RUNTIME_SPEC.md](RUNTIME_SPEC.md).
- **Registry**: four YAML files for providers / models / agents / settings, all under `.home/`, managed from
  the UI. Keys live only in the local registry and in the core's outbound calls (boundaries in
  [REGISTRY_SPEC.md](REGISTRY_SPEC.md)).
- **Two interfaces**: the terminal transcript center (default) and the local web UI (`-webUI`, binds
  `127.0.0.1` only, port 3081 by default).
- **One decision channel**: everything that needs your call — the core's gates (roster / start / plan review /
  failed node), tool-level confirmation, and questions raised by the tool layer itself — uses the same card:
  **message + options** (the option id is the contract, wording belongs to the UI), answered by one command.
  It **blocks**: nothing moves until you answer; pressing stop voids the whole queue and releases waiters as
  "denied". Cards and answers are persisted with the session (already answered ones are never re-asked); when no
  option can actually be executed, no card is shown — the session is stopped and a warning is recorded.

## Install and run

Requirements: **Node** (to run the launcher) and **Rust** (to build the core). The environment has two layers:
the **runtime environment** (just running the product) and the **dev environment** (changing code / running gates).

### Runtime environment

```bash
node setup-runtime.js    # prepare the runtime environment: platform/<os>/ and .tools/ inside the project; borrows a usable Rust read-only
node start.js            # any platform; on Windows also start.bat, on macOS / Linux also ./start.sh
node start.js -webUI     # go straight to the local web UI (127.0.0.1:3081)
node start.js --provision                # optional: read-only vm tier status (JSON: QEMU / guest image / agent and runtimes)
node start.js --provision --ensure --yes # optional: install QEMU and a Debian guest into the project's .vm/ (--yes confirms the download)
```

- Toolchain and caches stay **inside the project**: a usable Rust is **borrowed read-only** (caches and artifacts
  still land under `platform/<os>/`); only when none is usable does it ask for consent and install into the
  project, never touching the system; with **no interactive terminal** it skips the install, prints the manual
  steps, and exits. Borrowing means the system `~/.cargo/config.toml` (mirrors / credentials) is not read and
  deps are re-downloaded into the project — the price of "no writes outside"; the gate also snapshots
  out-of-project cache roots and fails if anything was written.

### Dev environment

```bash
node setup-dev.js        # install the dev environment: dev uses the **project-local** toolchain, artifacts land only under platform/<os>/ and .tools/
node setup-dev.js --check  # read-only check: lists what is missing, exits non-zero if anything is
node run-tests.js        # quality gate + full test run (see TESTING.md)
node run-tests.js --skip-e2e  # gate without L4 (the quality-job mode used while iterating)
node dev.js test|fmt|clippy   # one-layer dev commands; do not call cargo directly (see docs/testing/execution-ci.md)
```

- The **item inventory** of the dev environment is [dev-tools.js](dev-tools.js) (the single authority): `setup-dev.js`
  installs per it, `run-tests.js` checks / `env-skip`s per it. It covers Rust components
  (rustfmt / clippy / llvm-tools-preview), cross targets (Windows + macOS) and project-local tools
  (cargo-audit / cargo-deny / cargo-llvm-cov).
- Why it is separate from the runtime environment: people who only run the product do not need any of it, and the
  gate still runs on a runtime-only machine — missing items are reported as `env-skip`, never silently.
- The environment is its own layer ([env.js](env.js)): path conventions, toolchain detection, environment
  composition and installation all live there; `node env.js` shows the resolved environment and
  `node env.js --print-env` prints it machine-readable. `setup-runtime.js` / `setup-dev.js` only orchestrate;
  `node setup-runtime.js` and `node env.js setup` are equivalent.
- **Outbound-link ledger**: `links.yaml` is the single authority for external URLs the project uses and that may
  change (currently: toolchain bootstrap sources, the pinned winlibs upstream asset, the self-hosted dependencies release prefix, guest image sources, and the SMB helper asset); edit it once and the whole
  repository follows. User overrides go through `.home/settings.yaml` and environment variables, not the ledger.
- **You can run it without a provider**: it walks the flow with the built-in fake model and says so plainly.
- Common flags: `--root <dir>` (product root), `--web-port <port>`, `--release`.
- Commands at the prompt: `single [agent…]` / `collab [agent…|?]` / `proxy` (hand the whole decision to the core) / `module [module-id.tool [json]]` (run a module tool directly, without AI) / `resident list|start|stop|enable|disable|call` (resident services) / `secret` (secret fields) / `webui`.

### Add a provider and a model

In the UI, add a provider (endpoint + key) and models, and pick the core default model. Keys are written only
to `.home/providers.yaml` and are never echoed back.

## Module contract

A module is a folder plus a `module.yaml`, and it **depends on nothing in this project**; the core does not
depend on the module's implementation either — both sides depend only on this contract (full fields and rules
in [MODULE_SPEC.md](MODULE_SPEC.md)):

```yaml
id: research                 # globally unique, = the folder name
brief: Research and comparison.   # short capability note: what the core picks people from
system: You take care of research...  # responsibility prompt: scope + how your tools are used
runtimes: [python]           # optional: runtime capabilities the tools need (versions are chosen per session)
tools:                       # optional: external tool table
  read_txt:
    command: python tools/read_txt.py   # working directory = the module root
    params:
      path: { type: string, required: true, desc: path written the way this environment spells it }
```

- **One calling convention for external tools**: a start command + arguments + a stdin/stdout receipt. Scripts,
  skills and other harnesses are all instances of it; a long-running MCP server is not — declare it under `services`.
- A tool process can reach: that agent's private sandbox + its own module directory; the shared main copy is not
  among them (pull before editing, commit when done); it accepts paths written **the way this environment spells them**
  (host tier = host paths, vm tier = guest paths), and anything outside the allowed roots is refused.
- **Every module keeps a cross-task private area** `<module>/userdata/`: it is ensured to exist when the
  module is loaded (directory only, idempotent; failure never blocks loading, it is just reported — that seat then
  has no private landing). Module directories are read-only by default, `userdata/` excepted.
- If it runs, it is a valid module; if it does not, the core reports the reason plainly — no guessing, no fallback.

### Run the demo

The demo is a **real-machine test**: it runs against a real model — it feeds real files, calls real tools and
checks real artifacts. So it verifies the preconditions first. The first two scripts need the three modules in
the roster, the indexer built, and a provider plus a usable model (`SOLOMNI_DEMO_MODEL`, or the core default);
the **MCP** script needs `mcp-toolbox` in the roster plus a provider and a usable model (it declares its
`services` itself, so nothing has to be built); the **proxy** script needs a provider, a model and a **core
default model** (it picks the roster itself and may assemble a zero-module agent); the **VM** script needs `harvest` in the roster, a
usable model, and a guest base that declares the runtime it ships (`runtimes` in `guest.yaml`) — a missing
runtime is refused at creation time.
**When a precondition is missing it prints `DEMO-SKIPPED` with how to fix it and exits with code 2** — it never
runs the flow on the built-in demo channel just to look successful.

```bash
node start.js -webUI
node demo/run-demo.mjs         # composite: one agent holding three modules (python / node / C++)
node demo/run-demo-collab.mjs  # collaboration: three agents with separate powers, end to end
node demo/run-demo-mcp.mjs     # MCP: one agent holding only mcp-toolbox, starts the resident service and calls it
node demo/run-demo-proxy.mjs   # proxy: talk to the core only; it picks people, one stop stops the tree
node demo/run-demo-vm.mjs      # VM tier: one agent running harvest inside the guest; artifacts land on the host live
```

The C++ module has to be compiled once (the artifact is not committed); the command and the reason are in
[modules/indexer/README.md](modules/indexer/README.md) — the preflight check confirms it for you.

## Not wired up yet

Every unfinished item, how to fix it and its acceptance criteria have exactly one authority:
[tests/gaps.yaml](tests/gaps.yaml); the planned core-proxy system tools are in [systool_gaps.yaml](systool_gaps.yaml).

## What it is not

- Not a model or a provider: channels and keys are product resources, and models are replaceable.
- Not a runtime that keeps modules resident: `tools` start a one-shot process per task; the `services` a module declares are resident services started and reaped by the core under your authorization.
- Not a server: it binds `127.0.0.1` only — no server side, no account.

**Quality**: `node run-tests.js` is the single entry point — T0 (format / compile / clippy / duplicate deps /
structural review / cross-target type check / out-of-project writes / supply chain, zero tolerance; supply-chain
tools report `env-skip` when absent) + unit + cross-platform integration + frontend smoke + end-to-end; the
real-machine fence probes run on three CI platforms (see [TESTING.md](TESTING.md)).

## Document guide

| I want to… | Read |
| --- | --- |
| See the overview and quick start | This file (Chinese: [README.md](README.md)) |
| See what this project is meant to become (direction and commitments) | [PHILOSOPHY.md](PHILOSOPHY.md) |
| Use the product / see user journeys and interfaces | [PRODUCT.md](PRODUCT.md) |
| Write a module (YAML contract and module tools) | [MODULE_SPEC.md](MODULE_SPEC.md) |
| System tools, roles and the path model (reports and acceptance too) | [SYSTOOL.md](SYSTOOL.md) |
| The planned core-proxy system tools | [systool_gaps.yaml](systool_gaps.yaml) (planned, not current capability) |
| Build a runtime package (`package.yaml`) | [RUNTIME_SPEC.md](RUNTIME_SPEC.md) |
| Manage providers / models / agents / settings (and key boundaries) | [REGISTRY_SPEC.md](REGISTRY_SPEC.md) |
| Change code (layers, ports, logging, prompts, on-disk contracts) | [ARCHITECTURE.md](ARCHITECTURE.md) |
| Write tests and acceptance (levels, doubles, gates, gaps, CI) | [TESTING.md](TESTING.md) |
| Repository rules and document routing | [AGENTS.md](AGENTS.md) |

> **Two layers**: the repository root holds **portals** (positioning + references), `docs/` holds the **details**
> — every fact has exactly one authority. Full routing is in [AGENTS.md](AGENTS.md) under "Document routing".

Apache License 2.0 · see [LICENSE](LICENSE)
