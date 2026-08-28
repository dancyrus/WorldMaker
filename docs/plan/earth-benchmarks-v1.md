# Earth benchmarks v1

Quantitative calibration and test targets for WorldMaker, drawn from the academic literature. Every row carries a source (author, year) and, where the literature gives one, an uncertainty or range. Values are consensus figures from reviews and primary papers, not simulator outputs. Contested items are flagged in-line. Read the confidence notes at the end before hard-coding any value as a pass/fail gate.

Compiled 2026-08-28.

---

## 1. Supercontinent cycle

### Table 1.1 — Assembly and breakup ages

| Supercontinent | Assembly (peak) | Breakup onset | Uncertainty / notes | Source |
|---|---|---|---|---|
| Kenorland (≈ Superia/Sclavia) | ~2.7–2.5 Ga | ~2.5–2.1 Ga | Existence as one mass CONTESTED — Bleeker argues two+ independent blocks | Bradley 2011; Bleeker 2003 |
| Nuna / Columbia | ~1.9–1.6 Ga (core ~1.7 Ga) | ~1.5–1.3 Ga | Naming and configuration debated | Bradley 2011; Rogers & Santosh 2002; Zhang et al. 2012 |
| Rodinia | ~1.3–0.9 Ga (peak ~1.1–1.0 Ga) | ~0.83–0.72 Ga | Breakup diachronous 825–720 Ma | Bradley 2011; Li et al. 2008 |
| Pannotia (Greater Gondwana) | ~0.65–0.57 Ga | ~0.57–0.54 Ga | CONTESTED — many deny it was ever a true supercontinent; assembly and breakup overlap | Powell 1995; Scotese 2009; Nance et al. 2014 |
| Pangea | ~0.32–0.25 Ga (peak ~300–270 Ma) | ~0.20–0.18 Ga | Best-constrained; final assembly ~320 Ma, Central Atlantic ~200 Ma | Bradley 2011; Nance & Murphy 2013 |

Note: pre-Pangea ages are paleomagnetically model-dependent; real uncertainty on the older events is often ±100 Myr or more. Kenorland and Pannotia are the two whose existence as single supercontinents is actively disputed.

### Table 1.2 — Lifetime and cycle period

| Quantity | Value | Range | Source |
|---|---|---|---|
| Supercontinent lifetime (assembly-to-breakup) | ~100–200 Myr | Pangea ~100–150 Myr | Nance et al. 2014; Bradley 2011 |
| Full supercontinent cycle period | ~500–700 Myr (commonly ~600 Myr) | 300–800 Myr published spread | Nance & Murphy 2013 |
| Wilson cycle (single ocean open-and-close) | ~400–500 Myr | 300–500 Myr | Wilson 1966; Nance et al. 2014 |

Note: the "period" is a statistical mean over 4–5 poorly dated events. Whether the cycle has lengthened or shortened through time is itself contested (Bradley 2011).

### Table 1.3 — Number of major plates through time

| Epoch | Value | Notes | Source |
|---|---|---|---|
| Present — "major" plates | 7 (8 incl. Nazca-scale) | Conventional textbook count | standard usage; Bird 2003 |
| Present — 14 largest plates | cover ~94.6% of surface | 52 plates total in PB2002 model (38 small) | Bird 2003 |
| Past epochs | not well constrained | No reliable census exists; deep-time counts are speculative | Bradley 2011 (discussion) |

---

## 2. Sea level

### Table 2.1 — Phanerozoic eustatic highstands and lowstands

| Interval / event | Value (m rel. present) | Range | Source |
|---|---|---|---|
| Cretaceous peak highstand, earliest Turonian (~90 Ma) — Exxon curve | ~+250 m | +250 to +280 m | Haq, Hardenbol & Vail 1987 |
| Cretaceous peak highstand — Exxon revised | ~+240 to +250 m | long-term amplitude ~165–175 m | Haq 2014 |
| Late Ordovician (early Katian) peak — Exxon | ~+225 m | events up to ~125 m each | Haq & Schutter 2008 |
| Late Cretaceous peak (~80 Ma) — backstripping | +100 m | ±50 m | Miller et al. 2005 |
| Amplitude discrepancy Exxon vs backstripping | Exxon ≥2.5× too high | Late-K: Miller ~100 m vs Exxon ~250–320 m | Miller et al. 2005 |

