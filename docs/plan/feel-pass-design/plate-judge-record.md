# WO-0003 Fix 2 — plate-generator judge record (M2.6, BINDING)

Panel run 2026-08-25 on Daniels-MacBook-Air. Inputs: committed metrics JSON
(`plategen-feelpass-*.json` / M1 incumbent rows) + the 20 L7 PNGs in
`target/plate-panel/`. Three lens judges (subagents, repo rule 4) + this
synthesizer. Protocol: d2-fix2-design.md §6 (P2/P3); gate contract §4.

## 1. Judge scores

Per-seed scores at L7, seeds in order 42 / cyrus / 7 / 1002 / 271828.

### Lens 1 — size hierarchy
| generator | 42 | cyrus | 7 | 1002 | 271828 | overall |
|---|---|---|---|---|---|---|
| incumbent | 1 | 1 | 1 | 2 | 1 | **1/10** |
| warped | 4 | 4 | 3 | 4 | 4 | **4/10** |
| growth | 8 | 7 | 7.5 | 7 | 7 | **7.5/10** |
| hybrid | 9 | 9 | 7.5 | 7.5 | 8.5 | **8.5/10** |

### Lens 2 — wandering boundaries
| generator | 42 | cyrus | 7 | 1002 | 271828 | overall |
|---|---|---|---|---|---|---|
| incumbent | 2 | 2 | 2 | 2 | 2.5 | **2/10** |
| warped | 4 | 5 | 4.5 | 4 | 4.5 | **4/10** |
| growth | 6.5 | 8 | 7.5 | 6 | 6 | **6.5/10** |
| hybrid | 7.5 | 7 | 8.5 | 7.5 | 8 | **8/10** |

### Lens 3 — symmetry / cross-seed fingerprint / artifacts
| generator | 42 | cyrus | 7 | 1002 | 271828 | overall |
|---|---|---|---|---|---|---|
| incumbent | 1 | 2 | 1 | 2 | 1 | **1/10** |
| warped | 4 | 4 | 4 | 3 | 4 | **4/10** |
| growth | 7 | 6 | 7.5 | 5.5 | 4.5 | **6/10** |
| hybrid | 7.5 | 6.5 | 8 | 7.5 | 8 | **7.5/10** |

Overall totals (sum of lens overalls): incumbent 4, warped 12, growth 20,
**hybrid 24**. Lens verdicts: HYBRID, HYBRID, HYBRID — unanimous.

## 2. Winner — HYBRID (unanimous, 3–0)

- **Size hierarchy:** best area CVs on the panel (triple: 0.86 / 0.72 /
  0.94); a clear single dominant plate in 3/5 seeds; the small tier reads as
  intentional attached minors (back-arc style), never confetti. Growth's
  tail degenerates in 2–3 of 5 seeds (enclave dot L7:271828, enclave loop
  L7:1002, near-pinch-off cyrus) and its dominance splits into two co-equal
  polar giants.
- **Boundaries:** hybrid's wander has the right spectrum — long sweeping
  arcs with kinks and promontories at continental scale (seed 7 judged best
  map on the panel); growth trends toward isotropic blobs with
  constant-amplitude wobble.
- **Symmetry/artifacts:** hybrid shows the richest silhouette vocabulary and
  the weakest cross-seed fingerprint; growth's polar-supergiant-plus-potato-
  belt template is recognizable in 3/5 seeds, and its 271828 enclave is a
  near-regular hexagon speck — a visible defect.
- **Enclaves (synthesizer adjudication of the judges' disagreement):** the
  symmetry judge flagged hybrid seedcyrus's teal oval (~x 819–934, y 364–435
  in the L7 PNG) as a "confirmed enclave"; the sizes judge said hybrid has
  none. Pixel-level re-inspection by the synthesizer OVERTURNS the flag: the
  teal region is a single component whose east margin contacts the blue
  plate directly (60 four-neighbour contact samples onto bright blue
  (0,130,200), with the teal–blue boundary stroke rendered in blue's
  darkened shade (0,58,90) — the renderer only darkens pixels at genuine
  plate-id changes, so this is cell-graph adjacency, not projection
  coincidence). It is a minor plate wedged at the crimson/blue junction,
  touching two plates. Likewise hybrid L7:1002's lavender plate borders
  three plates (crimson 207 / purple 85 / blue 26 contact samples) — nearly
  enclosed, not enclosed. **Hybrid has zero true enclaves across the panel;
  growth has two (implementer-confirmed, L7 seeds 1002 and 271828).** This
  removes the symmetry judge's caveat and strengthens the verdict.

## 3. FINAL gate values (for `plategen_gate_tests.rs`, wired at M3)

Gate triple: L7 seed 42 + L6 seed 7 + L6 seed cyrus (pinned, d2 §4).

