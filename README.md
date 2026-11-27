# Scarlett-Victory

A work-in-progress fork of [VIctory](https://github.com/Marvisak/VIctory), a
compatibility layer that runs an Xbox Series title natively on Windows from
files the user supplies. This fork works on the graphics side: shader
translation, ray tracing, GPU compatibility on AMD cards and rendering
correctness. Maintained by StrixEvo; developed with
[Claude Code](https://claude.com/claude-code).

**This repository contains no game files, code or assets.** You need your own
copy of the game. It is not affiliated with or endorsed by the game's
developer or publisher.

Fork chain: [VIctory](https://github.com/Marvisak/VIctory) (Marvisak) →
[victory-fork](https://github.com/fxceless/victory-fork) (fxceless) →
[victory-amd](https://github.com/myokex/victory-amd) (myokex, merged in `d92d2af`) → Scarlett-Victory (this
repository, branch `scarlett/shader-raytracing`).

## Test machine

Every result here comes from one PC:

| Part | |
|------|---|
| OS | Windows 11 Pro, build 26300 |
| CPU | AMD Ryzen 5 5600X (6 cores) |
| GPU | AMD Radeon RX 6700 XT, 12 GB (driver 32.0.21030.2001) |
| RAM | 32 GB |
| Console profile | Xbox Series X, with the Series S render path (`VICTORY_RENDER_S=1`, below) |

## Status (2026-11-27)

**Not playable yet**, but the opening plays through to the prison at up to
about 6 fps.

| Stage | State on the test machine |
|-------|---------------------------|
| Boot, menus, start screen | works, 30 fps |
| Opening cutscenes | play; characters, boat, water and sky are drawn |
| The prison (first walk-around) | **3.8 to 5.7 fps** with the Series S render path and the AMD FSR DLL (run38); 3.2 to 3.6 fps at 4K (run32) |
| Indoor light | the ray-traced indirect light does not run (`VICTORY_BVH_BUILDER=1` crashes the title: it reads structure sizes before they land). A stand-in from Belobug's fork, `VICTORY_AMBIENT=2`, lights rooms close to the console's look at no cost in frame rate |
| The boat | the cause of the sinking is found (see Findings); the fix (`VICTORY_TEXTURE_BACK=1`) is in, not yet checked in game |
| Async compute | `VICTORY_ASYNC_COMPUTE=1` runs the title's compute queue on the card's compute engine: 5.3 to 5.6 fps against 4.5 in the prison cell (185 against 220 ms a frame), debug layer clean |
| Faces, some foliage and bridges | sometimes without their texture: the 12 GB card runs out of texture room |
| Stability | runs of 5 to 30 minutes end normally; no device loss in the latest runs |

### Recommended run

```powershell
$env:VICTORY_TEXTURE_BACK = '1'      # water maps back to the CPU (boat)
$env:VICTORY_HEAP_PASS_MARKED = '1'  # heap pass looks only at changed slots
$env:VICTORY_RENDER_S = '1'          # Series S render path: 3D at 1080p
$env:VICTORY_FSR_DLL = '<path>\amd_fidelityfx_upscaler_dx12.dll'
$env:VICTORY_AMBIENT = '2'          # stand-in indirect light: rooms lit
$env:VICTORY_QUEUE2 = '3'           # copy-back and uploads on the copy engine
$env:VICTORY_ASYNC_COMPUTE = '1'    # the title's compute queue on the card's compute engine
target\release\victory.exe run <game dir>\<the title's exe> --execute
```

`amd_fidelityfx_upscaler_dx12.dll` is AMD's signed PC upscaler, from
`Kits/FidelityFX/signedbin` of the
[FidelityFX SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)
(v2.4.0 was used); it is not in this repository. `VICTORY_RENDER_S` needs it:
the title's own Xbox upscaler has 10 compute shaders that do not translate, so
without the DLL the 3D picture is black. The full `--console series-s` profile
stays black at the start screen and is not needed.

## What this branch changed

| Commit | Change |
|--------|--------|
| `29575cd` | **Async compute** (`VICTORY_ASYNC_COMPUTE=1`): the title's compute submissions run on a host D3D12 COMPUTE queue, synchronised with fences on the card. About 20 % more frames; the submitting thread waits for the card 22 ms a frame instead of 67. `=2` (no wait before direct lists) gains nothing. |
| `036b70b` | **Stand-in indirect light** (`VICTORY_AMBIENT`, from Belobug's AMD fork): rooms the sun does not reach are lit from an estimate of the frame's own light. |
| `62ab248` | **Series S render path on a Series X profile** (`VICTORY_RENDER_S=1`, from Neptune): the title reads a Series S's device type and renders a 1920x1080 game plane under a 3840x2160 UI plane; present now draws the second plane over the first. 3.8 to 5.7 fps against 3.2 to 3.6 at 4K. |
| `e726e53` | **The Series S upscaler through a PC FidelityFX DLL** (`VICTORY_FSR_DLL`, from Neptune): the title's FidelityFX calls go to AMD's PC DLL with the host device, list and textures. |
| `65b8002` | Upload buffers in the card's memory through ReBAR (`VICTORY_REBAR_UPLOAD`, idea from rodrion-vi): GPU time down a little, frame rate unchanged here; off by default. |
| `7272b73`, `790b134` | CPU-read detector (`VICTORY_WATCH_GPU_TEXTURES=1`, diagnostics): found the code that reads the water textures. |
| `0df7901` | **Integer render targets are never blended.** A title blend state that blends every target, an `R8_UINT` one too, made the host refuse the mesh pipeline `[mesh f5e50eb2108c425b, pixel ce3f0432a08f1d35]` in every run (D3D12 error 676). It is made now. |
| `a862386` | **A lane mask parked in other registers and moved back is the number it was.** Compute `926c46bab3b86bd6` translates now; the warm-up refuses 56 shaders instead of 58. |
| `052038b` | Card test for the BVH box test: an absent child whose box is nearer sorts after a child that is there. |
| `374021e` | Defaults: texture room 94 % of the card, ray-tracing resolve pipelines made. |
| `6229aae` | Scalar buffer loads read four dwords a call (`VICTORY_SHADER_OPT` bit 64, off by default). |
| `f6d04ed`, `c62da9f`, `790b134` | Diagnostics (below). |

The fixes come with tests: the blend fix builds the mesh pipeline on the card, the
lane-mask tests run the translated code on the card.

## Findings

From diagnostic runs on the test machine. Each is recorded with its evidence
in the commit messages.

- **The boat sinks** because the title's CPU code reads two textures the
  GPU writes: the 256x256 water simulation outputs (`R32_FLOAT` and
  `R32G32_FLOAT`, read at `+0x4e61029`, `+0x4e61794` and `+0x4e612d4` of its
  main image). On the console CPU and GPU share memory; here those textures
  live on the card only, so the CPU read stale memory and the hull went from
  -2.1 m to -4.1 m. Upstream's `texback` copies them back
  (`VICTORY_TEXTURE_BACK=1`).
- **Where a frame goes** (prison, 4K, run32, about 290 ms): copying GPU
  results back to title memory about 160 ms (72 MB), the pass over the view
  heap about 90 ms, uploads about 75 ms (190 MB). Neither chip is busy
  (GPU about 60 %, CPU about 45 %): the time is the hand-off between them.
  The Series S render path halves the copy-back's bytes.
- **`VICTORY_QUEUE2=3`** (in the fork, off by default) moves the copy-back
  fetch and the uploads of changed bytes to the card's copy engine: copy-back
  104-143 ms to 37-48 ms a frame (run41 to run42); the frame itself barely
  moved, its time went to preparing (about 110 ms) and uploads (about 60 ms).
- **`VICTORY_HEAP_PASS_MARKED=1`** (already in the fork, off by default)
  takes the heap pass from 136-203 ms to 86-102 ms.
- **Tried without gain here:** ReBAR uploads, `VICTORY_WIDE_MERGE=1`,
  `VICTORY_JOIN_COPIES=1`.
- **Other builds on this PC:** Neptune (VIctory Edit.) 1.5 fps (its settings
  overflow a 12 GB card: 11.1 GB used against a 10.3 GB budget); Belobug's AMD
  fork about 3 fps with a black screen on the first run.
- **Missing textures.** The card is full (about 8 to 9.7 of a 10 GB budget);
  textures picked by index wait, and the ones that never arrive draw black.
- **Pipelines the title goes without.** About 30 a run go through device
  method `0x218`, which takes precompiled GFX10 code and a metadata block;
  its layout is not known yet.
- **Dropped draws.** The 100 to 600 draws dropped per frame are stages of the
  ray-tracing structure builder, which is off.

## Plan

1. Indoor light: test the ray-tracing structure builder
   (`VICTORY_BVH_BUILDER=1`); if it is too slow or does not fit the card, a
   stand-in ambient light as Neptune and Belobug have.
2. Check the boat floats with `VICTORY_TEXTURE_BACK=1`.
3. Frame rate: the copy-back (Neptune's own-queue fetch and quiet copy-back
   need its write journal, which this tree does not have). Goal: 10 fps.
4. Missing textures: keep room on a 12 GB card for faces and near objects.
5. Device method `0x218`, and the 56 shaders that still do not translate.

## Inherited from the forks

From victory-fork and victory-amd (each change is explained in a comment where
it is):

- Meshlet culling reads its scene buffers as the right kind of view; per-draw
  descriptor tables no longer run out; light packed by compute shaders is read
  back for every mip and layer.
- Many more shaders translate (fog, shadow mask, hair, skinning, fire,
  materials); gathers at a named mip level; implicit-level samples outside
  pixel shaders.
- Ray tracing, off by default: the title's structure-builder programs
  translate, title memory can be read by address from shaders, and the
  hardware ray/box and ray/triangle tests are emulated in DXIL. See
  [docs/fork-ray-tracing.md](docs/fork-ray-tracing.md).
- Memory sync: a write through one view of a physical page is seen by the
  copies taken through its other views; descriptor records the title copies as
  plain bytes are mirrored; kernel reads into title memory are read beside.
- Speed: large title buffers get card copies with shader writes found by
  hashing on the card; submissions can be sent ahead of the wait
  (`VICTORY_LATE_WAIT`, `VICTORY_RELAY`); shader clean-up passes cut the
  host instruction count by about 43 %.
- AMD: texture views told apart by the memory their mips take, lost views put
  back (`VICTORY_TEXTURE_SWEEP`), dispatches over 65,535 groups split, view
  indices clamped, programs that stall the AMD compiler left out.

Most speed changes sit behind switches that are off by default; each is
described in a comment where it is read.

## Getting started

Requirements: Windows 10/11 x64, a CPU with AVX2, a Direct3D 12 GPU with
shader model 6.6 and mesh shaders, Rust (pinned in `rust-toolchain.toml`). The
title needs several GiB of free Windows commit at startup; if `crashes.log`
reports Windows error 1455, close other applications or enlarge the paging
file.

```powershell
cargo xtask install-hooks                # asset-policy check on every commit
copy victory.example.toml victory.toml   # then set your paths (outside the repo)
cargo test --workspace                   # needs no game data
cargo run -p victory -- config           # check your paths
```

Build, warm the shader cache, run:

```powershell
cargo build --release -p victory
target\release\victory.exe warm <game dir>
target\release\victory.exe run <game dir>\<the title's exe> --execute
```

For the Series S render path see "Recommended run" above. The window caption shows the frame
rate. Saves, settings and `crashes.log` are under the data folder (default
`%LOCALAPPDATA%\victory\data`).

Input: XInput pads. With none connected the keyboard is a pad while the
title's window has focus:

| Keys | Pad |
|------|-----|
| W A S D / I J K L | left / right stick |
| arrows | D-pad |
| Enter, Backspace, X, Y | A, B, X, Y |
| Q / E, Z / C | shoulders, triggers |
| F / H | stick clicks |
| Esc, Tab | Menu, View |

## Commands

```
victory inspect <exe|dll> [--imports] [--exports]
victory census [paths...] [--missing] [--strict]
victory lib-exports <import.lib> [--list]
victory shader-info [paths...] [--verbose] [--disasm]
victory translate [paths...] [--validate] [--pipelines] [--list] [--jobs N]
victory warm <game dir> [--jobs N]
victory run <exe> [--execute] [--unimplemented trap|zero] [--strict]
            [--console series-x|series-s] [-- <title arguments>]
victory config
```

Logging: `VICTORY_LOG=victory_loader=trace,info` (a release build logs info
and above).

## Debugging

```powershell
cargo build --profile diag -p victory --features diagnostics   # target\diag\victory.exe
```

| Variable | Effect |
|----------|--------|
| `VICTORY_CAPTURE=<dir>` | saves presented frames (powers of two, `VICTORY_CAPTURE_FRAMES=<list>`); **F9** captures the next frame, its draws listed in `frame-<n>-ops.txt` |
| `VICTORY_DUMP_TARGETS=1` | with the above, every render target of those frames |
| `VICTORY_DUMP_SHADER=<hash>,...` | on captured frames, the raw buffers those shaders read |
| `VICTORY_SHADER_SAVE=<dir>` | each shader as the title gave it and as translated; whole calls of device methods `0x1c8`/`0x218` |
| `VICTORY_WATCH_GPU_TEXTURES=1` | logs code that reads or writes the memory of textures the GPU writes |
| `VICTORY_NULL_MAGENTA=1` | a texture that is not there shows magenta instead of black |
| `VICTORY_D3D12_DEBUG=1` | host debug layer (needs Windows' Graphics Tools) |
| `VICTORY_D3D12_GBV=1` | with the above, GPU-based validation |
| `VICTORY_D3D12_DRED=1` | on a GPU failure, the commands in flight |
| `VICTORY_LOG=<target>=debug,...` | debug and trace output |
| `VICTORY_STACKS=20,60` | every thread's stack at those seconds |
| `VICTORY_WATCH=<rva>,...` | logs a title u32 when it changes |
| `VICTORY_KEYS=30:Enter,35:X` | presses keys at those seconds |
| `VICTORY_CRASH_SAVE=<rva>:<len>,...` | saves title memory on a crash |
| `VICTORY_PIPELINE_WAIT_MS=24` | shorter wait for background pipeline creation |
| `VICTORY_SKIP_PIPELINES=<hash>,...` | omits draws whose pipeline holds a listed shader |
| `VICTORY_GPU_TIMING=1` | each list timed on its own, slow ones logged |
| `VICTORY_D3D12_WARP=1` | software adapter (slow; cannot reset the driver) |
| `VICTORY_SHADER_LOOP_LIMIT=<n>` | translated loops stop after `n` iterations |
| `VICTORY_AUDIO_CAPTURE=<file.wav>` | records what the title plays |

The diagnostics build also counts dropped draws by reason with their shaders.
Every build appends crashes and host GPU failures to `crashes.log`.

## Credits

- **[VIctory](https://github.com/Marvisak/VIctory)** by Marvisak and its
  contributors: the base of everything here (loader, runtime services,
  Direct3D 12 back end, shader recompiler). Upstream's history is kept intact.
- **[victory-fork](https://github.com/fxceless/victory-fork)** by fxceless:
  the fork this one grew from, and the idea of looking at textures again
  after their upload (`a803c87`).
- **victory-amd** by **myokex** (commit `c6dc4d8`): the guard over every view
  of a memory page, adapting victory-fork pull request
  [#115](https://github.com/fxceless/victory-fork/pull/115) by **Spexivus**,
  [its correction](https://github.com/Spexivus/victory-fork/pull/1) by
  **joroyt**, and part of
  [VIctory #1122](https://github.com/Marvisak/VIctory/pull/1122) by
  **tightfall**.
- **Joshua Odom**: the read-beside fix for kernel reads into title memory
  (`10ea38e`).
- Pull requests whose ideas were written anew here: descriptor-record mirroring
  (#305, **Flyander**), levels on demand (#747, **AscilCH**), write-watch and
  in-flight card fetches (#670, #648, **Jonirulah**), and others named in the
  code.
- **[XCL](https://github.com/ivall/xcl)** by ivall and its contributors
  (GPL-2.0-only): a reference for how the same title behaves under another
  layer; fixes it inspired were written anew.
- **Neptune (VIctory Edit.)** by Traffic and zm0kie, as
  [WillyboPeep/neptune-ve](https://github.com/WillyboPeep/neptune-ve) carries
  it: the Series S render path (`VICTORY_RENDER_S`), the second present plane
  and the FidelityFX redirect (`VICTORY_FSR_DLL`), ported here.
- **[AnyXboxX](https://github.com/LooperSalty/AnyXboxX)** (LooperSalty): its
  measured switch sets and notes pointed at the heap pass and the 12 GB limits.
- **[rodrion-vi](https://github.com/Paradoxo321/rodrion-vi)** (Paradoxo321):
  the ReBAR upload idea.
- **AMD**: the signed FidelityFX upscaler DLL, from its
  [FidelityFX SDK](https://github.com/GPUOpen-LibrariesAndSDKs/FidelityFX-SDK)
  (not in this repository).
- This fork's changes were developed by StrixEvo with
  [Claude Code](https://claude.com/claude-code).

See [docs/architecture.md](docs/architecture.md) for how VIctory works and
[docs/asset-policy.md](docs/asset-policy.md) for what may enter the
repository.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
