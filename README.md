# Image Sort Visualizer

A graphical sorting algorithm visualizer for images. Takes any image, splits it into a grid of cells, shuffles them, and plays back a sorting algorithm rearranging the cells to restore the original image.

## Features

- 6 builtin sorting algorithms: Bubble, Insertion, Selection, Quicksort, Heapsort, Mergesort
- Sort by **original index** (restore image) or **luminance** (arrange by brightness)
- **Compare mode**: run multiple algorithms side-by-side on the same shuffle
- Deterministic seeded shuffles with on-disk caching for instant replay
- Configurable grid size, animation speed, and duration
- Interactive controls: pause, reshuffle, switch algorithms, adjust speed
- **Procedural sorting sounds**: every comparison/swap/set emits a short sine tone pitched by cell value (mute with `M`, disable with `--no-sound`)

## Build

> [!CAUTION]
> You need to have Rust + Cargo installed in your system to be able to build the repo

```bash
cargo build --release
```

## Usage

```bash
cargo run --release -- <IMAGE_PATH> [OPTIONS]
```

### Options

| Flag | Description | Default |
|---|---|---|
| `--algo <ALGO>` | Sorting algorithm (`bubble`, `insertion`, `selection`, `quick`, `heap`, `merge`) | `bubble` |
| `--key <KEY>` | Sort key: `index` or `luma` | `index` |
| `--cols <N>` | Number of columns | 20 |
| `--rows <N>` | Number of rows | 20 |
| `--cell <PX>` | Cell size in pixels (overrides cols/rows) | - |
| `--seed <N>` | Random seed for reproducible shuffles | random |
| `--duration <SEC>` | Target animation duration in seconds | 10 |
| `--speed <N>` | Events per frame (overrides auto-speed) | auto |
| `--grid` | Draw cell grid lines | off |
| `--compare [ALGOS]` | Side-by-side comparison mode | all algorithms |
| `--cache-dir <DIR>` | Cache directory for seeded animations | `~/.cache/sort_visualizer` |
| `--no-cache` | Disable caching | off |
| `--no-sound` | Disable sorting sounds | off |

### Examples

```bash
# Basic: bubble sort on default 20x20 grid
cargo run --release -- photo.jpg

# 40x30 grid, heapsort by luminance
cargo run --release -- photo.jpg --cols 40 --rows 30 --algo heap --key luma

# 32px cell size
cargo run --release -- photo.jpg --cell 32

# Compare quicksort, mergesort, and bubblesort side-by-side
cargo run --release -- photo.jpg --compare quick,merge,bubble

# Reproducible 15-second animation
cargo run --release -- photo.jpg --seed 42 --duration 15
```

### Interactive Controls

| Key | Action |
|---|---|
| Left / Right | Switch algorithm (single mode) |
| Space | Pause / resume |
| Up / Down | Increase / decrease animation speed |
| M | Mute / unmute sorting sounds |
| R | Reshuffle with a new random permutation |
| Q / Esc | Quit |

## How It Works

1. The image is loaded and cropped to fit an integer grid.
2. It is split into a grid of cells, each assigned a luminance value (Rec. 709).
3. Cells are shuffled into a random permutation (seeded for reproducibility).
4. The selected sorting algorithm runs on the permutation, recording every comparison and swap as an event.
5. Events are replayed at a configurable rate, animating the cells back into sorted order.

In **compare mode**, the same initial shuffle is shared across all panes, so you can watch different algorithms solve the identical puzzle side-by-side.

## Sound

Sorting sounds play by default when an audio device is available (disable at startup with `--no-sound`, or mute anytime with `M`). Each event produces short decaying sine tones:

- **Comparison** — two quiet notes for the cells being compared
- **Swap** — two louder notes for the values being exchanged
- **Set** (mergesort) — a single note for the value settling into its slot

Pitch rises logarithmically with the cell's sort key (its original index or luminance), so the sequence of tones traces the algorithm's activity — `bubble` sounds rhythmic, `quick` frantic, `merge` smooth. In compare mode each pane plays its own tones simultaneously on the same output device. Note that at very high speeds (many events per frame) tones are dropped once the audio output falls behind, keeping the sound tied to the animation instead of lagging behind it.

## Project Structure

```
src/
  main.rs    -- CLI, render loop, keyboard controls
  sort.rs    -- Sorting algorithms and macro-driven registry
  grid.rs    -- Image-to-grid decomposition and luminance
  cache.rs   -- On-disk binary cache for seeded animations
  sound.rs   -- Procedural sorting sounds (sine tone synthesis)
```

## License

No license specified.