Note: the disagreement is methodological. Haq/Exxon curves come from seismic coastal-onlap (uncalibrated in amplitude); Miller uses backstripping cross-checked with δ18O/Mg-Ca. Miller amplitudes are the more defensible; Exxon absolute heights are widely held to be exaggerated. For a mass-based sea-level model, target the Miller amplitudes.

### Table 2.2 — Last Glacial Maximum lowstand (~21 ka)

| Value (m below present) | Range | Source |
|---|---|---|
| −134 m (ice-volume-equivalent minimum, 29–21 ka) | ±~5 m | Lambeck et al. 2014 |
| ~−130 m (peak lowstand) | field/model cluster −125 to −134 m | Clark et al.; Peltier ICE-6G |

### Table 2.3 — Present ice-sheet sea-level equivalents (SLE)

| Reservoir | Value (m SLE) | Source |
|---|---|---|
| Greenland Ice Sheet | 7.42 m | Morlighem et al. 2017; IPCC AR6 2021 |
| Antarctic Ice Sheet (total) | 57.9 m | Fretwell et al. 2013 (Bedmap2); Morlighem 2020 |
| — East Antarctica | 53.3 m | Fretwell et al. 2013 |
| — West Antarctica | 4.3 m | Fretwell et al. 2013 |
| Glaciers & ice caps (outside GrIS/AIS) | 0.32 m | Farinotti et al. 2019; IPCC AR6 |

Note: total present land-ice SLE ≈ 65 m.

---

## 3. Plate kinematics

### Table 3.1 — Present-day absolute plate speeds (NNR-MORVEL56, area-weighted RMS per plate)

| Plate | Speed (mm/yr) | Notes | Source |
|---|---|---|---|
| Cocos | 74.6 | fastest | Argus, Gordon & DeMets 2011 |
| Nazca | 70.0 | | Argus et al. 2011 |
| Pacific | 65.5 | | Argus et al. 2011 |
| Australia | 63.1 | | Argus et al. 2011 |
| Philippine Sea | 52.6 | | Argus et al. 2011 |
| Nubia (Africa) | 29.2 | | Argus et al. 2011 |
| Eurasia | 22.8 | | Argus et al. 2011 |
| North America | 19.8 | | Argus et al. 2011 |
| Antarctica | 15.2 | | Argus et al. 2011 |
| South America | 10.6 | slowest major plate | Argus et al. 2011 |
| Global lithosphere RMS | 43.9 | whole-lithosphere RMS | Argus et al. 2011 |

Note: kinematics from MORVEL (DeMets, Gordon & Argus 2010); NNR-MORVEL56 adds 31 small plates to the no-net-rotation frame.

### Table 3.2 — India rapid drift and post-collision slowdown

| Quantity | Value (mm/yr) | Source |
|---|---|---|
| Peak drift speed, ~67–65 Ma | ~180–200 (plume-assisted) | Kumar et al. 2007 |
| Pre-collision speed (~55–50 Ma) | ~150 | Copley, Avouac & Royer 2010 |
| Post-collision speed (after ~50 Ma) | ~40–50 (≈3× slowdown) | Copley et al. 2010 |
| Present India–Asia convergence | ~40 | van Hinsbergen et al. 2011 |

### Table 3.3 — Slab-attached vs slab-free plates

| Class | Representative speed (mm/yr) | Source |
|---|---|---|
| Large subducting (slab) margins — Pacific, Cocos, Nazca, Philippine, Australia, India | ~60–100 (slab-pull dominated) | Forsyth & Uyeda 1975 |
| Lacking significant slab margins — Eurasia, Antarctica, Africa, Americas | <~20–30 (ridge-push / drag) | Forsyth & Uyeda 1975 |
| Driver: velocity rises with fraction of margin that is subducting trench | slab-pull > ridge-push | Conrad & Lithgow-Bertelloni 2002; Zahirovic et al. 2015 |

---

## 4. Orogens

### Table 4.1 — Orogen dimensions, structure, and mechanism (one row each)

