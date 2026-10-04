# Solomni

> **中文:** [README.md](README.md) —— the same overview in Chinese.

Solomni is an **agent runtime environment (AgentOS) that runs on your own machine**: one executable plus a
`modules/` folder is the whole product. No installer, no server, no account.

## What it is made of

| Thing | What it is |
| --- | --- |
| **Core** | The only program in the product (Rust). It provides the mechanisms: sessions and transcripts, task orchestration and acceptance, tool execution and fencing, the registry, rewind and compaction |
| **Module** | A folder `modules/<id>/`: a `module.yaml` (responsibility prompt + tool declarations) plus the tools themselves — any language, anything that runs |
| **Agent** | A name + the modules it holds + a model + its own sandbox. Whichever agent a module is loaded into, it speaks in that agent's name |
| **Users** | People and agents. People use the interfaces (terminal / local web UI), agents use the module contract — the same modules, two doors |

## What works today

- **Three forms** (chosen per piece of work):
  - **single agent**: one AI holding several module capabilities, working directly;
  - **group collaboration**: several agents each holding one capability — form-up → discussion → a task chain
    is drafted → **review gate** (nothing starts until you approve) → execution per stage with concurrency →
    one acceptance pass per stage → final acceptance → delivery; on failure only the named nodes go back;
  - **proxy**: hand the whole decision to the core, which picks people and creates child sessions; entered
    from the API / scripts today.
- **Modules are drop-in**: a folder with a valid `module.yaml` under `modules/` appears; remove it and it is
  gone — no registration, no build, no restart.
- **Sessions and history**: one directory per piece of work (`session/<name>/`); append-only transcripts,
  rewind to any line, context compaction; child sessions live under the parent's `children/` (nesting is
  allowed) and the whole tree shares **one** work area.
- **Three isolation layers**: path checks inside the built-in tools → every tool process runs behind the gate
  process (environment allowlist / process tree / timeout kills the whole tree) → platform fences
  (Windows AppContainer, Linux Landlock, macOS seatbelt). If a mechanism cannot be installed, the capability
  grade is reported honestly instead of pretending.
- **Registry**: four YAML files for providers / models / agents / settings, all under `.home/`, managed from
  the UI. Keys live only in the local registry and in the core's outbound calls (boundaries in
  [REGISTRY_SPEC.md](REGISTRY_SPEC.md)).
- **Two interfaces**: the terminal transcript center (default) and the local web UI (`-webUI`, binds
  `127.0.0.1` only, port 3081 by default).

## Install and run

Requirements: **Node** (to run the launcher) and **Rust** (to build the core).

```bash
node start.js            # any platform; on Windows also start.bat, on macOS / Linux also ./start.sh
node start.js -webUI     # go straight to the local web UI (127.0.0.1:3081)
```

- The first run keeps the toolchain **inside the project** (`platform/`, `.tools/`): if Rust is missing it asks
  for consent before installing, and never touches the system.
- **You can run it without a provider**: it walks the flow with the built-in fake model and says so plainly.
- Common flags: `--root <dir>` (product root), `--web-port <port>`, `--release`.
- At the terminal prompt: `single <agent>…` / `collab <request>` / `proxy` / `webui`.

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
      path: { type: string, required: true, desc: real absolute path to read }
```

- **One calling convention**: a start command + arguments + a stdin/stdout receipt. Scripts, skills, MCP servers
  and other harnesses are all instances of it.
- A tool process can reach: this work's shared area + that agent's private sandbox + its own module directory;
  it accepts **real absolute paths** only, and anything outside is refused.
- If it runs, it is a valid module; if it does not, the core reports the reason plainly — no guessing, no fallback.

### Run the demo

The demo is a **real-machine test**: it runs against a real model — it feeds real files, calls real tools and
checks real artifacts. So it verifies the preconditions first. The first two scripts need the three modules in
the roster, the indexer built, and a provider plus a usable model (`SOLOMNI_DEMO_MODEL`, or the core default);
the **proxy** script needs a provider, a model and a **core default model**, plus at least one module or a
stored agent.
**When a precondition is missing it prints `DEMO-SKIPPED` with how to fix it and exits with code 2** — it never
runs the flow on the built-in demo channel just to look successful.

```bash
node start.js -webUI
node demo/run-demo.mjs         # composite: one agent holding three modules (python / node / C++)
node demo/run-demo-collab.mjs  # collaboration: three agents with separate powers, end to end
node demo/run-demo-proxy.mjs   # proxy: talk to the core only; it picks people, one stop stops the tree
```

The C++ module has to be compiled once (the artifact is not committed); the command and the reason are in
[modules/indexer/README.md](modules/indexer/README.md) — the preflight check confirms it for you.

## Not wired up yet

- **The guest body for the VM tier**: selection, diagnostics, the mount plan and the per-item pre-checks are in
  place, but the guest itself is not wired in, so the **VM tier cannot be selected at all** right now (the UI
  lists what is missing and how to fix each item).
- Credential custody for external tool services (module tools must not depend on credentials today), module
  distribution and a marketplace, and wider concurrent dispatch.
- Every unfinished item, how to fix it and its acceptance criteria have exactly one authority:
  [tests/gaps.yaml](tests/gaps.yaml).

## What it is not

- Not a model or a provider: channels and keys are product resources, and models are replaceable.
- Not a runtime that schedules resident processes: modules never sit around waiting; they work once per task.
- Not a server: it binds `127.0.0.1` only — no server side, no account.

**Quality**: `node run-tests.js` is the single entry point — T0 (format / compile / clippy / duplicate deps /
structural review, zero tolerance) + unit + cross-platform integration + frontend smoke + end-to-end; the
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
