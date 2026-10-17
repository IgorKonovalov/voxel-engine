# Room acoustics and artificial reverberation: external sources

An annotated reading list for the cavern reverb work (Plan 0020, ADR-0015), collected
2026-10-17. Every entry names what it establishes and the figures it claims, so a plan can cite
it by title. Numbers are the sources' own; `nfr.md` holds ours. An entry marked **(not
re-verified today)** records what was known from memory and could not be opened on
2026-10-17; the bibliographic record was confirmed, the quoted detail was not.

Open-access copies sit in `local/` (gitignored); `local/index.txt` lists the URLs that were
fetched.

## The statistical room model

- **Sabine, W. C., "Collected Papers on Acoustics", Harvard University Press, 1922.** The
  reverberation formula `T60 = 0.161 V / A` seconds, with `V` the room volume in m^3 and
  `A = S * alpha` the absorbing area in m^2 (surface area times the mean energy absorption
  coefficient). The mean free path between reflections in a diffuse field, `4V / S`, is also
  attributed to Sabine (Smith, "Mean Free Path", below). Plan 0020 uses both: the ray fan
  estimates `V` and `S`, and `4V / S` sets the delay lengths.
- **Eyring, C. F., "Reverberation Time in 'Dead' Rooms", J. Acoust. Soc. Am. 1(2A):217-241,
  1930.** DOI 10.1121/1.1915175. Replaces Sabine's `S * alpha` with `-S * ln(1 - alpha)`:
  `T60 = 0.161 V / (-S ln(1 - alpha))`. Sabine's formula is the small-`alpha` limit; for
  absorptive rooms Sabine gives too long a time, or absorption above unity when solved from a
  measurement. With the mean free path `d = 4V / S` substituted, Eyring reads
  `T60 = 0.04025 d / (-ln(1 - alpha))`, which needs no separate volume. Plan 0020 uses this
  form.
- **Polack, J.-D., "Revisiting Sabine's formula", Proc. Institute of Acoustics (UK).**
  <https://www.ioa.org.uk/system/files/proceedings/jd_polack_revisiting_sabines_formula.pdf>
  (the PDF is served to browsers only). States that the mean free path of a three-dimensional
  ergodic enclosure is `4V / S` regardless of shape (the two-dimensional analogue is
  `pi S / P`), citing Joyce for the proof; the figure is therefore not a box-only result.
- **Kuttruff, H., "Room Acoustics", 6th edition, CRC Press, 2016.** The textbook for the
  diffuse-field model: the mean free path, the energy balance that gives the steady-state
  reverberant intensity `4W / R` for a source of power `W` and room constant
  `R = S alpha / (1 - alpha)`, and the critical distance `sqrt(R / 16 pi)` at which the direct
  and reverberant intensities are equal. **(Page numbers not re-verified today; the relations
  are standard and Plan 0020's level calibration follows them: a unit-energy source impulse
  leaves `16 pi / R` of energy in the tail, independent of the distance to the listener.)**
- **Schroeder, M. R., "New Method of Measuring Reverberation Time", J. Acoust. Soc. Am.
  37:409-412, 1965.** Backward integration of the squared impulse response gives the ensemble
  average decay curve in one measurement. Plan 0020's headless tests read `T60` this way: the
  curve's slope between two levels below the tail's start, extrapolated to 60 dB (the T20 and
  T30 conventions of ISO 3382-1).
- **Abel, J. S. and Huang, P., "A Simple, Robust Measure of Reverberation Echo Density", AES
  121st Convention, San Francisco, October 2006.** The normalized echo density: in a sliding
  window, the fraction of samples more than one standard deviation from the mean, divided by
  the fraction Gaussian noise would give; 1 means a fully dense tail. The measure correlates
  strongly with listening tests. Plan 0020 records it for the rendered tail at 100 and 300 ms.

## Artificial reverberators

- **Schroeder, M. R. and Logan, B. F., "'Colorless' Artificial Reverberation", J. Audio Eng.
  Soc. 9(3):192-197, 1961**, and **Schroeder, M. R., "Natural Sounding Artificial
  Reverberation", J. Audio Eng. Soc. 10(3):219-223, 1962.** The all-pass reverberator and the
  parallel-comb-plus-series-all-pass design. The 1962 paper names the targets: an echo
  density of about 1000 per second and a flat frequency response; one 40 ms feedback delay
  gives 25 echoes per second, so parallel combs alone are impractical. Local copies in
  `local/`.