| Orogen | Width (km) | Length (km) | Peak elev. (m) | Crust thickness (km, root) | Main orogeny (Ma) | Mechanism | Source |
|---|---|---|---|---|---|---|---|
| Himalaya–Tibet | ~1500 (Himalaya proper ~300–350) | ~2500 | 8849 | ~70–80 | onset ~55–50, ongoing | continent–continent | Yin & Harrison 2000; Molnar & Tapponnier 1975; CRUST1.0 |
| Andes (Central/Altiplano) | ~300–400 (plateau) | ~1800 | 6893 | ~65–74 | subduction since Jurassic; uplift Neogene ~25–10 | ocean–continent (flat-slab segments) | Beck & Zandt 2002; Isacks 1988 |
| Alps | ~100–150 | ~1000–1200 | 4809 | ~50–60 | onset ~65–35; collision ~35–0 | continent–continent (Adria–Europe) | Schmid et al. 2004; ESSD 2023 |
| Zagros | ~200–300 | ~2000 | ~4548 | ~45–55 | collision onset ~25–35, ongoing | continent–continent (Arabia–Eurasia) | Paul et al. 2006, 2010; Mouthereau et al. 2012 |
| Caucasus (Greater) | ~100–160 | ~1100 | 5642 (volcanic); 5205 non-volcanic | ~50–55 | shortening since ~Eocene ~35; rapid uplift <5–10 | continent–continent (inverted rift) | Forte et al. 2022; Cowgill et al. 2016 |
| Rockies (Laramide) | ~800–1500 (broad foreland) | ~1500–2000 | 4401 | ~45–50 | ~80–40, long inactive | ocean–continent flat-slab (far-field) | DeCelles 2004; Bird 1998 |
| Appalachians | ~300–500 | ~2000–3000 | 2037 (paleo ~4000+) | ~35–50 (root re-equilibrated) | Taconic ~470–440; Acadian ~420–380; Alleghanian ~330–270 | accretionary → continent–continent | Hatcher 2010 |
| Urals | ~150–250 (~500 w/ foreland) | ~2500 | 1895 | ~50–55 (root preserved) | ~320–250, long inactive | continent–continent (Baltica–Kazakhstania/Siberia) | Berzin et al. 1996; Puchkov 2009 |
| Caledonides (Scandinavian) | ~200–400 | ~1800 (~3000 w/ Greenland/Britain) | 2469 (paleo Himalayan-scale) | ~30–45 (root removed) | Scandian collision ~430–390 | continent–continent (Laurentia–Baltica) | Gee et al. 2008, 2010 |

Notes: Appalachians and Caledonides are deeply eroded — present ~2000–2500 m peaks are erosional remnants of once-Himalayan-scale ranges; present crust is near-normal thickness. Urals are the exception among Paleozoic orogens: a ~50–55 km root is still seismically imaged (URSEIS) after ~250 Myr. Caucasus Elbrus (5642 m) is a Quaternary stratovolcano; highest non-volcanic summit is Dykh-Tau (~5205 m).

---

## 5. Erosion and uplift rates

### Table 5.1 — Denudation rate ranges by setting

| Setting | Typical (mm/yr) | Range (mm/yr) | Dominant method | Source |
|---|---|---|---|---|
| Active orogen (Himalaya, S. Alps NZ, Taiwan) | 1–5 | 0.5 to >10 | cosmogenic 10Be; sediment flux; low-T thermochronology | Montgomery & Brandon 2002; Herman et al. 2013 |
| Post-orogenic / decaying (Appalachians) | ~0.01–0.03 | 0.005–0.05 | 10Be; (U-Th)/He | Portenga & Bierman 2011; Matmon et al. 2003 |
| Stable craton / shield | 0.001–0.01 | 0.0003–0.02 | 10Be outcrop & basin; AFT | Portenga & Bierman 2011 |
| Passive-margin escarpment | ~0.01–0.05 | 0.003–0.1 | 10Be; AFT | Montgomery & Brandon 2002 |
| Global bare-outcrop mean | 0.012 (mean); 0.0054 (median) | — | 10Be in-situ | Portenga & Bierman 2011 |
| Global drainage-basin mean | 0.218 (mean); 0.054 (median) | — | 10Be catchment-averaged | Portenga & Bierman 2011 |

Note: Portenga & Bierman outcrop means by rock type — igneous 8.7±1.0, metamorphic 11±1.4, sedimentary 20±2.0 m/Myr. Erosion rate rises steeply with mean local relief in active ranges (Montgomery & Brandon 2002).

### Table 5.2 — Stream-power incision erodibility K

Stream-power law: E = K·A^m·S^n. K is NOT dimensionless; its units are m^(1−2m)·yr^−1 (A in m², E in m/yr) and depend on the adopted m and n. Values below are comparable only at the same (m, n). Common choice: n≈1, m/n≈0.5.

