# NFR section 3: memory, the readings

> The budget and its table are in `../nfr.md` section 3; this file holds every reading
> taken against it, in the order recorded, each naming its machine, scene and commit.
> New readings go here. A reading that replaces a proposal also updates the budget line
> in `nfr.md`. A bare `§N` in this text is a section of `nfr.md`.

**The contour-sharp default leaves the mirror's bytes unchanged (Plan 0039's fix pass, the
merged binary of 4d68e91 against main's binary of dedd83c, both adapters):** the level reads
157 872 640 B live and 259 203 264 B allocated under both, and the streamed flight's full
residency (17 077 full and 40 700 LOD-only chunks) 871 679 852 B live and 3 221 014 080 B
allocated under both; the flights' peaks differ with the flight (882 016 044 to 996 021 932
B live across both binaries).

**The `face` default carries no attribute and no band term (Plan 0054's close readings, main
`28948a5`'s release binary against `86611cc`'s, RTX 3080 Laptop GPU, exclusive perf lock and
memory cap, one quiet round each at a start load of 1.75 to 1.99, `r_full` 16; logs
`renders/plan-0054-live/p4c/close/` (local)).** The level: main's default 157 872 640 B live and
259 203 264 B allocated; the `face` default 62 846 720 B live and 88 285 376 B allocated. The
streamed flight's full residency (17 077 full and 40 700 LOD-only chunks): main's default 871
679 852 B live, peak 992 795 308 B, 3 221 014 080 B allocated; the `face` default 502 928 492 B
live, peak 532 183 724 B, the same allocation. The opt-in `--look field`, as a reading of an
option: the level 159 089 984 B live (96 243 264 B of band slots over 418 668 band bricks), 223
027 392 B allocated; the flight 941 827 116 B live (438 898 624 B of band slots over 1 829 483
band bricks), peak 967 839 468 B, against Plan 0054 owner answer 4's 106 and 530 MB.

**The `face` default's frame targets (Plan 0054's review fix, `Frame::frame_bytes` printed by
`frame_bytes_add_the_histories_and_the_output_target` in `apps/flythrough/tests/upscale.rs`,
1080p output from 1280 x 720 internal, lavapipe, exclusive perf lock).** Main `28948a5` reads
130 267 168 B; Plan 0054's tip before the fix (`3d6718d`) 145 012 800 B, the split walk's
hand-off list (16 B of header and 16 B a pixel, 14 745 616 B) and its 16 B dispatch words
allocated in every frame; after the fix 130 267 216 B, main's plus the 32 B placeholder list
and the 16 B dispatch words. The full list is allocated by the first frame that runs the split
walk (`--look field` with a crisp row); `only_a_split_frame_allocates_the_split_list` in
`apps/flythrough/tests/look.rs` pins both.

