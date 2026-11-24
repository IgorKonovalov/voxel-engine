# ADR-0087: The renderer's pipeline cache is the driver's, through wgpu, as bytes the consumer keeps

> **Status:** proposed (2026-11-24, with Plan 0103)
> **Date:** 2026-11-24
> **Related plan(s):** [Plan 0103](../plans/0103-a-pipeline-cache-for-the-renderer-bytes-the-consumer-keeps.md)

## Context

A cold start compiles every pipeline in the driver: 29.4 s on the RTX 3080 Laptop with an
emptied driver cache, against 0.24 s through a `wgpu::PipelineCache` (Plan 0094 Phase 2a). The
driver's own disk cache usually hides the cost, but it is not ours: on 2026-11-16 NVIDIA's was
over its size limit and every start recompiled.

The engine is a library. It owns no application directory, and since ADR-0055 it reads and
writes no file of its own choosing: a consumer serialises what it wants to keep. A cache that
survives a run has to live somewhere, so who holds it is a decision.

wgpu `=30.0.1` implements the pipeline cache on Vulkan only (its feature's documentation and
the DX12 backend's empty implementation say so). It checks a header of its own on load (the
backend, the adapter, a key from the driver, the length) and does not checksum the payload;
creating a cache from bytes is `unsafe`. Lavapipe keeps no compiled code in it (84 B).

Today a cache exists only as a test hook: one process-wide slot every compute pipeline reads.

## Decision

**The renderer's pipeline cache is the driver's cache through wgpu, wrapped in an engine
envelope and exchanged with the consumer as bytes.** A `ShaderCache` is made per device from
optional bytes and passed explicitly to each object that builds pipelines; `bytes()` returns
what to store. The engine opens no file. The envelope carries a key over the renderer's
composed shader sources and the crate version, the payload's length and a checksum; anything
that does not match, and anything wgpu or the driver refuses, is a cold cache with a stated
reason, never an error the consumer must handle and never a panic. Where the backend has no
cache the type is inert.

## Consequences

### Positive
- A second start no longer depends on the driver's cache surviving, on the adapters where
  wgpu supports it.
- The engine stays file-free; a game stores the bytes with its other data, a tool can keep
  them in memory, a test passes none.
- The cache is scoped to a device by construction: no hidden state, two devices do not cross.
- A truncated or rotted file cannot reach the driver: the length and checksum stop it first.

### Negative
- **Vulkan only.** DX12 gets nothing until wgpu implements it. Windows on DX12 keeps relying
  on the system's shader cache.
- **Every consumer writes the same twenty lines** (read a file, pass the bytes, write the
  bytes back atomically) and can get them wrong; the fly-through is the worked example.
- **More constructor surface:** the frame, the overlay, the loading frame and the presenter
  each take the cache, beside the forms without it.
- **No help on the first start**, and none for the lavapipe gate.
- **The checksum is not a signature.** A file an attacker can write is still handed to the
  driver if they recompute the checksum; keeping the file private is the consumer's job.
- **The key throws the whole cache away** on any shader edit or engine upgrade, where the
  driver could have reused most of it. Bounded growth was preferred to the best warm rate.

## Alternatives considered

### Alternative A: the engine reads and writes the file
Given a path, or picking the OS's cache directory itself. It is less code for a consumer. It
lost because it puts file IO, a directory policy and platform lookups into a library that has
none (ADR-0055), and a consumer with a packaged or sandboxed data store would have to work
around it.

### Alternative B: an engine-side cache of translated shader modules
Keep naga's SPIR-V, or precompile shaders at build time, and skip the driver's cache. It lost
on the reading: a warm start through the driver's cache is 0.24 s with all of naga's work
still in it, so translation is at most that, and the tens of seconds are the driver's compile,
which only the driver's cache removes.

### Alternative C: promote today's process-wide slot to the API
`set_pipeline_cache` once, every pipeline reads it; no constructor changes. It lost because it
is hidden global state in a library, holds one device at a time, and a cache used on another
device is a validation error. It stays as the tests' hook.

### Alternative D: rely on the driver's own disk cache
Nothing to build. It lost because it is the mechanism that failed: its size limit and eviction
are the driver's, and a full cache made every start cold with no word to the user.