| Substrate | K value | Assumed m, n | Source |
|---|---|---|---|
| All lithologies (field inversion) | ~10^-2 to 10^-7 (≈5 orders) | per-site fit (m≈0.3–0.5, n≈0.7–1) | Stock & Montgomery 1999 |
| Weak — mudstone/argillite/volcaniclastic | ~10^-2 to 10^-4 | as above | Stock & Montgomery 1999 |
| Hard — granite/basalt/resistant metamorphic | ~10^-5 to 10^-7 | as above | Stock & Montgomery 1999 |
| Generic bedrock (modeling reference) | ~10^-6 | m=0.5, n=1 → K in yr^-1 | Whipple & Tucker 1999 |
| Global 10Be-calibrated K (59 areas) | spans ~4 orders of magnitude | n=1, m/n=0.5 | Harel, Mudd & Attal 2016 |

Note: K absorbs discharge variability, channel-width scaling, incision thresholds, and sediment effects. n is often >1 (commonly ≈2) in threshold-stochastic data; a K quoted for n=1 cannot be reused at n≠1 (Lague 2014). The local K–precipitation correlation does not hold globally (Harel et al. 2016).

---

## 6. Rivers

### Table 6.1 — Ten largest basins by drainage area

| River | Area (km²) | Range | Source |
|---|---|---|---|
| Amazon | ~6,300,000 | 5.9–7.05 M | Milliman & Farnsworth 2011; Latrubesse et al. 2005 |
| Congo | ~3,700,000 | 3.70–4.01 M | Milliman & Farnsworth 2011 |
| Nile | ~3,250,000 | 3.0–3.4 M | Milliman & Farnsworth 2011 |
| Mississippi–Missouri | ~3,200,000 | 3.20–3.27 M | Milliman & Farnsworth 2011 |
| Ob–Irtysh | ~2,990,000 | 2.95–2.99 M | Dai & Trenberth 2002 |
| Paraná | ~2,580,000 (La Plata ~3.1 M) | 2.58–2.80 M | Milliman & Farnsworth 2011 |
| Yenisei | ~2,580,000 | 2.54–2.62 M | Dai & Trenberth 2002 |
| Lena | ~2,490,000 | 2.42–2.49 M | Dai & Trenberth 2002 |
| Niger | ~2,090,000 | 2.09–2.27 M | Milliman & Farnsworth 2011 |
| Amur | ~1,855,000 | 1.86–2.05 M | Dai & Trenberth 2002 |

Note: ranking near #3–#8 is sensitive to basin-boundary/endorheic conventions.

### Table 6.2 — Ten largest rivers by mean discharge

| River | Discharge (m³/s) | Range | Source |
|---|---|---|---|
| Amazon | ~209,000 | 200–220 k | Milliman & Farnsworth 2011; Dai & Trenberth 2002 |
| Congo | ~41,000 | 40–42 k | Milliman & Farnsworth 2011 |
| Ganges–Brahmaputra | ~38,000 | 35–42 k | Milliman & Farnsworth 2011 |
| Orinoco | ~37,000 | 33–37 k | Milliman & Farnsworth 2011 |
| Madeira (Amazon trib.) | ~31,000 | 28–32 k | Milliman & Farnsworth 2011 |
| Yangtze | ~30,000 | 28–31 k | Dai & Trenberth 2002 |
| Río de la Plata / Paraná | ~22,000 | 18–25 k | Milliman & Farnsworth 2011 |
| Yenisei | ~19,600 | 18–19.8 k | Dai & Trenberth 2002 |
| Mississippi | ~18,000 | 16.8–18.4 k | Milliman & Farnsworth 2011 |
| Lena | ~17,000 | 16–17 k | Dai & Trenberth 2002 |

### Table 6.3 — Longest rivers

| River | Length (km) | Range | Source |
|---|---|---|---|
| Nile | ~6,650 | 6,650–6,695 | Britannica; standard refs |
| Amazon | ~6,400 | 6,400–6,992 | INPE 2007–2008 surveys |
| Yangtze | ~6,300 | 6,300–6,418 | standard refs |
| Mississippi–Missouri | ~6,275 | 5,970–6,275 | USGS |
| Yenisei–Angara–Selenga | ~5,540 | 5,075–5,540 | standard refs |

