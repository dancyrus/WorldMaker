# Plate physics model proposal (WO-0005)

Proposed replacement for the ad-hoc mechanics cataloged in
`plate-physics-audit.md`, under Dan's ruling (2026-08-27): behavior must
come from real physics and geology; no mechanic may exist for the sake of
happening. Status: **proposed, awaiting Dan's ruling.** This document
changes no code; measured defects it responds to are in
`docs/results/plate-physics-probe-*.json`.

Everything below respects the standing engineering rules: one master seed
with PCG sub-streams, fixed iteration order, no libm in the sim path,
integer or serial id-ordered reductions, and the keyframe-exact resume
contract. New per-cell state noted in §2 must join the keyframe encoding.

## 1. Force balance (per plate, per step)

Plates are inertialess: lithosphere motion is Stokes flow, so speed is the
instantaneous quotient of driving forces over resistances — **no momentum
term** — relaxed over the timescale on which the mantle re-equilibrates.

Driving forces:

- **Slab pull** — proportional to attached slab area, weighted by the age
  of the crust when it subducted (older = colder = denser = stronger
  pull): `F_slab = k_s · Σ_segments area · min(1, age_at_subduction / 80 My)`,
  summed over this plate's ledger segments (§2) that are still attached
  and younger than the detachment age. Basis: slab pull is the dominant
  plate driver, ~70–90% of the net driving force (Forsyth & Uyeda 1975;
  Lithgow-Bertelloni & Richards 1998); negative buoyancy scales with
  thermal age of the subducted lithosphere (half-space cooling).
- **Ridge push** — proportional to divergent boundary length:
  `F_ridge = k_r · L_ridge`, with `k_r` calibrated so a ridge-only plate
  is ~5–10× slower than a slab-attached one. Basis: ridge push is
  gravitational sliding off the ridge swell, ~2–4 × 10¹² N per meter of
  ridge vs ~10¹³ N/m available from slab pull (Turcotte & Schubert,
  *Geodynamics*).
- **Residual mantle traction** — a small uniform basal driving term
  `F_resid = k_m · A_plate`, standing in for large-scale mantle flow the
  model does not solve (§8). Because it scales with area exactly like
  basal drag, it yields a size-independent **residual drift**
  `v_resid = k_m / c_d` for a plate with no slab and no ridge; calibrate
  to ~0.3–1 cm/yr (slab-free continental plates on Earth: Eurasia,
  ~1 cm/yr).

Resistances:

- **Basal drag** — `R_drag = c_d · A_plate`, viscous coupling to the
  asthenosphere, opposing motion (Forsyth & Uyeda's drag terms).
- **Boundary resistance** — `R_bnd = c_c · Σ_contact strength(cell)` over
  continent–continent contact cells (strength from §4), plus
  `c_t · L_transform` for transform friction. Collision resistance is a
  *term in the balance*, not a multiplier that zeroes the target: a small
  contact slows a well-driven plate a little; a long strong contact can
  stall it. India–Asia (~15 → ~5 cm/yr across the collision, not to zero)
  is the calibration anchor (Molnar & Tapponnier 1975).

Speed update:

```
v_target = (F_slab + F_ridge + F_resid) / (R_drag + R_bnd)
v       += (DT_MY / TAU_MY) · (v_target − v),   TAU_MY = 30 (range 20–50)
```

τ = 30 My reflects how long plate speeds take to re-equilibrate after a
force change (India's slowdown played out over ~10–20 My; mantle
convective adjustment ~10⁷ yr). One symmetric timescale — the audit's
asymmetric up/down relaxation is gone. `SPEED_MAX` survives only as a
safety rail (~20 cm/yr, the fastest sustained motion known, Cretaceous
India). There is **no speed floor**: a plate whose forces vanish coasts
down to `v_resid` and that is correct behavior. The sub-cell
pending-rotation bank already keeps slow plates from freezing to the
grid, so no kinematic decree is needed.

Pole (direction) update — replaces the random walk: each driving element
contributes a torque direction — a subducting segment pulls the plate
toward its trench (`x_cell × n̂_trench`, n̂ the outward contact normal),
a ridge segment pushes away from the ridge — summed in fixed cell order
into a target rotation vector `ω_target`; the pole relaxes toward
`ω_target`'s direction with the same τ. Poles then wander exactly when
the plate's boundary makeup changes, which is what real pole shifts are
(e.g., Hawaiian–Emperor bend ≈ Pacific pole change when the Izanagi
ridge subducted). Determinism: no RNG in motion at all after setup.

## 2. Slab ledger

The record of what has been subducted — the physical memory that drives
§1 and, later, an "Overlay" map layer.

- **Per plate**: a list of slab segments `{area_cells, age_at_subduction_my,
  subducted_at_my, attached: bool}`, appended by the advection consumption
  branch (which already identifies the consumed plate and cell), merged
  per step (one segment per step is enough), fixed order.
- **Per cell**: two new fields, `slab_plate` (u16, NONE default) — whose
  slab lies beneath this cell — and `slab_since_my` (f32/u16): when it
  went under. Written at the trench cell when consumption happens,
  advected with the *overriding* plate thereafter (the slab hangs under
  the margin it subducted beneath). These two fields join the keyframe
  (16 → 20 B/cell at L7 ≈ 0.66 GB per 2 Gy — still inside the 1 GB
  budget; re-measure).
- **Slab detachment**: a segment older than `SLAB_DETACH_MY = 60` (range
  40–100) stops pulling (`attached = false`). Basis: slabs decouple from
  the surface plate once they founder into the lower mantle — upper-mantle
  transit at 5–8 cm/yr takes ~10–30 My, and post-collision slab breakoff
  is observed seismically ~10–20 My after continental arrival (Alps,
  Zagros; von Blanckenburg & Davies 1995). Detachment after a collision
  jam is what *ends* slab pull there — the India-style slowdown emerges
  instead of being scripted by `COLLISION_DAMP`.

## 3. Suture

A plate pair welds only when **all three** hold, sustained for
`SUTURE_AFTER_MY = 30` (kept from the audit — it matches real
collision-to-lock times, India–Asia ~50 → ~20 Ma):

1. **Contact extent**: continent-on-continent contact along ≥ 30% of the
   smaller plate's perimeter (Dan's ruling, WO-0005). Physical reading: a
   weld must span a substantial fraction of the margin, as in real
   terminal collisions (India's northern front is roughly a third of its
   perimeter); a pinprick contact must never weld two plates.
