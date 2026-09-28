# Task 16 baseline: the approved seasons, mountainsides and rock forms (generator v3, renderer v3)

**Approved** on 2026-09-28 ([round 9 result](../round-09/RESULT.md)): "I explicitly approve!" This is the reference for the regression tests. It adds the seasons (the default, midsummer, is the approved look plus the round 8 changes), mountainsides with their own anatomy, and more varied rock forms. The earlier approved baselines, [baseline-25](../baseline-25/README.md) (v2) and [baseline-08](../baseline-08/README.md) (v1), are never regenerated or replaced.

- **Versions:** recipe schema 2, `nfc-lf-utf8/1`, `pigment-seed/1`, `GENERATOR_VERSION` 3, `RENDERER_VERSION` 3. Scenes are frozen by `checksums_are_frozen`.
- **Device:** NVIDIA GeForce RTX 4070 Ti, Vulkan, Linux, wgpu 30.0.1, driver 615.71.09. Image hashes hold for this device and driver only.
- **Settings:** everything at its default (season midsummer) unless the cell label says otherwise.
- **Contents and regeneration:** as for [baseline-25](../baseline-25/README.md#regenerate).
