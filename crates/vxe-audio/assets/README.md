# vxe-audio assets

## `sadie2_d2_48k.hrir`

Head-related impulse responses in the engine's pack format (see `examples/hrir_pack.rs` for the
layout), derived from the **SADIE II Database**, University of York.

- **Source:** SADIE II Database, Zenodo record 10886409, DOI
  [10.5281/zenodo.10886409](https://doi.org/10.5281/zenodo.10886409), file `D2_HRIR_SOFA.zip`
  (md5 `f555491d1b6b1a108bb4d26b63ad84a4`), member `D2_48K_24bit_256tap_FIR_SOFA.sofa`.
- **Subject:** D2 (KEMAR dummy head), 48 kHz, 8802 measured directions.
- **Modifications:** every 3rd azimuth of each elevation ring kept, plus both poles (2950
  directions); every response truncated to the same 128-tap window, starting 8 samples before
  the earliest onset over all kept directions and both ears (sample 62 of 256; an ear's onset
  is its first sample reaching a tenth of the louder ear's peak for that direction); stored as
  little-endian `f32`. The packer prints the window start and the least fraction of an ear's
  energy the window keeps (0.9714). Regenerate with
  `cargo run -p vxe-audio --example hrir_pack -- --azimuth-stride 3 <SOFA path> assets/sadie2_d2_48k.hrir`.
- **Licence:** Apache License 2.0; the copy is `LICENSE-SADIE-II.txt` (the record's
  `LICENSE.txt`). It covers this file only; the rest of the repository keeps its own licence.

### Attribution

As SADIE II asks, the original dataset is referenced here:

> SADIE II Database. Copyright 2018, University of York. All data recorded and developed at The
> Audio Lab, Department of Electronic Engineering, University of York, UK, by Cal Armstrong,
> Lewis Thresh and Gavin Kearney. Licensed under the Apache License, Version 2.0.
> <https://www.york.ac.uk/sadie-project/database.html>

For academic work, SADIE II asks that the following Open Access paper be cited: C. Armstrong,
L. Thresh, D. Murphy and G. Kearney, "A Perceptual Evaluation of Individual and Non-Individual
HRTFs: A Case Study of the SADIE II Database", *Applied Sciences* 8(11), 2029, 2018, DOI
[10.3390/app8112029](https://doi.org/10.3390/app8112029).