Note: Nile vs Amazon #1 is disputed; the answer depends on source-point and estuary-endpoint definitions.

### Table 6.4 — Drainage density by terrain/climate

| Terrain / climate | Density (km/km²) | Source |
|---|---|---|
| Coarse — resistant rock, humid, vegetated | <4–5 | Smith 1950; Charlton 2008 |
| Medium — mixed lithology/climate | ~5–14 | Smith 1950; Charlton 2008 |
| Fine — weak/impermeable rock, semi-arid | ~14–155 | Smith 1950; Charlton 2008 |
| Badlands (weak clay/shale, sparse vegetation) | >155 (to ~500–1000) | Smith 1950 |

### Table 6.5 — Delta progradation (examples)

| Delta | Rate | Source |
|---|---|---|
| Huanghe (Yellow R.), historic fastest | net land gain ~20–25 km²/yr (peak 20th C.) | Wu et al. 2017; Wang et al. 2021 |
| Mississippi (Wax Lake / Atchafalaya) | ~1–3 km²/yr building | Hoitink et al. 2020 |
| Ganges–Brahmaputra–Meghna | few km²/yr net | Hoitink et al. 2020 |
| Nile (post-Aswan 1964) | net retreat ~10–100+ m/yr | Stanley & Warne |

Note: many deltas have reversed to net erosion where upstream damming cut sediment supply; pre-dam natural rates were larger.

---

## 7. Hypsometry

### Table 7.1 — Earth's bimodal elevation distribution (present)

| Quantity | Value | Range | Source |
|---|---|---|---|
| Continental (upper) mode | broad peak near 0 to +0.3 km | modal land ~0.1–0.8 km | Cogley 1984 |
| Abyssal (lower) mode | ~−4 to −5 km | 3–6 km depth | Cogley 1984 |
| Mean land elevation | ~840 m | ~800–840 m | Cogley 1984 |
| Mean ocean depth | ~−3.7 km (−3688 m) | −3.68 to −3.8 km | Cogley 1984 |
| Land fraction of surface | ~29% | ocean ~71% | Cogley 1984 |
| Continental shelf fraction of surface | ~5–8% | definition-dependent (~200 m shelf break) | Cogley 1984 |

### Table 7.2 — Emergent land / continental freeboard through deep time

| Interval / model | Emergent land fraction | Source |
|---|---|---|
| Present day | ~29–30% | Cogley 1984 |
| Constant-freeboard hypothesis (near-modern since ~2.5 Ga) | ~near modern for most of Phanerozoic–Proterozoic | Wise 1974; Eriksson |
| Pre–late-Archean (thermal-evolution school) | ~2–4% of surface | Flament et al. 2008 |
| Archean → modern rise mechanism | low emergence from hotter mantle / deeper basins, rising by ~2.5 Ga | Flament et al. 2013; Korenaga et al. 2017 |

Note: genuinely contested. The constant-freeboard school (Wise, Eriksson) argues near-modern emergence since the late Archean; the thermal-evolution school (Flament, Korenaga) argues Archean emergent land was far lower, rising through the Neoarchean–Paleoproterozoic.

---

## 8. Ocean floor

### Table 8.1 — Age–depth relation constants (t in Ma, d in m)

| Model / parameter | Value | Validity | Source |
|---|---|---|---|
| Half-space, young: d = 2500 + 350·√t | ridge 2500 m; coeff 350 m/√Ma | t ≲ 70 Ma | Parsons & Sclater 1977 |
| Half-space, old: d = 6400 − 3200·exp(−t/62.8) | asymptote 6400 m; scale 62.8 Ma | t ≳ 20 Ma | Parsons & Sclater 1977 |
| P&S plate thickness / basal T | ~125 km; ~1333 °C | — | Parsons & Sclater 1977 |
| GDH1, young (t<20 Ma): d = 2600 + 365·√t | ridge 2600 m; coeff 365 m/√Ma | — | Stein & Stein 1992 |
| GDH1, old (t≥20 Ma): d = 5651 − 2473·exp(−0.0278·t) | flattening depth 5651 m | — | Stein & Stein 1992 |
| GDH1 plate thickness (a) | 95 ± 10 km | — | Stein & Stein 1992 |
| GDH1 basal temperature | 1450 ± 100 °C | — | Stein & Stein 1992 |
| GDH1 thermal expansion α | 3.1 × 10^-5 K^-1 | — | Stein & Stein 1992 |
| GDH1 heat flow, young: q = 510·t^(−1/2) mW/m² | coeff 510 | t < 55 Ma | Stein & Stein 1992 |