```
GATE_CV        = 0.50
GATE_SINUOSITY = 1.18
```

**Derivation (contract: GATE = max(provisional, incumbent_best_on_triple +
margin), ≤ winner_worst_on_triple − margin; margins 0.05 CV / 0.02 sin):**

- **CV:** incumbent best on triple 0.1361 (L7:42) → floor max(0.5, 0.1861)
  = 0.50. Hybrid worst on triple 0.7210 (L6:7) → ceiling 0.6710. Band
  [0.50, 0.6710]; the formula value 0.50 stands. Winner margin on the
  triple: ≥ 0.2210.
- **Sinuosity:** incumbent best on triple 1.1465 (L7:42) → floor max(1.15,
  1.1665) = 1.1665. Hybrid worst on triple 1.2295 (L7:42) → ceiling 1.2095.
  Band clean: [1.1665, 1.2095]. **Set 1.18, above the formula floor, with
  logged reasoning:** the incumbent's best sinuosity across ALL 10 (seed,
  level) pairs is 1.1545 (L6:271828), and the implementer flag establishes
  ~1.10–1.15 as the hex-grid zigzag baseline — i.e. values near 1.1665 are
  barely distinguishable from a straight boundary on this grid. 1.18 clears
  every incumbent measurement anywhere by ≥ 0.0255 (not just the triple's),
  while leaving hybrid ≥ 0.0495 margin at its triple worst. Metrics are
  bit-deterministic, so both margins protect future retuning, not machines.
- **Crossing resolution (logged per contract):** the formula CROSSES for
  growth only (growth worst-on-triple 1.1828 → ceiling 1.1628 < floor
  1.1665). Selecting hybrid dissolves the crossing — no gate adjustment is
  needed to accommodate the winner. The crossing is itself corroborating
  evidence against growth: its gate-triple wander is too close to the hex-
  zigzag baseline to be separable from the incumbent at contract margins.
- **Strict exclusion check:** every incumbent score on all 10 pairs fails
  BOTH gates independently (CV max 0.1361 < 0.50; sinuosity max 1.1545 <
  1.18). Warped also fails CV everywhere (max 0.4381). Hybrid clears both
  gates on the gate triple with the margins above; its CV clears 0.50 on
  all 10 pairs (min 0.5982). Off-triple sinuosity is panel evidence, not
  gated: hybrid's off-triple min is 1.1735 (L7:7), below 1.18 but above the
  formula floor 1.1665 — acceptable, since the CI gate asserts only the
  triple and CV excludes the incumbent everywhere on its own.

The gate-test doc comment must quote verbatim: incumbent triple CV 0.1361 /
0.0789 / 0.0878 and sinuosity 1.1465 / 1.1232 / 1.1132; hybrid triple CV
0.8564 / 0.7210 / 0.9406 and sinuosity 1.2295 / 1.2296 / 1.2853.

## 4. Conditions on the winner (binding for M3)

1. **No enclave fix is required before wiring** — hybrid has no true enclave
   on any competition (seed, level) pair (§2 adjudication). However, M3 MUST
   run a one-off cell-graph connectivity check of hybrid on all 10 pairs
   (each plate id = exactly one connected component over CSR neighbors). If
   any pair unexpectedly fails, fix criterion: deterministically reassign
   each minority component to the neighboring plate sharing the longest
   boundary (arc_len3; ties → lowest plate id), then re-run metrics and
   re-verify §3 gates before the golden regen. Optionally promote this
   connectivity check into the gate test on the triple (cheap; directly
   encodes the enclave criterion that sank growth) — recommended, not
   required.
2. **Pole-row striping** (horizontal streaks near top/bottom edges on 3/5
   hybrid seeds, also present on growth): shared across generators, judged a
   projection/render effect of the equirect panel raster. Non-blocking;
   M3 gives the panel renderer's polar rows a quick look and notes the
   finding in the decision log. Not a generator defect.
3. **hybrid L6 seed7 "36 boundary segments" implementer flag:** a metrics
   count, not a defect; no action.
4. Hybrid anneal pins primaries only; 3-factor step cost casts all factors
   to u64 (F13) — unchanged, restated as binding.

## 5. Losers to delete at M3 (P4, d2 §2/§6)

Delete `Incumbent`, `Warped`, `Growth` impls + `trait PlateGenerator` +
`all_generators()` + `score_generators`; `plate_gen` demoted to private;
`render_plate_maps` kept, reduced to rendering `SimState::setup` output.
Curated judge PNGs → `docs/media/feel-pass/plate-panel/` with this record.

## Re-judging addendum — retuned hybrid C9

Re-panel run 2026-08-26 on Daniels-MacBook-Air, after the ruled hybrid failed
the committed 2 Gy stability gate and was retuned (commit 394ecb4: C9 giant
band 0.22–0.25, smallest share pinned 0.03, pure ladder mid-tail). Same three
lenses, retuned hybrid2 vs the §2 original, L7 PNGs in `target/plate-panel/`
(`plates-hybrid2-*`), metrics in
`docs/results/plategen-feelpass-Daniels-MacBook-Air.json` (`hybrid2_*` rows).

### A1. Lens scores (hybrid2, per seed 42 / cyrus / 7 / 1002 / 271828)

| lens | 42 | cyrus | 7 | 1002 | 271828 | verdict |
|---|---|---|---|---|---|---|
| size hierarchy | 9 | 8 | 7 | 9 | 8 | **CONFIRM** |
| wandering boundaries | 8 | 8.5 | 8 | 7 | 7.5 | **CONFIRM** |
| symmetry / artifacts | 9 | 9 | 8 | 6 | 9 | **CONFIRM** |

All three lenses: retuned ≥ original net (42 and 1002 improved on sizes,
cyrus's original teal near-enclave now visibly three-neighboured, no seed
regressed decisively), and still decisively better than the incumbent.

### A2. Seed 1002 enclave dispute — adjudicated, flag OVERTURNED

The symmetry judge scored 1002 a 6 claiming the lavender plate is a full
enclave inside crimson ("crimson channel severs its contact with blue");
the boundaries judge saw a retained blue contact arc. Synthesizer pixel
re-inspection of `plates-hybrid2-L7-seed1002.png` (same method as §2):
lavender–blue contact arc at x≈402–414, y≈286–317; 75 strict stroke-only-gap
samples and 99 first-bright-plate samples reach blue; the gap stroke renders
in darkened lavender (99,85,114) and darkened blue (0,58,90) — the renderer's
plate-id-change shades — vs a single stray darkened-crimson pixel. Lavender
borders crimson AND blue: nearly enclosed, not enclosed. **Hybrid2 has zero
true enclaves across the panel**, matching the metrics run (all 10 pairs
single-component). The 1002 score's regression premise is void; the lens's
CONFIRM stands a fortiori.

### A3. FINAL ruling

**Retuned hybrid C9 (hybrid2) is CONFIRMED as the generator M3 wires.**
Unanimous 3–0 CONFIRM; the only claimed visual regression (A2) is overturned
on pixel evidence; stability seed 42 passes at 4.1% drift (gate ≤ 5%) with
alive plates 6–12 and a live breakup cycle. No conflict to report.

### A4. FINAL gate values — unchanged

```
GATE_CV        = 0.50
GATE_SINUOSITY = 1.18
```

Retuned P3 bands: CV [0.50, 0.7713], sinuosity [1.1665, 1.2148]; both ruled
values sit inside. Reasoning: the incumbent-side floors are untouched by the
retune (0.50 formula CV floor; 1.1665 sinuosity floor, with 1.18 still
clearing every incumbent measurement on all 10 pairs by ≥ 0.0255). Winner-side
margins GREW: CV triple worst 0.8213 (margin 0.3213, was 0.2210), sinuosity
triple worst 1.2348 (margin 0.0548, was 0.0495), and hybrid2's off-triple
sinuosity min 1.2023 now clears 1.18 outright (original dipped to 1.1735).
No judge evidence argues for moving either gate. §3's strict-exclusion result
is unchanged. **M3 must update the gate-test doc comment to quote the hybrid2
triple verbatim: CV 0.8280 / 0.8213 / 0.9152 and sinuosity 1.2348 / 1.2831 /
1.2927 (order L7:42, L6:7, L6:cyrus), replacing §3's original-hybrid figures.**

### A5. Stability drift gate — orchestrator ruling, applied not debated

The committed 2 Gy stability gate stays seed-42-only for this order, exactly
as Phase 1 defined it — the multi-seed drift numbers are recorded-not-gated,
flagged as an open question for Phase 2.

### A6. Realized-vs-target shares

Realized smallest shares dipping to ~0.9% against the pinned 0.03 target
band match competition-era behavior (targets are band-pinned, realized
shares are not — §2's panel was judged under the same regime). Non-blocking;
the minors still render as legible plates (sizes lens, all five seeds).
Realized giant is reliably 22–26% of the sphere.

### A7. Notes for M3

- §4 conditions stand unchanged, now applied to hybrid2. The symmetry judge
  re-recommends promoting an automated enclave check (plate with exactly one
  distinct neighbor) into the gate suite alongside §4.1's connectivity
  check — still recommended, not required.
- Cross-seed fingerprint: pinning smallest share at 0.03 makes the smallest
  plates slightly more uniform across seeds; mild, present in the original,
  non-blocking.
- This addendum is committed by Track B's M3 run, not by the re-judging panel.
