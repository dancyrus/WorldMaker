# WorldMaker

A fantasy world maker and map painter grounded in real geology and climatology.

WorldMaker builds a planet in five simulated global stages — planet setup, plate
tectonics, terrain and rivers, climate, biomes — and lets you paint on the world at
any stage, with downstream stages recomputing around your edits. A later
regional-refinement stage renders a chosen continent in fine detail, because a
detailed region backed by a believable planet is the end goal.

**Current status: Phase 0 — walking skeleton.** A seeded planet with placeholder
fractal-noise elevation, visible on a 3D globe and a flat projected map
(equirectangular and Robinson), with synchronized cursor mapping between the two.
The real science arrives in later phases; see [docs/plan/roadmap.md](docs/plan/roadmap.md).

## How to run it

On Windows, double-click **`WorldMaker.bat`** in the repository root. It builds the
app in release mode (first build takes a few minutes; later launches are quick) and
opens the window.

If you have Rust installed and prefer the command line:

```
cargo run --release -p worldmaker-app
```

### Controls

- **View switcher** — Globe, Flat, or Split (both side by side).
- **Globe** — drag to rotate, scroll to zoom.
- **Flat map** — drag to pan, scroll to zoom; projection dropdown (Equirectangular,
  Robinson); optional graticule.
- **Seed** — type anything into the seed box and press Generate; any text works.
- **Sea level** — slider recolors ocean and land live in both views.
- **Preset** — draft L6 / standard L7 / high L8 grid resolution.
- Hovering either canvas shows the cell id and latitude/longitude under the cursor;
  both views agree on the same ground position.

The timeline strip along the bottom is a placeholder — it becomes the geologic era
picker in Phase 1.

## Project layout

| Crate | What it holds |
|---|---|
| `worldmaker-core` | Geodesic grid, seeded RNG, field storage, projection math |
| `worldmaker-sim` | Stage pipeline scaffolding, placeholder noise-elevation stage |
| `worldmaker-io` | Results-JSON writer, save/export stubs |
| `worldmaker-app` | eframe/egui + wgpu application shell |

Documentation lives in [docs/](docs/): the science outline, the phase roadmap, the
decision log, work orders, and machine-labelled benchmark results.

The app makes no network calls and has no telemetry. Logs go to a rotating file
beside the executable.

## License

MIT — see [LICENSE](LICENSE).