**The frame's targets (Plan 0024 Phase 11, `Frame::frame_bytes` printed by `flythrough
--autopilot --headless --seed 1 --render 1920x1080`):** 308 736 000 B with full-resolution
lighting and 195 379 200 B at half resolution, against the 1.5 GB line; the light-sample buffer
adds 16 MiB plus its 16 KiB candidate table (2^20 samples of 16 B), the blue noise 16 KiB and
the emission scale 256 KiB. Per pixel at full resolution: the hit buffer twice (48 B), the
translucent layers 8 B, the irradiance 8 B, the accumulation and moments twice (32 B), the
filter's pair 16 B, the reservoirs twice (32 B), the output 4 B, and the gradient strata 8 B
per 9 pixels. The plan's projection was about 276 MB.

**The probe volume (Plan 0029 Phase 4, 83ae737's tree, the same autopilot command):**
`Frame::frame_bytes` reads 254 216 224 B at 1080p with half-resolution lighting, 58 837 024 B
above Plan 0024's 195 379 200: the atlases 50 331 648 B, the state 131 072 B, the update list,
ray buffer and uniform 4 227 104 B (the volume, 54 689 824 B, printed as `autopilot: probe
volume`), and the gather's lighting-grid target 4 147 200 B (8 B per lighting pixel). The
plan's bar (amended 2026-10-22, owner) counts all of it: 58 837 024 B and no more, against the
1.5 GB line.

**The frame's targets with the upscaler (Plan 0030 Phase 5, `Frame::frame_bytes`, 1080p
output):** 124 621 440 B at scale 1.5 (1280 x 720 internal): the 86 835 840 B of Plan 0024's
frame at 1280 x 720, plus the composite's `rgba8` target grown to the output size (4 608 000
B) and two output-size `rgba16float` histories (33 177 600 B); 228 556 800 B at scale 1 with
the jitter on (195 379 200 B of the frame plus the histories); 195 379 200 B at scale 1 with
the jitter off, as before. Plan 0030 Phase 8 (12c6c57): unchanged at 124 621 440 B at scale
1.5; the light-change clip adds no target (the upscale pass's settings are a 96 B uniform).

**The frame's targets under FSR 2 (Plan 0041 Phase 6, 6e64744's tree, `Frame::frame_bytes`,
1080p output, the probe volume included as the fly-through runs it):** our upscaler at 1.5
reads 181 154 464 B (Plan 0030's 124 621 440 B plus the probe volume and its gather target,
56 533 024 B at that lighting size); FSR 2 at 1.5 reads 305 574 892 B, which is ours less
the two output-size histories (33 177 600 B, not allocated in FSR mode) plus FSR 2's
inventory, 157 598 028 B: 66 355 200 B at the render size (72 B a pixel: the rgba32float
colour input, depth, motion, the masks, the reconstructed previous depth, the dilated depth,
two dilated motions, the lock luma, the prepared colour and the dilated masks), 91 238 400 B
at the output size (44 B a pixel: the rgba16float upscaled colour twice, which RCAS also
writes into, the lock status twice, the luma history twice and the new locks) and 4 428 B
for the luminance pyramid's two stored mips, the exposure texels and the 1 x 1 debug weight.
`frame_bytes_under_fsr2_swap_the_histories_for_fsr2s_inventory` asserts the sum. FSR 2 at
scale 1 reads 494 763 692 B; Plan 0024's frame with the probe volume 254 216 224 B.

**Readings (dev reference, 2026-10-16, Plan 0001 Phase 4 commit, `flythrough` cave seed 1,
`cargo run --release -p flythrough --example world_budget`):**

| Extent (chunks) | `Full` fraction | CPU bytes | Mirror bytes (no LOD) |
|---|---|---|---|
| 20 × 20 × 8 (the level, 128 × 128 × 51 m) | 23.41 % | 399.6 MiB | 383.8 MiB |
| 32 × 32 × 32 (204.8 m cube) | 20.79 % | 3661.6 MiB | 3496.2 MiB |

With occupancy masks (Plan 0001 Phase 8), the level-size mirror reads 402 449 984 B from the
fly-through's first sync. Both measured `Full` fractions are above the 10 % assumption above; the cave is about 29 %
air with caverns on a 6.4 m lattice.

With 8-bit palette indices (Plan 0002 Phase 4, ADR-0004) the level-size mirror reads
207 681 088 B: pool 196 407 296 B (512 B for each of the 383 608 `Full` bricks, no wide chunk),
masks 8 B per brick, records 2560 B per chunk (512 entries and a 256-entry `u16` palette),
table 4 B per chunk, from `apps/flythrough/tests/level_mirror.rs` on the dev box.

With the traversal hierarchy (Plan 0015 Phases 2 and 3) the same level-size mirror reads
232 436 992 B: pool 196 407 296 B as above, masks 72 B per `Full` brick (27 619 776 B: a
64-byte opaque mask per slot in groups of eight, plus two translucent words), records 2624 B
per chunk (16 words of brick bits, 512 entries, the palette), table 4 B per chunk, from the
same test on the dev box (lavapipe).

**Streamed residency (dev reference, Plan 0002 Phase 8, unbounded cave seed 1 around its spawn,
hysteresis 1, `cargo run --release -p flythrough --example world_budget -- --stream
R_FULL,R_LOD`, 16 workers, Ryzen 9 5900HS).** Mirror bytes at 8-bit palette indices (ADR-0004):
512 + 8 B per `Full` brick or mip-brick, 2560 B per record for full and LOD-only chunks alike,
plus the `W^3` chunk table. (Since Plan 0015 Phase 3 a slot carries 512 + 72 B and a record
2624 B; the example counts the new figures, the table below keeps the readings taken at the
old ones, and the paragraph after it holds Plan 0015's.) CPU bytes: full chunks' slot arrays and `Full` bricks, LOD-only
chunks' mip-brick slots and `Full` mip-bricks. In every row about 18.5 to 18.8 % of a full
chunk's bricks and about 39 % of the ring's mip-bricks are `Full`.

| r_full | r_lod | Full chunks | LOD-only chunks | CPU MiB | Mirror MiB (full) | Mirror MiB (LOD) | Mirror MiB (total) |
|---|---|---|---|---|---|---|---|
| 10 | 14 | 4 169 | 7 344 | 634.2 | 205.7 | 123.4 | 329.2 |
| 10 | 18 | 4 169 | 20 236 | 1 006.2 | 205.7 | 336.0 | 541.9 |
| 12 | 16 | 7 153 | 9 924 | 1 012.7 | 355.9 | 165.5 | 521.5 |
| 12 | 20 | 7 153 | 26 248 | 1 486.2 | 355.9 | 436.0 | 792.2 |
| 16 | 20 | 17 077 | 16 324 | 2 210.0 | 855.1 | 270.5 | 1 126.0 |
| 16 | 24 | 17 077 | 40 700 | 2 915.9 | 855.1 | 673.9 | 1 529.5 |

**With the traversal hierarchy (dev reference, Plan 0015 Phase 6, d3d2f1c, headless, NVIDIA
GeForce RTX 3080 Laptop GPU, Ryzen 9 5900HS):** mirror bytes at 512 + 72 B per `Full` brick
or mip-brick and 2624 B per record. The level (`--level --seed 1`) holds 232 436 992 B live
and 238 303 552 B allocated (207 886 080 and 213 662 016 B at 106998d, before the masks grew).
The streamed defaults at rest after full residency (`--seed 1 --frames 3000`, 17 077 full and
40 700 LOD-only chunks) hold 1 786 641 644 B live and 1 865 490 412 B allocated
(1 607 517 932 and 1 679 368 172 B at 106998d). The autopilot's flight
(`--autopilot --headless --render 1920x1080`) peaks at 1 889 760 188 B live and
2 714 672 620 B allocated, against Plan 0002's 1 672 823 572 B live and 2 431 425 836 B
allocated: about 217 MB more live and 283 MB more allocated, under the 3 GB line; the
residency latency wanted-to-mirrored reads p95 493 ms, max 1804 ms over 72 849 chunks
(p95 472 ms, max 2362 ms at e564f44), against the 500 ms proposal.

**Under motion (dev reference, Plan 0002 fix pass, 076b1c4, headless `flythrough --autopilot`,
unbounded cave seed 1, r_full 16, r_lod 24, h 1, 15 workers, RTX 3080 Laptop GPU, NVIDIA
610.57.04, Ryzen 9 5900HS, two runs):**

- **World VRAM:** the mirror peaked at 1 672 823 572 and 1 692 743 692 B by live accounting
  (`GpuWorld::mirror_bytes`) and at 2 431 425 836 B allocated (`GpuWorld::allocated_bytes`,
  both runs), against the 3 GB line, then read on allocated bytes (buffers grew by half
  again and never shrank). At full residency, 1 603 820 012 B live and 1 675 304 748 B
  allocated. Since Plan 0017 (ADR-0013) the line is read on live bytes against the reserved
  budget; the readings below replace these allocated figures.
- **World RAM:** resident chunks account for 3 242 091 312 and 3 239 741 232 B at peak (slot
  arrays, `Full` bricks and mips), and persisted RAM for 233 616 B. The process peak RSS
  (`VmHWM`, the whole fly-through, dev reference) reads 3 869 564 928 and 3 873 890 304 B,
  about 3.87 GB against the 4 GB line. RAM is the tightest margin at these defaults; they are
  revisited on the baseline card at M2.
  Since Plan 0021 (ADR-0012's packed brick, closed 2026-10-20) the reading is 0.44 GB of
  resident chunks and about 0.97 to 1.01 GB `VmHWM` on the dev box ("The packed brick" below);
  it replaces these figures, and the 4 GB line holds with room.
  Since Plan 0027 (ADR-0022, closed 2026-10-21) the mirror also holds its staging belt's
  mapped chunks in host RAM: 1 to 2 chunks of 4 MiB in flight (§2's quiet runs), at most
  `BELT_KEEP_CHUNKS` (3, 12 MiB) between syncs, a transient burst of about 56 chunks during
  the bounded level's unbudgeted first sync, released once its copies complete.

**Attribute pages (dev reference, Plan 0005, RTX 3080 Laptop GPU).** The contour attribute
(`u16` per voxel) lives in attribute pages of the pool: 1040 B more live per `Full` brick of a
full chunk, and a page holds 1364 bricks in 4096 slots. At the level (`flythrough --level
--headless --seed 1 --frames 1`, 6d77f12): 207 681 088 B live and 213 334 144 B allocated
without attribute pages; 606 633 408 B live and 613 759 104 B allocated with them
(`--look contour`), a live rise of exactly 1040 x 383 608 B. 128 B more with 64-byte material
rows at 37b22fa. Streamed (`flythrough --autopilot --headless --look contour`, 37b22fa, r_full
16, r_lod 24, h 1, 15 workers): at full residency 3 309 723 820 B live and 4 103 413 164 B
allocated, over the 3 GB line, against Plan 0002's 1 675 304 748 B allocated in face mode; the
run then stopped in its outbound flight on `wgpu error: Out of Memory` on the 8 GiB card, so
no peak was read. A recorded miss: ADR-0011 keeps contour planes, and fitting this footprint
under the 3 GB line is owed to the surface follow-up plan (Plan 0005 follow-up 2).
Since Plan 0015's masks an attribute-page brick costs 1168 B more live (two 512 B attribute
slots and their two 72 B mask entries). At the level (`flythrough --level --headless --seed 1
--look contour --frames 3`, Plan 0013 Phase 5, RTX 3080 Laptop GPU, exclusive perf lock,
1-minute load 3.47): 680 491 136 B live, the face level's 232 436 992 B plus 1168 x 383 608 =
448 054 144 B, under a 797 779 456 B reservation. The layout is unchanged; surface-only
attribute storage is ADR-0012's mirror-slot plan.

**Size-classed slots, projected (arithmetic, not a reading; Plan 0028 Phase 1, `world_budget
--chunks 20x20x8 --census` and `--stream R,24 --surface`, cave seed 1, dev box).** A `Full`
brick's slot is 64-byte units (mask, header, 0/1/2/4 plane units or 16 of raw ids: 2, 3, 4, 6
or 18 units) in bins of 2, 3, 4, 6, 8, 12, 16, 20, 24 and 36 units; a record is 2112 B. With
contour attributes a full chunk's brick adds a surface-mask unit and `ceil(n / 32)` units for
its `n` chunk-local surface voxels (an air face neighbour inside the chunk, or the chunk's
face). The level: pool 56 075 200 B, records 6 758 400 B, 62 846 720 B with table and rows
(232 436 992 B live today). Streamed, at full residency (57 777 records):

| residency | plain look | contour look | surface voxels (chunk-local / exact plane) | attribute units per brick |
|---|---|---|---|---|
| 12/24 | 409 969 196 B | 563 102 380 B | 37 518 244 / 23 899 197 (1.570) | 3.07 |
| 14/24 | 451 317 548 B | 700 607 596 B | 61 043 072 / 39 015 306 (1.565) | 3.08 |
| 16/24 | 502 928 492 B | 871 679 852 B | 90 331 692 / 57 703 350 (1.565) | 3.08 |

**Size-classed slots, read (dev reference, Plan 0028 Phase 2, RTX 3080 Laptop GPU, exclusive perf
lock, 1-minute load 5.2 to 8.8).** The level (`flythrough --level --headless --seed 1 --frames 1
--look plain`, and `apps/flythrough/tests/level_mirror.rs` on lavapipe) holds 62 846 720 B live
(56 075 200 B of 2-, 3- and 4-unit slots, 3200 records of 2112 B, the table, the rows) under an
88 285 376 B reservation, against 232 436 992 B live before; its first sync uploads 63 027 696 B
in 405 writes in 342 ms against 500 ms for the pre-plan binary (1d1e67c, 232 617 968 B in 435
writes) in the same session, lights included (65 to 67 ms in both). The streamed plain spawn at
full residency (16/24, `--frames 3000`) holds 502 928 492 B live against 1 786 641 644 B, as
projected.

**Surface-only contour attributes, read (dev reference, Plan 0028 Phase 3, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 9.5 to 10.2).** The level in the default look (`flythrough
--level --headless --seed 1 --frames 3 --look contour`) holds 157 872 640 B live (680 491 136 B
before) under a 259 203 264 B reservation; its first sync uploads 158 053 616 B in 826 writes in
687 and 690 ms against 652 and 659 ms for 1d1e67c (232 617 968 B), the chunk-local surface
masks computed on the render thread; the attribute pass's initial fill of its 383 608 bricks
reads 34.7 and 47.2 ms against 55.2 ms. The default autopilot flight (`--autopilot --headless
--seed 1`, same load) keeps `r_full` 16 with no radius change and no deferral, peak pressure
0.6447 (records; units 0.3027), peak 1 003 260 588 B live, where 1d1e67c lowered `r_full` to 12
at pressure 0.911 with 2 864 318 452 B live.

**The streamed default at `r_full` 16 (dev reference, Plan 0028 Phase 4, RTX 3080 Laptop GPU,
exclusive perf lock, 1-minute load 1.49 to 9.53 from the other lanes' work; `flythrough
--autopilot --headless --seed 1`, paired with 1d1e67c).** The default look reaches full
residency at 16/24 (17 077 full, 40 700 LOD-only chunks) with `r_full` 16: 871 679 852 B live
there, peak 941 911 020 B over the flight, peak pressure 0.6514 (records; units 0.2701), no
radius change, no sync with a deferral, return hash matching; 1d1e67c settled at `r_full` 13
with 2 864 309 692 B live, pressure 0.911 and four radius changes. Plain: 502 928 492 B at
full residency, peak 532 183 724 B, against 1 786 641 644 and 1 890 756 140 B. Fragmentation at
full residency (the fly-through's `bins:` line): 11 908 spare units of 11 705 061 live (0.10 %)
in the default look over six bins (2, 3, 4, 6, 8 and 12 units), 3817 of 5 943 321 (0.06 %) in
plain over two; the same figures on the slow adapter's runs.

**The reserved budget (dev reference, Plan 0017 Phase 4, ef37395, headless `flythrough
--autopilot --headless`, unbounded cave seed 1, h 1, 15 workers, RTX 3080 Laptop GPU, Ryzen 9
5900HS, exclusive perf lock, 1-minute load 10.6 to 23.2 from other builds on the box, two runs
each).** The mirror reserves its pool, masks and records once at startup (`--mirror-budget`,
default 3072 MiB) and never reallocates; `allocated_bytes` is the budget rounded down to whole
pages and records. Records are reserved for the residency's ball over 0.7 (93 239 at 16/24,
244 659 136 B; 72 690 at 14/22) instead of the window cube. Pressure is the larger of pool-slot
and record occupancy; the fly-through lowers `r_full` a chunk after 30 frames above 0.9 and
restores it after 300 frames under 0.7, down to 6. A sync writes the nearest chunks that
fit and defers the rest, which stay queued (ADR-0013 Outcome). No run stopped on
out-of-memory; every return hash matched. The face rows, every column, are at ef37395; the
contour rows, every column including full residency, were re-taken at b474737 (the partial
sync with its per-chunk check), at load 13.6 to 26.0.

| run | split (slots, records) | full residency live | flight peak live / budget | peak pressure (slots, records) | radius changes, final r_full | deferrals: syncs (empty), chunks, longest streak | reserve, first frame |
|---|---|---|---|---|---|---|---|
| 16/24 face | 5 095 424, 93 239 | 1 786 641 644 B, 0.620 | 0.5844, 0.5667 | 0.6473 (0.5803, 0.6473); 0.6200 (0.5643, 0.6200) | none, 16 | none | 83.1, 87.1 ms; 2669 (cold), 554 ms |
| 16/24 contour | 5 095 424, 93 239 | 2 864 309 692 B at r_full 13, 0.911 (both runs) | 0.8894, 0.8894 | 0.9114 (0.9114, 0.6197); 0.9114 (0.9114, 0.6197) | 4 each (16 to 12), ends 12 | 1 (0), 1605, 1; none | 100.6, 96.2 ms; 2330 (cold), 857 ms |
| 14/22 face | 5 185 536, 72 690 | 1 312 177 076 B, 0.612 | 0.4339, 0.4179 | 0.6469 (0.4203, 0.6469); 0.6118 (0.4063, 0.6118) | none, 14 | none | 82.2, 97.9 ms; 564, 1105 ms |
| 14/22 contour | 5 185 536, 72 690 | 2 607 896 708 B, 0.822 (both runs) | 0.8747, 0.9198 | 0.9020 (0.9020, 0.6118); 0.9442 (0.9442, 0.6327) | none, ends 14; 1 (14 to 13), ends 13 | none; 15 (0), 19 801, 3 | 105.4, 197.3 ms; 673, 1030 ms |

Contour fits the 3 GB line at both radii only by the rule: at 16/24 it settles at r_full 12
(11 513 full, 46 264 LOD-only chunks at full residency, r_full 13), at 14/22 it stays at 14 or
drops to 13 in the flight. With attribute pages the pool runs out of pages while slots are
spare in plain pages that each hold a live mip-brick. Those syncs upload what fits and defer
the rest; no sync was empty and every deferral streak cleared within 3 syncs (the bar is 90).
The one 16/24 deferral came at pressure 0.796, under the 0.9 trigger, and cleared in its own
sync with no radius change; the 14/22 deferrals came at 0.855 to 0.944, after the decrease to
13 (tick 796, first deferral at tick 807). Two of the four runs deferred nothing. A recorded
finding, not a miss of the line: live bytes peak at 2.96 GB under 3 GB. At 52cf84d, before the
per-chunk check, the same runs read longest streaks of 1, 3, 2 and 6 syncs. These readings stand
at the close tip (16e17a7, v0.17.0): the later fixes change only the wide path (more than 256
materials) and syncs that write nothing, neither of which these runs reach. Deferring under
the 0.9 trigger with attribute pages is page fragmentation, Plan 0017's open followup.

**The default look (dev reference, Plan 0013 Phase 7, `flythrough --autopilot --headless
--seed 1` with no `--look`, so contour with detail and attribute pages, r_full 16, r_lod 24, h 1,
the 3072 MiB default reservation, same machine, exclusive perf lock, 1-minute load 3.6 to
11.1, two runs).** Reserved 3 220 387 776 B (5 095 424 slots, 93 239 records). Both runs:
`r_full` 16 to 15 to 14 to 13 at pressure 0.960, 0.982, 0.918 (ticks 469, 499, 664), full
residency at r_full 13 with 2 864 309 692 B live, pressure 0.911, then 13 to 12 at 0.911
(tick 905) and back to 13 at 0.669 and 0.651 (ticks 2444, 2088), ending at 13; lowest 12.
Peak live 2 864 318 452 B, peak live / budget 0.8894, peak pressure 0.9114 (slots 0.9114,
records 0.6513 and 0.6495). Run 1 deferred nothing; run 2 deferred 5032 chunks over 10 syncs
at pressure 0.755 to 0.763, none empty, longest streak 3. Both returned `return: 2 chunks,
hash match` with no wgpu error. The default look therefore holds full detail to the radius
the rule settles at, 12 to 13 chunks (76.8 to 83.2 m), against this section's "about 100 m",
until ADR-0012's mirror-slot plan stores the attribute for surface voxels only; `--look
plain` holds 16 (102 m). Since Plan 0028 (surface-only attributes, below) the default holds
`r_full` 16 (102 m) at 0.87 GB live with pressure 0.65: the "about 100 m" figure is met in both
looks on the dev reference.

**The pressure rule's gate (Plan 0017 Phase 3, same machine and flags, r_full 16, r_lod 24,
no `--look`, `--mirror-budget 1930`: 3 043 328 slots and 93 239 records, 2 021 964 224 B,
reserved in 63.0 and 70.7 ms).** Run 1 (06d1f17, load 24.1): `r_full` 16 to 15 to 14 at
pressure 0.920 right after full residency, back to 15 at 0.676 in the flight; run 2 (ef37395,
load 18.3): 16 to 15 at 0.920, back to 16 at 0.688. Both: 0 syncs past the reservation, peak
pressure 0.9197 (slots 0.9197, records 0.6227 in run 2), peak live / budget 0.8836, return hash
match, `r_full` never under 14. Run 3 (b474737, partial sync with its per-chunk check, load 19.8 to
14.9): 16 to 15 to 14 at 0.920, back to 15 at 0.604; 0 deferred chunks, peak pressure 0.9197
(slots 0.9197, records 0.6488), peak live / budget 0.8836, hash match. At 3072 MiB the same
flight makes no change (peak pressure 0.6363 and 0.6473, 0.6200).

**Brick-array pool (Plan 0014, retired by Plan 0021).** The pool added at most 8 MiB per world (8192 arrays x 1 KiB); the fly-through held a 4096-array reserve (2326 KiB pooled after the `B` bench, 5850218, dev reference). Plan 0021 retired it: a `Full` brick's box is 72 to 352 B and is allocated on the edit path (11.2 ns per expansion; below).

**The packed brick (dev reference, Plan 0021 Phase 8, ADR-0012, Ryzen 9 5900HS, RTX 3080
Laptop GPU, 8440aa9 with Phase 8's two printouts, exclusive perf lock, 1-minute load 8.4 to
14.2 from other lanes' builds).** A `Full` brick is a mask, an ascending palette and bit
planes in one of five classes (72, 136, 200, 352 or 1024 B by material count), behind the
same 16-byte slot. The autopilot (`flythrough --autopilot --headless --render 1920x1080`,
streamed defaults, alternated with main at 5ca4151 in the same window): **resident chunks peak
at 441 596 664 B and the process `VmHWM` at 965 971 968 B**, against 3 242 091 312 B and
3 886 219 264 B for main (Plan 0002's 3.24 GB and 3.87 GB), under the 4 GB line; a second
Phase 8 run read 441 555 208 B and 1 009 381 376 B. `world_budget` CPU bytes: the level
60 807 552 B (58.0 MiB, against 399.6 MiB in the table above), the streamed defaults
415 231 624 B (396.0 MiB: full 274 067 464, LOD 141 164 160, against 2915.9 MiB). The census
at rest (`--census`): the level's 383 608 `Full` bricks hold 1 material (275 466), 2 (107 325)
or 3 to 4 (817); the streamed defaults' 1 640 292 `Full` bricks and 1 158 516 `Full`
mip-bricks hold 1 or 2. Mirror bytes are unchanged by construction: the level reads
232 436 992 B live.

**The showcase cavern beside the level (Plan 0038 close, 2026-10-28, b9f4b08, `world_budget
--source showcase --census` and `--chunks 20,20,8 --census`, seed 1, release, the dev
reference; arithmetic from the generated worlds, no timing):** the showcase's 1176 chunks hold
71 046 `Full` bricks (26 187 of 1 material, 44 859 of 2) against the level's 383 608 in 3200
chunks: CPU bytes 17 620 080 against 60 807 552, mirror bytes 14 453 792 against 62 846 720,
and the fly-through's bounded budget (`level_mirror_budget(world, chunks, contour)`, dig
headroom included) 134 604 544 B against 259 203 328 B. Live in the fly-through: 33 003 744 B
of mirror in 134 604 480 B allocated. Light samples 29 806 of the 1 048 576 reserved (amber
3792, crystal 13 497, amethyst 1552, ember 10 965; the fullest chunk 2880 of its 8192 cap), so
no new VRAM. Phase 5's census at 654d3c6, before Phase 6a moved the ore, read 71 282 `Full`
bricks and 99 326 samples.


**Stats HUD (arithmetic, Plan 0010; ADR-0009).** The overlay holds about 130 KiB of VRAM
under "everything else": a 128 KiB cell buffer at the 256 x 128 cap layout (`u32` per cell),
a 1 KiB sample buffer (256 `f32`), the font and a small parameter block, under the 200 KiB ADR-0009 states.

**Eye adaptation (Plan 0040 close, 2026-10-28; ADR-0033).** `Frame::frame_bytes` on the level
at 1920x1080 internal (same quiet runs as §2's eye-adaptation reading, both adapters): 254 216 224
B at main 05a3a28, 254 476 352 B at 8a3fff3, +260 128 B: the meter's 240 x 135 tiles (259 200 B),
the 32-byte state and its uniform copy, the pass's 96-byte parameters and three 256-byte
read-back buffers. The plan projected +0.26 MB.

**The emissive accumulation in `f32` (Plan 0047 Phase 2, the close's reading at v0.36.0; ADR-0036).** The temporal pass's
accumulation pair is `rgba32float`, 16 B more per lighting pixel. `Frame::frame_bytes` printed by
`flythrough --autopilot --headless --seed 1 --render 1920x1080` (RTX 3080 Laptop, exclusive perf
lock, 1-minute load 7.7, loaded): 184 956 992 B at the default (`own` 1.5, half: 640 x 360
lighting), +3 686 400 B over the merged base (the FSR 2 entry's 181 154 464 B plus the eye
adaptation's meter at 1280 x 720, 116 128 B); 262 770 752 B at scale 1, half (960 x 540
lighting), +8 294 400 B over the eye adaptation's 254 476 352 B. The plan's bar is 16 B per
lighting pixel at most.

**The probe volume on demand (Plan 0052 Phase 4, the close's reading at v0.37.0).** A `Frame`
builds its `ProbeVolume` only on the first encode with indirect light on (or on
`probes()`, `probes_mut()` or a spacing change), so the default frame (indirect off) no longer
holds it. The volume is 54 689 824 B: 12 615 680 + 1 056 768 words of atlases, state and scratch
(54 689 792 B) and its 32-byte uniform. **Reading** (`crates/vxe-render/tests/frame.rs`,
lavapipe, a 64 x 64 frame, exclusive perf lock, 1-minute load 2.2, `1fdac4d`): `frame_bytes`
412 008 B with indirect off, before an encode and after one; 55 101 832 B after an encode with
it on, +54 689 824 B, equal to `ProbeVolume::bytes`; the test asserts the figure. The
fly-through turns indirect light on by default, so its frame is unchanged: `flythrough
--autopilot --headless --seed 1 --render 1920x1080 --adapter NVIDIA` (release, RTX 3080 Laptop,
exclusive perf lock, 1-minute load 9.3 to 17.8, loaded; the close review on `200607c`) prints
frame targets 184 956 992 B, the 0047 figure above, of which the probe volume is 54 689 824 B
(atlases 50 331 648 B, state 131 072 B, update list, rays and uniform 4 227 104 B, spacing 16).
With `--indirect off` it would hold 130 267 168 B (arithmetic, not read). The saving is every
engine `Frame` at `Lighting::default()`, the render tests' host RAM among them.

**Prefabs (arithmetic, Plan 0008; no reading needed).** A `Prefab` holds 2 B of material plus
one mask bit per voxel: a 16³ prefab is 8 KiB + 512 B, a 64³ one 512 KiB + 32 KiB. A parsed
`.vox` model is capped at 256³ per axis (`MAX_MODEL_EDGE`), so the largest `to_prefab`
allocation is about 50 MiB including its index buffer. `StampedSource` holds one rotated copy
of the prefab per placement, so its RAM grows with the placement count, not the prefab count.
**Reading (Plan 0042 Phase 4, 1474e59, `--castle`, release, RTX 3080 Laptop GPU, Ryzen 9
5900HS, exclusive perf lock, 1-minute load 2.4 to 13.0, so loaded and indicative):** the
generated castle keeps 21 of its 27 tiles (600 x 600 x 500 voxels split at 256): its prefabs
hold 343 566 464 B (the sum of `2n + 8 ceil(n / 64)` over the kept tiles), of which the
castle's listed voxels are 13 154 879 (about 8 % of the kept tiles' 161 678 336 voxels; the
rest is the air in each tile's box). The encoded `.vox` is 52 623 529 B. Process peak RSS (`VmHWM`,
streamed seed 1, headless, `--frames 3000`, 1280 x 720) is 1 242 440 to 1 331 404 kB
with the castle against 949 184 to 986 124 kB without it, three runs each: about 260 to 350
MiB more, the prefabs' 328 MiB plus the build's transients. Chunk generation over the 990
chunks the castle meets (`examples/castle_gen`, single thread, three runs, load 9.1 to 6.3):
mean 423.5 to 427.9 us and p99 2258 to 2380 us per chunk with the castle, against 186.2 to
188.6 us and 1650 to 1710 us for the hall alone.