### Table 8.2 — Oldest surviving in-situ oceanic crust

| Location | Age (Ma) | Notes | Source |
|---|---|---|---|
| Herodotus Basin, E. Mediterranean | ~340 | relic Tethys; inferred from magnetic anomalies; CONTESTED | Granot 2016 |
| Western Pacific (Pigafetta/E. Mariana) | ~170 (Middle Jurassic) | oldest in main ocean basins | Müller et al. 2008 |
| Global age-grid max (excl. Mediterranean relic) | ~180–200 | W. Pacific | Müller et al. 2008 |

### Table 8.3 — Ridge full spreading rates and classification

| Ridge / class | Full rate (mm/yr) | Range | Source |
|---|---|---|---|
| East Pacific Rise (fast/superfast) | ~130–150 | 90–170 | DeMets et al. 2010; Macdonald 2001 |
| Mid-Atlantic Ridge (slow) | ~25 | 20–40 | DeMets et al. 2010 |
| Central/SE Indian (intermediate) | ~50–70 | 40–90 | DeMets et al. 2010 |
| Gakkel / SW Indian (ultraslow) | ~7–15 | <20 | Dick et al. 2003 |
| Global mean full spreading rate | ~50 | 40–55 | Müller et al. 2008 |
| Class thresholds: ultraslow / slow / intermediate / fast / superfast | <20 / 20–55 / 55–90 / 90–140 / >140 | — | Macdonald 1982, 2001; Dick et al. 2003 |

---

## 9. Continental crust growth

### Table 9.1 — Present-day continental crust

| Quantity | Value | Range | Source |
|---|---|---|---|
| Area (continental-type crust, incl. submerged margins) | ~2.0–2.1 × 10^8 km² | ~40–41% of surface | Cawood et al. 2013; Stein & Ben-Avraham 2007 |
| Fraction emergent (subaerial) | ~29–30% of surface | ~71% of continental area | Cawood et al. 2013 |
| Volume | ~7.2 × 10^9 km³ | 7.0–7.6 × 10^9 km³ | Cawood, Hawkesworth & Dhuime 2013 |
| Average thickness | ~36–40 km | 20–80 km | Stein & Ben-Avraham 2007 |

### Table 9.2 — Fraction of present crust volume established by a given age (competing models)

| Model family | By ~3.0 Ga | By ~2.5 Ga | By ~1.8 Ga | Source |
|---|---|---|---|---|
| Armstrong constant-volume / recycling (no net growth) | ~100% | ~100% | ~100% | Armstrong 1981, 1991 |
| Dhuime et al. (Hf-isotope) | ~65% | ~70% | ~75–80% | Dhuime et al. 2012 |
| Taylor & McLennan (Archean-dominated) | ~50–60% | ~60–70% | ~75% | Taylor & McLennan 1985, 1995 |
| Condie (episodic pulses ~2.7, 1.9, 1.2 Ga) | ~40–50% | ~55–65% | ~70–80% | Condie 1998, 2000 |
| Cawood et al. / progressive-growth curves | ~65% | ~70% | ~75% | Cawood et al. 2013; Belousova et al. 2010 |

Note: two signals are conflated in the literature — volume generated (juvenile addition) vs volume preserved/present. Armstrong-type models argue large early volumes were later recycled, so "% of present existing then" can approach or exceed 100%. Progressive-growth models track net preserved crust. Fractions carry roughly ±10–15% uncertainty.

### Table 9.3 — Crust production / addition rates

| Process / epoch | Value (km³/yr) | Range | Source |
|---|---|---|---|
| Present arc addition (gross juvenile) | ~1.65 | 1.0–2.0 | Reymer & Schubert 1984 |
| Present subduction removal | ~0.6 | up to 1.3–3.2 (high-recycling) | Stein & Ben-Avraham 2007; Clift et al. 2009 |
| Present NET growth | ~1.0 | 0.9–1.1; some argue net ≈ 0 | Reymer & Schubert 1984 |
| Gross flux (arc+plume, near-balanced) | ~2.5–3.8 add / ~2.5–3.2 remove | net near zero | Scholl & von Huene 2007, 2009 |
| Pre-3.0 Ga production | ~3 | model-dependent | Dhuime et al. 2012 |
| Post-3.0 Ga average | ~0.8 | model-dependent | Dhuime et al. 2012 |

