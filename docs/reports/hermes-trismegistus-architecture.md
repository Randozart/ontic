# Hermes Trismegistus — Containerized Dual-Slot Inference Architecture

> **Date:** 2026-08-25
> **Status:** Architecture specification, container-ready. Reference
> implementation runs bare-metal today (VITRIOL @ `a6c712b`, ontic forge slot
> pin in `src/forge.rs`); the container layout below is the packaging target.
>
> **Name:** the thrice-greatest stack — **Hermes** (agent orchestrator),
> **VITRIOL** (inference engine), **Ontic** (stochastic spec compiler) — one
> model, one server, two isolated KV slots.

## 1. What this is

A single-container deployment where one llama-server instance serves two
clients with *structurally isolated* context budgets:

```
┌─────────────────────────── container ────────────────────────────┐
│                                                                  │
│  hermes-agent gateway ─────► :8279/v1/chat/completions           │
│  (orchestrator, tool calls)      │ slot 0 · ctx 90112            │
│      │                           │                               │
│      │ spawns `ontic solve`      ▼                               │
│      │  ┌──────────────────────────────────────────┐             │
│      └─►│ VITRIOL llama-server (:8279)             │             │
│         │   unified KV pool, c=98304               │             │
│         │   slot 0 = 90112  ← hermes               │             │
│         │   slot 1 =  8192  ← ontic (pinned)       │             │
│         └────────────▲─────────────────────────────-┘            │
│                      │                                           │
│  ontic solve ────────┘  POST :8279/completion                    │
│  (subprocess, sequential)    {"id_slot": 1, "grammar": …}        │
│                                                                  │
└──────────────────────────────────────────────────────────────────┘
              │  GPU passthrough (nvidia-container-toolkit)
              ▼
        CUDA devices (sm_61 / sm_86 class or newer)
```

**Concurrency contract:** hermes invokes `ontic solve` as a terminal command
and blocks until it exits — the clients are *sequential*, never simultaneous.
Isolation therefore protects against **state pollution**, not contention:
ontic's KV cells can never exceed 8192 tokens and are rejected at admission
(`HTTP 400 exceed_context_size_error`) if a sample outgrows the slot.

## 2. Slot isolation contract

| property | value | enforced by |
|---|---|---|
| total KV budget | 98304 tokens | `-c 98304 --kv-unified` |
| hermes slot cap | 90112 | `--slot-context "0=90112,1=8192"` |
| ontic slot cap | 8192 | same flag; hard admission check |
| ontic routing | slot 1 only | `"id_slot": 1` pinned in `forge.rs` (`ONTIC_SLOT`) |
| oversize behavior | reject, never truncate into slot 0 | upstream admission logic |

The client-side pin is mandatory: without it the scheduler may route an idle
ontic request to slot 0 and silently consume orchestrator budget. The pin is
one constant; the cap is the second fence.

## 3. Memory-adaptive quant selection

The container must pick **any Qwen3.8-27B GGUF quant based on what memory
allows** — no hardcoded per-model constants. VITRIOL's calibrator
(`libvitriol`, Rust) already does exactly this from first principles:

```
VRAM = base_model + pin·per_layer_expert + ctx·kv_per_token + scratch + overhead
```

- all terms computed from GGUF tensor data (`ffn_*_exps` analysis,
  embedding dims) and a hardware probe (device count, VRAM, arch);
- overhead heuristic keyed on GPU generation (Pascal 1800 MiB … Ada 3200);
- KV per-token derived from `(embd/head_count)·head_count_kv·2.5/1M`.

Entrypoint algorithm:

1. probe GPUs → combined VRAM budget `B`;
2. reserve fixed costs: KV(c=98304, q4_0) ≈ 2.4 GiB + scratch + arch overhead;
3. choose the largest quant whose `base_model ≤ B − reserved`
   (ladder: UD-Q4_K_M > Q3_K_M > UD-IQ3_S > UD-IQ2_S …);
4. if nothing fits with dual slots, fall back to single-slot master profile
   (drop `--slot-context`, keep c=98304) before degrading quant;
5. emit profile, launch.

**Certified reference point** (bare metal, 3060 12 GiB + 1070 Ti 8 GiB):
UD-IQ3_S, q4_0 KV, ts 24,12, ub64 → **92,642 filled tokens**, decode
7.8–12.4 t/s. Window ≠ depth: allocated window is not usable depth;
the calibrator reserves headroom accordingly (~23 KiB/token prefill creep
observed on Pascal).

## 4. Component inventory

| component | source | artifact in image |
|---|---|---|
| llama-server (dual-slot port) | VITRIOL fork, submodule `20129c2cc` | `/opt/vitriol/bin/llama-server` |
| launcher + profiles | `scripts/vitriol`, `profiles/` | `/opt/vitriol/` |
| calibrator | `libvitriol` (cargo --release) | `/opt/vitriol/bin/libvitriol` |
| hermes-agent gateway | pip env + systemd unit (converted to supervisord/s6 in-container) | `/opt/hermes/venv` |
| ontic | `cargo build --release` | `/usr/local/bin/ontic` |
| models | volume mount (never baked in) | `/models/*.gguf` |
| ontic vault/corpus | volume mount | `/data/.ontic/` |

## 5. Container layout (reference)

