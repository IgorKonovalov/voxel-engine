# ADR-NNNN: <decision title>

> **Status:** proposed | accepted | superseded by ADR-NNNN
> **Date:** YYYY-MM-DD
> **Related plan(s):** Plan NNNN (link), if any

## Context

The forces at play, and why this is a *decision* that could reasonably go either way. Two to
four short paragraphs of concrete facts: a GPU feature's availability on the baseline, a memory
figure, a determinism constraint, a consumer requirement, a lesson from a sibling project.

## Decision

One paragraph, active voice, present tense.

> We store a brick as 8³ `u16` material ids, and a brick whose voxels are all one material as
> that single value.

Capture nuance ("X unless Y") here, not in a footnote.

## Consequences

### Positive
- What this unlocks.

### Negative
- What it costs. **This is the most important section, so be honest.**

## Alternatives considered

One paragraph each: what it was, and the one decisive reason it lost. More than three usually
means padding.

### Alternative A: <name>

### Alternative B: <name>

## Outcome

_(Added only at acceptance if implementation falsified something above. Dated. Never rewrite the
body.)_