Note: production estimates disagree by ~3–4× chiefly over how much crust is lost to subduction erosion and relamination. A growing body of work (Clift, Scholl & von Huene, Stern) argues gross production is high but net growth is near zero today, consistent with the Armstrong end-member.

---

## 10. The future

### Table 10.1 — Next supercontinent models

| Model | Assembly (Ma from now) | Ocean behavior | Cluster & location | Source |
|---|---|---|---|---|
| Pangea Proxima (Pangea Ultima) | ~250 | introversion — Atlantic closes | Americas re-collide with Africa–Eurasia; ring around interior sea, low latitudes | Scotese 1982, 2003 |
| Novopangea | ~200–250 | extroversion — Pacific closes | Americas drift W into Antarctica then Africa–Eurasia | Nield 2007; Davies, Duarte et al. 2018 |
| Aurica | ~250 | both close; new Pan-Asian ocean opens | Australia central; Asia + Americas close Pacific; near-equatorial | Duarte et al. 2018 |
| Amasia | ~200–250 (orthoversion models to ~300) | orthoversion — closes ~90° from Pangea centroid, poleward | continents gather at North Pole; Antarctica stays at South Pole | Hoffman 1992; Mitchell et al. 2012; Yuan/Li/Davies et al. 2022 |

Note: assembly dates cluster around ~200–250 Myr from now across all four models.

### Table 10.2 — IPCC AR6 (2021) global mean sea-level rise (rel. 1995–2014; "likely" = 17–83%)

| Horizon / scenario | Rise | Confidence | Source |
|---|---|---|---|
| 2100, SSP1-2.6 | 0.32–0.62 m (median ~0.44) | medium | IPCC AR6 WG1, Fox-Kemper et al. 2021 |
| 2100, SSP2-4.5 | 0.44–0.76 m (median ~0.56) | medium | IPCC AR6 WG1 2021 |
| 2100, SSP5-8.5 | 0.63–1.01 m (median ~0.77) | medium | IPCC AR6 WG1 2021 |
| 2100, SSP5-8.5 + ice-sheet instability | up to ~1.6 m; ~2 m not ruled out | low | IPCC AR6 WG1 SPM D.5.2 2021 |
| 2300, SSP1-2.6 (low) | 0.3–3.1 m | low (deep uncertainty) | IPCC AR6 WG1 SPM D.5.2 2021 |
| 2300, SSP5-8.5 (high) | 1.7–6.8 m | low | IPCC AR6 WG1 SPM D.5.2 2021 |
| 2300, SSP5-8.5 + deep-uncertainty ice processes | up to ~16 m | low | IPCC AR6 WG1 SPM D.5.2 2021 |

### Table 10.3 — Milankovitch cycle periods

| Cycle | Present-day period | Deep-time behavior | Source |
|---|---|---|---|
| Eccentricity (short) | ~100 kyr (~95 & ~124 kyr components) | stable in period | Laskar et al. 2004; Berger & Loutre 1991 |
| Eccentricity (long) | 405 kyr | most stable ("metronome"); primary deep-time tuning target | Laskar et al. 2004; Hinnov 2013 |
| Axial obliquity | ~41 kyr (tilt 22.1°–24.5°) | shorter in deep past (~half at 2.46 Ga) | Laskar et al. 2004; Lantink et al. 2022 |
| Axial (climatic) precession | ~19 & ~23 kyr (mean ~21–22) | shorter in deep past (~11 kyr at 2.46 Ga) | Berger & Loutre 1991; Lantink et al. 2022 |

Note: obliquity and precession periods lengthen over geologic time as lunar tidal drag slows Earth's rotation. Do not assume a linear scaling; interpolate deep-time values from a tidal-evolution model (Waltham 2015; Farhat et al. 2022).

---

## Calibration quick sheet

The 20 numbers most useful as simulation gates. Tolerance is the band inside which a modeled Earth-like planet should fall; it reflects both measurement uncertainty and model spread, so it is wider than any single paper's error bar. Cross-reference the section tables before wiring any gate.