2. **Locked kinematics**: mean relative velocity across the contact below
   `SUTURE_LOCK_CMYR = 0.4` — the same value as the classification dead
   band, i.e., the contact is kinematically indistinguishable from plate
   interior. Basis: stable plate interiors deform at ≲ a few mm/yr; a
   boundary moving slower than that has ceased to be a boundary (Gordon
   1998, diffuse plate boundaries). Note this threshold is now measured
   against *emergent* speeds — the 1.2 cm/yr value existed only to sit
   above the artificial jam-creep floor, which no longer exists.
3. **Ocean closed**: no oceanic crust remaining on either side of the
   contact — every cell within 2 rings of the contact on both plates is
   continental. Basis: suturing is by definition the terminal act of the
   Wilson cycle after the intervening ocean is consumed (Wilson 1966);
   while ocean remains, subduction continues and the boundary lives.

Action on suture: the pair merges (smaller into larger), and every
contact cell records `suture_at_my = now` — the suture scar is data, not
just an event count. The scar feeds the strength field (§4) and is the
preferred path for future rifting (§5), closing the real Wilson loop:
oceans reopen along old sutures (the Atlantic opened along the
Caledonian–Variscan welds). No plate-count floor: if the world's
continents genuinely weld into one, that is a supercontinent, and §5
is what breaks it — not a counter.

## 4. Lithosphere strength field

Per cell, from fields the sim already carries plus the suture age of §3:

```
S(c) = S_type(c) · g_age(c) · g_suture(c)
S_type   = 1.0 ocean, 0.6 continent            (continents are weaker)
g_age    = clamp(age_ref(c) / 500 My, 0.2, 2.0)
           age_ref = crust_age (ocean) or min(crust_age, orogeny_age) (continent)
g_suture = 0.3 + 0.7 · min(1, (t − suture_at_my) / 300 My)
thin/thick penalty: multiply by clamp(thickness / 35 km, 0.5, 1.0) on
continents — over-thickened hot orogens (>50 km) also take a 0.7 factor
while orogeny_age < 50 My (hot crust is weak).
```