- **Stautner, J. and Puckette, M., "Designing Multi-Channel Reverberators", Computer Music
  J. 6(1):52-65, 1982.** The feedback delay network (FDN): several delay lines whose outputs
  are mixed by a matrix and fed back to every input. Gerzon proposed the orthogonal-matrix
  form in 1971 (Studio Sound), largely unseen; this paper is the one the field cites. Local
  copy in `local/`.
- **Jot, J.-M. and Chaigne, A., "Digital delay networks for designing artificial
  reverberators", AES 90th Convention, preprint 3030, 1991.** The FDN design used since:
  a unitary (lossless) feedback matrix, one absorptive filter per delay line that sets the
  decay time per frequency band, and a tonal correction filter so the decay time does not
  change the spectrum. Paywalled; the design is restated in Smith's book below.
- **Jot, J.-M., "Efficient models for reverberation and distance rendering in computer music
  and virtual audio reality", Proc. ICMC 1997.** HAL hal-01106168. The FDN as the late
  reverberator in a spatial audio system, with the early reflections rendered separately and
  the reverberator shared among sources. Plan 0020's structure (per-voice early taps, one
  shared tail) follows it.
- **Rocchesso, D. and Smith, J. O., "Circulant and Elliptic Feedback Delay Networks for
  Artificial Reverberation", IEEE Trans. Speech Audio Process. 5(1):51-63, 1997.** Shows an
  FDN with an orthogonal feedback matrix is a digital waveguide network with one scattering
  junction, and proposes circulant matrices for efficient mixing. Local copy in `local/`.
- **Schlecht, S. J. and Habets, E. A. P., "On Lossless Feedback Delay Networks", IEEE Trans.
  Signal Process. 65(6):1554-1564, 2017.** arXiv 1606.07729. The general losslessness
  condition: every irreducible component of the feedback matrix is diagonally similar to a
  unitary matrix. Householder and Hadamard matrices are in the known unitary class. Local
  copy in `local/`.
