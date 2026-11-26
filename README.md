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
| Console profile | Xbox Series X (the default); Series S not tested yet |

## Status (2026-11-26)

**Not playable yet.**

| Stage | State on the test machine |
|-------|---------------------------|
| Boot, menus, start screen | works, 30 fps |
| Opening cutscene | plays; characters, boat, water and sky are drawn |
| The world (first mission) | renders at about 2 to 6 fps (median about 3) |
| The boat | **sinks as soon as the cutscene hands over to play** (see Findings) |
| Faces, some foliage and bridges | sometimes without their texture (black; magenta with `VICTORY_NULL_MAGENTA=1`) |
| Ray-traced light | does not run (the structure builder is off by default) |
| Stability | runs of 5 to 30 minutes end normally; device losses were seen in older runs, none in the latest |

## What this branch changed

| Commit | Change |
|--------|--------|
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

- **The boat sinks.** Its hull goes from -2.1 m to -4.1 m in 40 s while the
  water is at 0 m, and stops there. The "walls" in the water around the boat
  and the swimmers are the water cut away around them (the title's
  water-volume list), sitting under water. Ruled out: the water surface
  shader (`f4fa58d124fdc362`, translates and validates), its volume list
  (right sizes), its wave and ripple textures (small values), the ripple
  query (`92b4790eb4c29f6d`, its results reach title memory), and the
  ray-tracing resolve pass. The game's own physics gets something wrong; the
  check under way is whether it reads a texture the GPU writes (on the
  console CPU and GPU share memory; here those textures live on the card
  only).
- **Missing textures.** The card is full (about 9.1 of a 10.3 GB budget);
  textures picked by index wait, and the ones that never arrive draw black.
- **Pipelines the title goes without.** About 30 a run go through device
  method `0x218`, which takes precompiled GFX10 code and a metadata block;
  its layout is not known yet.
- **Dropped draws.** In every captured frame, the 100 to 600 draws dropped are
  stages of the ray-tracing structure builder, which is off; no other draws
  were lost there.
- **Frame rate.** Each frame copies about 170 MB to the card and 65 MB back;
  the card's own work is about a third of the frame.

## Plan

1. Boat physics: find the input it reads wrong, then give it the right data
   (for a texture the GPU writes: copy it back to title memory).
2. Missing textures: see which textures wait when the card is full, and keep
   room for faces and other near objects. Try the Series S profile.
3. Device method `0x218`: decode its layout from the saved calls, then build
   those pipelines through the GFX10 translator.
4. The 56 shaders that still do not translate, one error kind at a time,
   with a test each.
5. Frame rate: less copying between title memory and the card each frame;
   the goal is at least 10 fps.

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

`--console series-s` runs as a Series S. The window caption shows the frame
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
- This fork's changes were developed by StrixEvo with
  [Claude Code](https://claude.com/claude-code).

See [docs/architecture.md](docs/architecture.md) for how VIctory works and
[docs/asset-policy.md](docs/asset-policy.md) for what may enter the
repository.

## License

GPL-3.0-or-later. See [LICENSE](LICENSE).