Old, thick, cold crust is strong: a 2,500 My craton scores ~2.0; a fresh
arc terrane ~0.2; a 100 My-old suture ~0.5. Basis: lithosphere strength
grows with thermal age (cooling half-space → thicker mechanical
lithosphere); cratons are the strongest, coldest lithosphere on Earth and
have survived > 2.5 Gy; suture zones and young orogens are the weakest
links and localize later deformation for hundreds of My (Vauchez,
Barruol & Tommasi 1997, "Why do continents break up parallel to ancient
orogenic belts?"). Exact coefficients are calibration targets, not
gospel; what is load-bearing is the *ordering* — craton > old ocean >
young continent > fresh suture/rift.

## 5. Rifting

A rift may start **only** where one of three real drivers exists, and it
follows the path of least strength. There is no random breakup and no
plate-count trigger of any kind.

Drivers (checked in fixed order, deterministically):

- **Plume under continent**: a hotspot (§ existing fixed mantle points)
  that has sat under continental crust for ≥ 20 My. Basis: plume-assisted
  rifting — Afar plume / East African Rift, CAMP plume / Pangea breakup
  (Gurnis 1988 adds the insulation argument: supercontinents trap heat,
  so plumes preferentially fire beneath them — which makes supercontinent
  breakup *emerge* from this driver with no area quota).
- **Back-arc extension**: a band 200–600 km inboard of a subduction zone
  whose slab segment is old (age_at_subduction > 60 My → steep rollback).
  Basis: trench rollback stretches the overriding plate — Sea of Japan,
  Aegean (Uyeda & Kanamori 1979 subduction modes).
- **Opposing slab pull**: one plate with subducting segments on two
  roughly opposite sides (their pull directions ≥ 120° apart), putting
  its interior in net tension. Basis: divergent slab loads rift the plate
  between them (proto-Atlantic configurations; stress-field rifting).

Nucleation and path: the rift nucleates at the driver's location (plume
cell; weakest cell of the back-arc band; the tension axis' weakest
continental cell) and grows a few cells per step by walking the
neighbor of least strength `S` (§4) — deterministic, id-ordered
tie-breaks — preferring old sutures because `g_suture` makes them
weakest. A finite propagation speed (~50–100 km/My: the East African
Rift lengthened over ~10–20 My) replaces the instantaneous great-circle
cut. Once a rift path exists, the existing maturation pipeline (thin at
0.2 km/My past onset, oceanize below 25 km — McKenzie 1978 stretching)
is kept as-is; when the path oceanizes across the plate, the plate
splits along it and the halves' motions come from §1 (their new ridge
supplies ridge push) — never from an imposed `BREAKUP_RIFT_SPEED`.

## 6. Microplates

A new small plate may be created only by:

- **Trench-trapped slice**: plate area isolated between an active trench
  and a ridge that has been consumed (Farallon → Juan de Fuca remnant).
- **Back-arc basin opening**: a §5 back-arc rift oceanizing detaches the
  arc sliver as its own plate (Philippine Sea style).
- **Ridge jump**: a §5 rift re-nucleating on the other side of a
  microcontinent transfers it (Easter / Juan Fernández microplates;
  Jan Mayen microcontinent).

Cells orphaned by ownership updates are **never** a microplate; they are
reassigned to the surrounding plate by §7. A microplate is born with a
slab ledger and force balance like any plate and lives or dies by them.

## 7. Exclave fix

**Invariant: every alive plate is one connected region on the grid, every
step.** Enforced in two layers:

- **Cause removal**: the continent–continent jam no longer freezes single
  cells in place (rigid plates cannot shed frozen cells — audit row 13);
  instead the overlap cell resolves to the plate whose motion §1 has
  already slowed, and the *plates* stop, staying rigid. Suture (§3) and
  rift split (§5) reassign only contiguous regions by construction (the
  rift path is a connected walk; the suture merge is a whole plate).
- **Backstop pass**: immediately after `advect()`'s ownership scatter
  (before `classify_boundaries()`, so classification and stats only ever
  see clean plates), a serial connected-components sweep in cell-id
  order: for each plate, keep the largest component (tie → the component
  containing the lowest cell id); reassign every other fragment's cells
  to the neighboring plate sharing the longest border with that fragment
  (tie → lowest plate id). O(n) BFS per step — measured cost of the same
  sweep in the WO-0005 probe is negligible at L6. The pass logs a
  per-run count; the acceptance target (§9) is that the *backstop*
  fires only for advection seam noise (a few cells), because the causes
  are gone.

## 8. Gaps — real processes this model still lacks

Listed to bound the claim; **no mechanic is added for any of these.**

- **Rock type / lithology.** On Earth, lithology sets erodibility and
  strength contrasts (granite vs basalt vs sediment). Would take a
  per-cell lithology field written by every crust-forming event.
- **Solved mantle convection.** Basal tractions are organized by real
  convection cells that drive and reorganize plates. Would take a coarse
  spherical convection solve coupled both ways — a project-scale addition
  (`F_resid` is its one-number stand-in).
- **Flat-slab subduction.** Shallow slabs shut off arcs and push orogeny
  inland (Laramide Rockies). Would take per-trench-segment slab dip state
  fed by slab age and overriding-plate motion.
- **Terrane accretion.** Island arcs, plateaus, and microcontinents dock
  onto margins as discrete exotic blocks (Wrangellia, most of Alaska).
  Would take sub-plate block identity surviving consumption, docking
  instead of vanishing.
- **Obduction / ophiolites.** Slivers of ocean crust thrust onto
  continents at closure (Oman). Would take an overlap special case
  preserving oceanic material above the suture.
- **True polar wander.** The whole lithosphere rotates relative to the
  spin axis when mass distribution shifts. Would take an inertia-tensor
  step rotating everything, including hotspot frames.