| # | Gate | Value | Tolerance | Source |
|---|---|---|---|---|
| 1 | Major plates today | 7 major; 14 largest cover ~95% of surface | ±2 on "major" count | Bird 2003 |
| 2 | Supercontinent cycle period | ~600 Myr | 300–800 Myr | Nance & Murphy 2013 |
| 3 | Pangea assembly → breakup | ~320 Ma → ~200 Ma | ±30 Myr | Bradley 2011 |
| 4 | Continental crust volume | 7.2 × 10^9 km³ | 7.0–7.6 × 10^9 | Cawood et al. 2013 |
| 5 | Continental crust area | ~40% of surface (29% emergent) | ±3% surface | Cawood et al. 2013; Cogley 1984 |
| 6 | Net continental crust growth rate | ~1.0 km³/yr | 0 to ~1.6 (net-zero end-member allowed) | Reymer & Schubert 1984; Scholl & von Huene 2007 |
| 7 | Fastest plate (Cocos) | 74.6 mm/yr | 70–100 mm/yr for fastest plate | Argus et al. 2011 |
| 8 | Slowest major plate (S. America) | 10.6 mm/yr | <~20 mm/yr | Argus et al. 2011 |
| 9 | Global RMS plate speed | 43.9 mm/yr | 35–50 mm/yr | Argus et al. 2011 |
| 10 | India peak drift (~68–65 Ma) → post-collision | 180–200 → ~40 mm/yr | peak 150–200; ≥2× slowdown at collision | Kumar et al. 2007; Copley et al. 2010 |
| 11 | Global mean full spreading rate | ~50 mm/yr (MAR ~25, EPR ~130–150) | 40–55 global mean | Müller et al. 2008; DeMets et al. 2010 |
| 12 | Age–depth, young crust | d = 2500 + 350·√t (m, Ma) | coeff 350–365 (P&S vs GDH1) | Parsons & Sclater 1977 |
| 13 | Age–depth, old-crust flattening | ~5650–6400 m asymptote | GDH1 5651; P&S 6400 | Stein & Stein 1992; Parsons & Sclater 1977 |
| 14 | Oldest in-situ ocean crust | ~170 Ma (main basins) | 170–200 Ma; ~340 Ma if relic Tethys counted | Müller et al. 2008; Granot 2016 |
| 15 | LGM eustatic lowstand | −134 m | −125 to −134 m | Lambeck et al. 2014 |
| 16 | Total land-ice sea-level equivalent | ~65 m (Greenland 7.4 + Antarctica 58) | ±2 m | Morlighem 2017; Fretwell 2013 |
| 17 | Hypsometry anchors | land +840 m, ocean −3.7 km, land 29% | mean land ±100 m; land 28–30% | Cogley 1984 |
| 18 | Himalaya–Tibet crustal thickness (thickest crust) | 70–80 km | 65–80 km for major collisional plateau | Yin & Harrison 2000; CRUST1.0 |
| 19 | Erosion rate span (orogen → craton) | 1–5 → 0.001–0.01 mm/yr; global basin median 0.054 | 3–4 orders of magnitude spread | Portenga & Bierman 2011 |
| 20 | Milankovitch periods | 405 / ~100 / 41 / 23 / 19 kyr | ±few % (present); shorter in deep past | Laskar et al. 2004 |

---

## Confidence and usage notes

Confirmed against primary sources this pass: MORVEL/NNR-MORVEL56 plate speeds, Parsons & Sclater equations, India drift history, Herodotus 340 Ma, EPR/MAR spreading rates, Lambeck LGM lowstand, ice-sheet SLE, Portenga & Bierman global erosion means, IPCC AR6 projections, river discharge/area figures.

Canonical values not independently re-derived this pass (check the primary PDF before treating as a hard gate): GDH1 parameters in Table 8.1 rows 4–9 (Stein & Stein 1992, Nature 359:123–128) and the Macdonald spreading-rate class thresholds — established constants, but worth one verification if safety-critical. Stock & Montgomery per-lithology K numerics could not be re-extracted from the publisher this pass; the ~10^-2 to 10^-7 span is the widely-cited characterization of their result — confirm exact table values before hard-coding, and remember K's units change with m and n.

Genuinely contested (model against a range, not a point): Pannotia's existence; Kenorland as a single mass; Phanerozoic eustatic amplitudes (Haq vs Miller — prefer Miller); Archean emergent-land fraction (constant-freeboard vs thermal-evolution); net crust growth vs constant-volume recycling; deep-time Milankovitch periods; all IPCC 2300 figures.