```dockerfile
FROM nvidia/cuda:12.4.1-devel-ubuntu22.04 AS build
# NOTE: base CUDA must retain sm_61 (Pascal) support; bump carefully.
RUN apt-get update && apt-get install -y cmake g++ git python3 python3-venv \
    cargo rustc curl && rm -rf /var/lib/apt/lists/*

COPY vitriol/llama.cpp /src/llama.cpp
RUN cmake -B /src/llama.cpp/build -S /src/llama.cpp \
      -DCMAKE_CUDA_ARCHITECTURES="61;86" -DGGML_CUDA=ON \
 && cmake --build /src/llama.cpp/build --target llama-server -j"$(nproc)"

COPY ontic /src/ontic
RUN cargo build --release --manifest-path /src/ontic/Cargo.toml

COPY vitriol /opt/vitriol
RUN cd /opt/vitriol/libvitriol && cargo build --release

FROM nvidia/cuda:12.4.1-runtime-ubuntu22.04
RUN apt-get update && apt-get install -y python3 python3-venv curl \
    && rm -rf /var/lib/apt/lists/*
COPY --from=build /src/llama.cpp/build/bin/llama-server /opt/vitriol/bin/
COPY --from=build /opt/vitriol /opt/vitriol
COPY --from=build /src/ontic/target/release/ontic /usr/local/bin/

# hermes-agent venv (pinned requirements)
COPY hermes-requirements.txt /tmp/
RUN python3 -m venv /opt/hermes/venv \
 && /opt/hermes/venv/bin/pip install -r /tmp/hermes-requirements.txt

ENV ONTIC_FORGE=127.0.0.1:8279 \
    VITRIOL_PROFILE_MODE=auto
EXPOSE 8279
COPY entrypoint.sh /entrypoint.sh
ENTRYPOINT ["/entrypoint.sh"]
```

### entrypoint sequence

```bash
#!/usr/bin/env bash
set -euo pipefail
export VITRIOL_KV_SCORE=probe VITRIOL_POOL_RESET=1          # certified exports

QUANT=$(/opt/vitriol/scripts/calibrate-pick.sh "$GPU_BUDGET") # §3 algorithm
PROFILE=$([[ -n "${DUAL_SLOT:-}" ]] && echo qwen38-ontic || echo qwen38-master)

/opt/vitriol/scripts/vitriol config load "$QUANT-$PROFILE"
/opt/vitriol/scripts/vitriol serve --detach                  # writes fingerprint to log

until curl -sf http://127.0.0.1:8279/health | grep -q ok; do sleep 2; done

exec /opt/hermes/venv/bin/python -m hermes_cli.main gateway foreground
```

### compose sketch

```yaml
services:
  trismegistus:
    build: .
    gpus: all
    volumes:
      - ./models:/models:ro
      - ./vault:/data/.ontic
      - ./hermes:/root/.hermes
    environment:
      - NVIDIA_VISIBLE_DEVICES=all
      - GEMINI_API_KEY_FILE=/run/secrets/gemini_key   # secrets, not ENV layers
    secrets: [gemini_key]
    ports: ["8279:8279"]   # optional: expose hermes to LAN
```

## 6. Configuration surface

| knob | default | notes |
|---|---|---|
| `ONTIC_FORGE` | `127.0.0.1:8279` | always loopback inside the container |
| `ONTIC_SLOT` | 1 (compile-time const) | keep in lockstep with `--slot-context` |
| `ONTIC_FORGE_WORKERS` | 2 | both hit slot 1; sequential admission |
| `VITRIOL_KV_SCORE` / `VITRIOL_POOL_RESET` | `probe` / `1` | certified exports, set by entrypoint |
| `--slot-context` | `0=90112,1=8192` | omit ⇒ single-slot fallback mode |
| checkpointing | `--checkpoint-every-n-tokens 8192`, 4 slots' worth | never pass zeros (`ctx-checkpoints 0` corrupts heap; `cache-ram 0` breaks readiness) |
| MTP | off | no measurable benefit on this hardware class |

## 7. Security notes

- **Secrets**: API keys enter via Docker secrets or injected env at runtime —
  never as build args or layer content. The development repo currently carries
  a plaintext key in `ontic/.env`; rotate it and move it to the secret store
  before any image push. Do not copy `.env` into images.
- **Network**: llama-server binds 127.0.0.1 inside the container; only the
  hermes gateway port is published. Ontic talks over loopback exclusively.
- **Provenance**: VITRIOL fork is GPL-2.0; upstream PR #23340 is MIT
  (attribution retained in-tree). CUDA runtime images subject to NVIDIA EULA.

## 8. Verification checklist (all passed on bare-metal reference)

1. `n_slots = 2`; per-slot logs `90112` / `8192`.
2. Default chat request → slot 0.
3. Pinned grammar completion → slot 1, valid candidate produced.
4. 9001-token prompt pinned to slot 1 → `400 exceed_context_size_error
   (n_ctx:8192)` — isolation holds under overload.
5. Fingerprint line present in server log (`slots=` field when dual).
6. OOM postmortem regression: c98304 survives sustained agent load where
   c131072 was killed by oom-killer (anon-rss 8.4 GiB).

## 9. Open items

- Image build CI: sm_61 toolchain availability must be asserted per CUDA release.
- `--no-cache-idle-slots` contingency (upstream #27148 contamination reports)
  if cross-conversation bleed ever observed.
- Optional second container variant: dedicated ontic-only micro-server
  (tiny ctx, small quant) for hosts with >24 GiB combined VRAM.