- **Large igneous provinces.** Plume *heads* deliver flood basalts at
  rift onset (Deccan, CAMP, Siberian Traps). Would take a one-shot
  emplacement event distinct from steady hotspot tails.
- **Sediment transport and passive-margin loading.** Margins subside
  under sediment wedges; trenches fill. Belongs to the erosion/climate
  phases, not tectonics.
- **Diffuse plate boundaries.** Some "boundaries" are 1,000 km-wide
  deforming zones (central Indian Ocean). The rigid-plate assumption is
  kept deliberately.

## 9. Acceptance metrics for the implementation session

All measured by the WO-0005 probe (extended where noted) at L6, 2 Gy,
seeds `cyrus` and 42, defaults — committed as results JSON. Earth
justifications inline. Targets are ranges, not points; the calibration
knobs are the §1 coefficients.

1. **Plate count drift**: alive count stays in 6–25 for the whole run
   *with no clamp in the code*, varies over time (std-dev ≥ 1.5 across
   samples), and is never pinned at one value for > 500 My. Earth: 7–8
   major plates plus minors, with the census changing through the Wilson
   cycle. (Probe today: pinned at exactly the floor, 6–7, from 300 My on.)
2. **Suture frequency**: 2–10 sutures per Gy, *every one* satisfying the
   three §3 conditions in the event log. Earth: a handful of major
   continental welds per few hundred My (Variscan, Alleghanian, Uralian,
   Himalayan). (Probe today: 17 per Gy, all fired by the slowness-only
   rule.)
3. **Breakup / new-ocean frequency**: 2–8 rift-to-oceanization splits per
   Gy, each attributed to a §5 driver in the event log; **zero** from any
   other code path (the gridlock breaker and area quota no longer exist).
   Earth: Atlantic, Indian, Red Sea-scale openings a few per few hundred
   My. (Probe today: 15 per Gy, two-thirds from the gridlock breaker.)
4. **Largest-plate share**: < 45% of the sphere except during a
   supercontinent epoch — defined as > 1/3 of continental crust in one
   plate — which may occur 0–2 times per 2 Gy and must disperse within
   100–300 My of forming. Earth: Pangea held for ~120 My; today's largest
   plate (Pacific) is ~20% of the sphere. (Probe today: 87% at 200 My,
   peak 97%.)
5. **Zero exclaves**: every sampled keyframe has zero multi-component
   plates (§7 invariant); backstop reassignments ≤ 10 cells per 100 My.
   (Probe today: fragmented plates in 20 of 21 samples.)
6. **Collisions build relief**: ≥ 80% of continent–continent contact
   zones that persist ≥ 20 My reach crust thickness > 45 km somewhere
   along the zone. Earth: every major collision has built an orogen —
   Himalaya/Tibet ~70 km, Alps ~55 km, Zagros ~45–55 km.
7. **Speeds are force-ranked**: run mean speed 2–6 cm/yr (Earth mean
   ~4–5 cm/yr, DeMets et al. 2010 MORVEL); plates with attached slabs
   average ≥ 2× the speed of slab-free plates (Forsyth & Uyeda 1975's
   trench-connectivity correlation — the single strongest observational
   fact about plate driving forces); slab-free continental plates drift
   at 0.3–2 cm/yr, and none of it comes from a floor constant.
8. **Liveliness (kept from WO-0003)**: no alive plate holds < 0.05
   deg/My for > 200 My *unless* it is in a §3-qualifying collision or is
   sutured — now as an emergent property, since no clamp can hide a
   violation.

## Open questions for Dan (before implementation)

1. **Slab-pull memory on plate death**: when a plate is fully consumed,
   its remaining slab ledger currently dies with it. Keep pulling the
   overriding plate for `SLAB_DETACH_MY` afterward (physically yes —
   slabs keep sinking), or drop it for simplicity?
2. **Keyframe growth**: the slab ledger's two per-cell fields raise
   keyframes 16 → 20 B/cell (~0.66 GB per 2 Gy at L7, still under the
   1 GB budget, re-measured at implementation). Acceptable, or should the
   Overlay layer be derived-on-demand instead of stored?
3. **Supercontinent cadence**: with breakup driven only by plumes,
   back-arcs, and opposing slabs, supercontinent lifetime will be an
   *outcome*. If a run's continents weld and no plume happens to sit
   under them for, say, 400 My, that is physically honest but may read as
   a boring epoch. Is a long quiet supercontinent acceptable world
   history, or should hotspot count/placement be biased under large
   continents (the mantle-insulation argument justifies a mild bias)?
4. **Microplate budget**: §6 can push the plate census above today's 24
   display palette in extreme runs. Cap microplate creation (physical
   rationale: small plates are quickly consumed anyway), or extend the
   palette?