- **Smith, J. O., "Physical Audio Signal Processing", W3K Publishing, 2010, online.**
  <https://ccrma.stanford.edu/~jos/pasp/FDN_Reverberation.html>. The working reference for
  Plan 0020's FDN:
  - "Mean Free Path": `4V / S`, due to Sabine; set the average delay-line length to the mean
    free path over `c * T_s`, treated as a lower bound because real reflections are diffuse.
  - "Mode Density Requirement": the sum of the delay-line lengths `M` should satisfy
    `M >= 0.15 * T60 * f_s` (Schroeder and Logan's 0.15 modes per Hz for a 1 s decay); at
    50 kHz and 1 s, 7500 samples.
  - "Choice of Delay Lengths": mutually prime lengths maximise the period of the lossless
    prototype.
  - "Achieving Desired Reverberation Times": a per-sample attenuation of `10^(-3 / N60)` gives
    a 60 dB decay in `N60` samples, so a delay line of `M` samples carries the gain
    `10^(-3 M / (T60 f_s))`; the damping filter is a low-pass with gain at most 1, so high
    frequencies decay faster.
  - "Householder Feedback Matrix" and "Hadamard Matrix": lossless mixing in `2N` or
    `N log N` operations.
  Local copies of these pages in `local/`.
- **Dattorro, J., "Effect Design, Part 1: Reverberator and Other Filters", J. Audio Eng. Soc.
  45(9):660-684, 1997, and "Part 2: Delay-Line Modulation and Chorus", 45(10):764-788,
  1997.** Part 1 is the plate reverberator most products descend from (input diffusers, a
  figure-of-eight tank). Part 2 covers reading a delay line at a moving fractional position:
  linear interpolation's low-pass at half-sample offsets and the artefacts of modulation. Plan
  0020's gliding taps read their delay lines this way. Part 1 local copy in `local/`.

## Game and interactive systems

- **Raghuvanshi, N. and Snyder, J., "Parametric Wave Field Coding for Precomputed Sound
  Propagation", ACM Trans. Graph. 33(4), article 38, 2014.** DOI 10.1145/2601097.2601184. The
  method behind Microsoft's Project Acoustics (Project Triton): a wave simulation is baked
  per level, and each source-listener pair's impulse response is compressed to a few
  perceptual parameters (loudness, early decay, late reverberation time); whole maps fit in
  about 100 MB and decoding costs about 100 microseconds per source. The Microsoft Research
  project page lists Gears of War 4 and 5, Sea of Thieves and Borderlands 3 as shipped titles
  and states the bake runs on static visual geometry. **Raghuvanshi, "Dynamic Portal
  Occlusion for Precomputed Interactive Sound Propagation", arXiv 2107.11548, 2021** says
  plainly that precomputed wave methods remain limited to static scenes and extends them to
  doors only. A world that is dug every frame cannot bake; this is ADR-0015's rejected
  Alternative B. Local copy of the 2021 paper in `local/`.
- **Schissler, C. and Manocha, D., "GSound: Interactive Sound Propagation for Games", AES
  41st International Conference: Audio for Games, London, 2011.**
  <https://gamma.cs.unc.edu/GSOUND>. Runtime ray tracing from the listener finds the
  surfaces in view, image-source paths are validated by occlusion rays, paths are cached
  between frames, and the CPU share is bounded. **(Its late-reverberation treatment, a
  statistical estimate feeding an artificial reverberator, is remembered and not re-verified
  today: the PDF links returned errors.)** The listener-centred fan and the frame-to-frame
  continuity of Plan 0020's taps are the same idea at voxel scale.
- **Valve, Steam Audio (Apache 2.0), C API guide.**
  <https://valvesoftware.github.io/steam-audio/doc/capi/guide.html>. The "Reflections"
  section calls real-time reflection simulation a significant CPU cost and shows example
  settings of 4096 rays, 16 bounces, a 2.0 s impulse response and first-order Ambisonics;
  "Baking" shows 32768 rays and 64 bounces offline, looked up from probes at runtime;
  "Reverb" models listener-space reverb by placing a source at the listener, at a cost
  independent of the source count. The reflection effect also offers a parametric mode that
  drives an artificial reverberator from per-band decay times estimated from the simulation,
  and a hybrid that convolves the early part and switches to the parametric tail **(the
  parametric and hybrid modes are remembered and not re-verified today; the reference pages
  could not be opened)**. In our integer DDA at about 40 ns per voxel step, Steam Audio's
  real-time example is 4096 x 16 x about 100 steps, about 260 ms per update, which is why
  ADR-0015 rejects a traced impulse response. Local copy of the guide in `local/`.
- **Audiokinetic, "Image Source Approach to Dynamic Early Reflections" (blog) and the Wwise
  Reflect plug-in.** <https://www.audiokinetic.com/en/blog/image-source-approach-to-dynamic-early-reflections>.
  Reflect renders image sources as delay lines with filters and panning, positioned by the
  Spatial Audio geometry API; acoustic textures carry four absorption bands split at 250,
  1000 and 4000 Hz by default, and a reflective surface never amplifies. Early reflections
  panned from their own direction, separately from the source, is what Plan 0020's direction
  buses do.
- **Costello, S. (Valhalla DSP), "Getting Started With Reverb Design, Part 2: The Best
  Papers", 2021.** <https://valhalladsp.com/2021/09/22/getting-started-with-reverb-design-part-2-the-foundations/>.
  A practitioner's annotated index of the papers above, with the reminder that real rooms'
  echo density grows with time while Schroeder's parallel combs keep it constant.

## Distilled for Plan 0020

- Estimate the room from the listener: `V` and `S` from a ray fan (exact for the star-shaped
  region the listener sees), mean free path `4V / S`, Eyring for `T60`, Kuttruff's room
  constant for the level.
- Synthesize the tail with a Jot-style FDN: a lossless Householder mix, delays around the
  mean free path, per-line low-pass damping for the per-band `T60`, delay sum above
  `0.15 T60 f_s`.
- Render early reflections from their own direction (Wwise Reflect, GSound), and share the
  tail among sources (Jot 1997, Steam Audio's listener-space reverb).
- Measure with Schroeder integration and the normalized echo density, not by ear alone.
- Do not bake (Project Acoustics) and do not trace an impulse response at runtime (Steam
  Audio real-time): the first forbids digging, the second costs two orders of magnitude more
  than the budget.
