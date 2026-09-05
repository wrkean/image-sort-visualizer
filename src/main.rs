mod cache;
mod grid;
mod sort;
mod sound;

use std::cmp::Ordering;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};
use grid::Grid;
use macroquad::prelude::*;
use ::rand::seq::SliceRandom;
use ::rand::SeedableRng;
use sort::{Algo, Event};

#[derive(Clone, Copy, PartialEq, Eq, Debug, ValueEnum)]
enum KeyMode {
    #[value(alias = "idx", alias = "position", alias = "original")]
    Index,
    #[value(alias = "luminance", alias = "brightness", alias = "intensity")]
    Luma,
}

impl KeyMode {
    fn name(self) -> &'static str {
        match self {
            KeyMode::Index => "original position",
            KeyMode::Luma => "luminance",
        }
    }
}

/// Sorting visualizer for images.
///
/// The image is split into a grid of cells, the cells are shuffled randomly,
/// and then a sorting algorithm rearranges them back — you watch the shuffle
/// collapse into a sorted arrangement.
///
/// With `--seed`, the whole animation is deterministic and cached on disk, so
/// re-running with the same seed + settings loads instantly.
#[derive(Parser)]
#[command(
    name = "sort",
    version,
    about = "Sorting visualizer for images",
    long_about = "The image is split into a grid of cells, the cells are shuffled randomly, \
                  and then a sorting algorithm rearranges them back — you watch the shuffle \
                  collapse into a sorted arrangement. With --seed, the animation is \
                  deterministic and cached on disk, so re-running with the same seed + \
                  settings loads instantly.",
    after_help = "CONTROLS:\n    Left/Right      switch sorting algorithm (single mode)\n    R               reshuffle (fresh random permutation)\n    Space           pause / resume\n    Up/Down         increase / decrease animation speed\n    M               mute / unmute sorting sounds\n    Q / Esc         quit\n\nCOMPARE MODE:\n    --compare              compare all algorithms side-by-side\n    --compare quick,merge  compare specific algorithms\n\nSOUND:\n    Sorting sounds play by default when an audio device is available. Each\n    comparison/swap/set emits short sine tones pitched by cell value; muted\n    with M or disabled entirely with --no-sound."
)]
struct Options {
    /// Input image to visualize
    image: PathBuf,

    /// Number of cell columns (default 20)
    #[arg(long, value_name = "N")]
    cols: Option<usize>,

    /// Number of cell rows (default 20)
    #[arg(long, value_name = "N")]
    rows: Option<usize>,

    /// Cell edge length in source pixels; alternative to --cols/--rows
    #[arg(long, value_name = "N", conflicts_with_all = ["cols", "rows"])]
    cell: Option<usize>,

    /// Sorting algorithm (ignored in compare mode)
    #[arg(long, value_enum, default_value = "bubble")]
    algo: Algo,

    /// What the cells are sorted by
    #[arg(long, value_enum, default_value = "index")]
    key: KeyMode,

    /// Events applied per frame (default: auto-sized to the --duration target)
    #[arg(long, value_name = "N")]
    speed: Option<usize>,

    /// Target animation length in seconds, used when --speed is not given (default: 10)
    #[arg(long, value_name = "SECS", default_value_t = 10.0)]
    duration: f32,

    /// Reproducible initial shuffle
    #[arg(long, value_name = "N")]
    seed: Option<u64>,

    /// Draw grid lines between cells (default off: cells are flush, boundaries invisible)
    #[arg(short = 'g', long)]
    grid: bool,

    /// Where seed-based visualizations are cached (default: $XDG_CACHE_HOME or ~/.cache/sort_visualizer)
    #[arg(long, value_name = "DIR")]
    cache_dir: Option<PathBuf>,

    /// Disable reading/writing the on-disk cache (only relevant with --seed)
    #[arg(long)]
    no_cache: bool,

    /// Compare algorithms side-by-side. Comma-separated list, or no value for all.
    #[arg(long, value_name = "ALGO1,ALGO2,...", num_args = 0.., default_missing_value = "")]
    compare: Option<Option<String>>,

    /// Disable sorting sounds (off by default: sounds play when an audio device is available)
    #[arg(long)]
    no_sound: bool,
}

impl Options {
    /// Extra checks that aren't natural to express as clap rules.
    fn validate(&self) -> Result<(), String> {
        if self.cols == Some(0) {
            return Err("--cols must be >= 1".into());
        }
        if self.rows == Some(0) {
            return Err("--rows must be >= 1".into());
        }
        if self.cell == Some(0) {
            return Err("--cell must be >= 1".into());
        }
        if self.speed == Some(0) {
            return Err("--speed must be >= 1".into());
        }
        if self.duration <= 0.0 {
            return Err("--duration must be > 0".into());
        }
        if !self.image.exists() {
            return Err(format!("image file '{}' does not exist", self.image.display()));
        }
        Ok(())
    }
}

fn window_conf() -> Conf {
    Conf {
        window_title: "Image Sort Visualizer".to_owned(),
        window_width: 1280,
        window_height: 900,
        window_resizable: true,
        ..Default::default()
    }
}

fn shuffled(n: usize, seed: Option<u64>) -> Vec<usize> {
    let mut v: Vec<usize> = (0..n).collect();
    match seed {
        Some(s) => v.shuffle(&mut ::rand::rngs::StdRng::seed_from_u64(s)),
        None => v.shuffle(&mut ::rand::rng()),
    }
    v
}

fn key_cmp(a: usize, b: usize, grid: &Grid, key: KeyMode) -> Ordering {
    match key {
        KeyMode::Index => a.cmp(&b),
        KeyMode::Luma => grid.luma[a]
            .partial_cmp(&grid.luma[b])
            .unwrap_or(Ordering::Equal),
    }
}

fn compute_events(algo: Algo, base: &[usize], grid: &Grid, key: KeyMode) -> Vec<Event> {
    let mut work = base.to_vec();
    let cmp = |a: usize, b: usize| key_cmp(a, b, grid, key);
    sort::collect_events(algo, &mut work, &cmp)
}

/// Auto-pick a playback speed so the whole animation takes roughly
/// `duration_s` seconds at 60 fps.
fn auto_speed(total_events: usize, duration_s: f32) -> usize {
    let frames = (60.0 * duration_s).max(1.0);
    ((total_events as f32 / frames).ceil()).clamp(1.0, 1_000_000.0) as usize
}

/// Determine the requested grid dimensions.
///
/// When `--cell` is given, each cell spans `cell` source pixels, so the grid
/// is `img_w / cell` by `img_h / cell`. Otherwise `--cols`/`--rows` are used
/// directly, falling back to the default 20 when unspecified.
fn resolve_grid(
    cell: Option<usize>,
    cols: Option<usize>,
    rows: Option<usize>,
    img_w: usize,
    img_h: usize,
) -> (usize, usize) {
    match cell {
        Some(c) => ((img_w / c).max(1), (img_h / c).max(1)),
        None => (cols.unwrap_or(20), rows.unwrap_or(20)),
    }
}

/// Draws `text` at `x`,`y`, scaling it down (if needed) so it fits within `max_w` pixels.
/// Uses the default font; text is clipped to the pane width instead of overflowing into neighbors.
fn draw_fitting_text(text: &str, x: f32, y: f32, max_w: f32, font_size: u16, color: Color) {
    let dims = measure_text(text, None, font_size, 1.0);
    let scale = if dims.width > max_w && max_w > 0.0 {
        (max_w / dims.width).clamp(0.35, 1.0)
    } else {
        1.0
    };
    draw_text_ex(
        text,
        x,
        y,
        TextParams {
            font_size,
            font_scale: scale,
            color,
            ..Default::default()
        },
    );
}

#[macroquad::main(window_conf)]
async fn main() {
    let opts = Options::parse();
    if let Err(e) = opts.validate() {
        eprintln!("error: {e}");
        std::process::exit(2);
    }

    let path = &opts.image;
    let img = match image::open(path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to load '{}': {e}", path.display());
            std::process::exit(1);
        }
    };

    // Make sure the GL context is fully initialised before uploading textures.
    next_frame().await;

    // Resolve grid dimensions: --cell derives them from the image size.
    let (grid_cols, grid_rows) =
        resolve_grid(opts.cell, opts.cols, opts.rows, img.width() as usize, img.height() as usize);

    let grid = Grid::new(&img, grid_cols, grid_rows);
    let cols = grid.cols;
    let rows = grid.rows;
    let n = cols * rows;

    let key = opts.key;
    let key_idx = match key {
        KeyMode::Index => 0,
        KeyMode::Luma => 1,
    };

    // Parse compare algorithms
    let compare_algos: Vec<Algo> = match &opts.compare {
        Some(inner) => {
            let s = inner.as_deref().unwrap_or("");
            if s.is_empty() {
                Algo::ALL.to_vec()
            } else {
                s.split(',')
                    .map(|a| Algo::from_str(a.trim(), true).unwrap())
                    .collect()
            }
        }
        None => vec![opts.algo],
    };

    // Check if we're in compare mode (more than 1 algorithm)
    let compare_mode = compare_algos.len() > 1;

    // ----- seed-based on-disk cache of the precomputed animation ----------
    let cache_active = opts.seed.is_some() && !opts.no_cache;
    let seed_val = opts.seed.unwrap_or(0);
    let image_hash = if cache_active {
        cache::fnv1a(img.to_rgba8().as_raw())
    } else {
        0
    };
    let cache_dir = opts.cache_dir.clone().unwrap_or_else(cache::default_dir);

    // Build animation state for each algorithm
    let make_header = |algo: Algo| -> cache::CacheHeader {
        cache::CacheHeader {
            seed: seed_val,
            image_hash,
            cols: cols as u32,
            rows: rows as u32,
            algo: algo.code(),
            key: key_idx,
            n: n as u32,
        }
    };

    struct AnimationState {
        algo: Algo,
        base: Vec<usize>,
        events: Vec<Event>,
        data: Vec<usize>,
        cursor: usize,
        speed: usize,
        paused: bool,
        done: bool,
        seeded_base: bool,
        sound: Option<sound::Sound>,
    }

    let mut animations: Vec<AnimationState> = Vec::new();

    // If in compare mode with seed, use the SAME seeded base for all algorithms
    let shared_base = if cache_active && compare_mode {
        let base = shuffled(n, Some(seed_val));
        Some(base)
    } else {
        None
    };

    for algo in compare_algos {
        let (base, events, speed) = if cache_active {
            let header = make_header(algo);
            let path = cache::cache_path(&cache_dir, &header);
            match cache::load(&path, &header) {
                Some(v) => {
                    eprintln!("[cache] hit   {}", path.display());
                    let s = opts.speed.unwrap_or_else(|| auto_speed(v.events.len(), opts.duration));
                    (v.base, v.events, s)
                }
                None => {
                    let b = if let Some(ref sb) = shared_base {
                        sb.clone()
                    } else {
                        shuffled(n, Some(seed_val))
                    };
                    let e = compute_events(algo, &b, &grid, key);
                    let s = opts.speed.unwrap_or_else(|| auto_speed(e.len(), opts.duration));
                    match cache::save(&path, &header, &b, &e) {
                        Ok(()) => eprintln!("[cache] wrote {}", path.display()),
                        Err(err) => eprintln!("[cache] warning: failed to write cache: {err}"),
                    }
                    (b, e, s)
                }
            }
        } else {
            let b = if let Some(ref sb) = shared_base {
                sb.clone()
            } else {
                shuffled(n, opts.seed)
            };
            let e = compute_events(algo, &b, &grid, key);
            let s = opts.speed.unwrap_or_else(|| auto_speed(e.len(), opts.duration));
            (b, e, s)
        };

        let seeded_base = cache_active && (shared_base.is_some() || opts.seed.is_some());
        animations.push(AnimationState {
            algo,
            base: base.clone(),
            events,
            data: base.clone(),
            cursor: 0,
            speed,
            paused: false,
            done: false,
            seeded_base,
            sound: None,
        });
    }

    // Sound system: one outlet per pane, all sharing the device's output.
    // If no audio device exists (or --no-sound was given) every pane stays
    // silent; no error is raised.
    let output = if opts.no_sound {
        None
    } else {
        sound::Output::open()
    };
    for anim in animations.iter_mut() {
        anim.sound = output.as_ref().map(|o| o.sound());
    }

    // Outer mute toggle — the per-pane `sound` generators hold the actual state.
    let mut paused = false;
    let mut highlight: Vec<Option<(usize, usize)>> = vec![None; animations.len()];

    // Map a cell id to a normalized pitch value (0..1) describing the key it
    // is sorted by. In index mode the sort key is the cell id itself; in
    // luminance mode it is the cell's brightness. Used to pitch the tones.
    let pitch_of = |id: usize| -> f32 {
        match key {
            KeyMode::Index => {
                if n <= 1 { 0.0 } else { id as f32 / (n - 1) as f32 }
            }
            KeyMode::Luma => grid.luma[id].clamp(0.0, 1.0),
        }
    };

    loop {
        // ---------------------------------------------------------- input
        if is_key_down(KeyCode::Escape) || is_key_down(KeyCode::Q) {
            break;
        }
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
            for anim in &mut animations {
                anim.paused = paused;
            }
        }
        if is_key_pressed(KeyCode::M) {
            for anim in &mut animations {
                if let Some(s) = &mut anim.sound {
                    s.set_muted(!s.is_muted());
                }
            }
        }
        if is_key_pressed(KeyCode::R) {
            for (i, anim) in animations.iter_mut().enumerate() {
                anim.base = if let Some(ref sb) = shared_base {
                    sb.clone()
                } else {
                    shuffled(n, None)
                };
                anim.seeded_base = shared_base.is_some();
                anim.events = compute_events(anim.algo, &anim.base, &grid, key);
                anim.data = anim.base.clone();
                anim.cursor = 0;
                anim.done = false;
                highlight[i] = None;
                if opts.speed.is_none() {
                    anim.speed = auto_speed(anim.events.len(), opts.duration);
                }
            }
        }
        if !compare_mode && (is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::Right)) {
            for (i, anim) in animations.iter_mut().enumerate() {
                if is_key_pressed(KeyCode::Left) {
                    anim.algo = anim.algo.prev();
                }
                if is_key_pressed(KeyCode::Right) {
                    anim.algo = anim.algo.next();
                }
                if anim.seeded_base {
                    let header = make_header(anim.algo);
                    let path = cache::cache_path(&cache_dir, &header);
                    match cache::load(&path, &header) {
                        Some(v) => {
                            anim.base = v.base;
                            anim.events = v.events;
                            eprintln!("[cache] hit   {}", path.display());
                        }
                        None => {
                            anim.events = compute_events(anim.algo, &anim.base, &grid, key);
                            match cache::save(&path, &header, &anim.base, &anim.events) {
                                Ok(()) => eprintln!("[cache] wrote {}", path.display()),
                                Err(err) => eprintln!("[cache] warning: failed to write cache: {err}"),
                            }
                        }
                }
                } else {
                    anim.events = compute_events(anim.algo, &anim.base, &grid, key);
                }
                anim.data = anim.base.clone();
                anim.cursor = 0;
                anim.done = false;
                highlight[i] = None;
                if opts.speed.is_none() {
                    anim.speed = auto_speed(anim.events.len(), opts.duration);
                }
            }
        }
        if is_key_pressed(KeyCode::Up) {
            for anim in &mut animations {
                anim.speed = anim.speed.saturating_mul(2).max(1);
            }
        }
        if is_key_pressed(KeyCode::Down) {
            for anim in &mut animations {
                anim.speed = anim.speed.saturating_div(2).max(1);
            }
        }

        // -------------------------------------------------------- advance
        // Reusable per-pane collection of this frame's tones (pitch, volume).
        // Kept outside the loop body and cleared each frame to avoid
        // reallocating per frame.
        let mut frame_tones: Vec<Vec<sound::Tone>> = vec![Vec::new(); animations.len()];
        let mut _all_done = true;
        for (i, anim) in animations.iter_mut().enumerate() {
            if !anim.paused {
                let remaining = anim.events.len().saturating_sub(anim.cursor);
                let steps = anim.speed.min(remaining);
                let end = anim.cursor + steps;
                let tones = &mut frame_tones[i];
                for &e in &anim.events[anim.cursor..end] {
                    match e {
                        Event::Cmp { a, b } => {
                            highlight[i] = Some((a, b));
                            // Quiet blip: the two values being compared.
                            tones.push((pitch_of(anim.data[a]), 0.45));
                            tones.push((pitch_of(anim.data[b]), 0.45));
                        }
                        Event::Swap { a, b } => {
                            // Two notes for the values leaving their slots
                            // (captured before they move).
                            tones.push((pitch_of(anim.data[a]), 0.85));
                            tones.push((pitch_of(anim.data[b]), 0.85));
                            anim.data.swap(a, b);
                            highlight[i] = Some((a, b));
                        }
                        Event::Set { idx, value } => {
                            anim.data[idx] = value;
                            highlight[i] = Some((idx, idx));
                            // Single note for the value settling into its slot.
                            tones.push((pitch_of(value), 0.65));
                        }
                    }
                }
                anim.cursor = end;
            }
            anim.done = anim.cursor >= anim.events.len();
            if !anim.done {
                _all_done = false;
            }
            // Render this pane's frame as one mixed buffer.
            if let Some(s) = &anim.sound {
                s.frame(&frame_tones[i]);
            }
            frame_tones[i].clear();
        }

        // ---------------------------------------------------------- render
        clear_background(Color::from_rgba(16, 18, 24, 255));

        let sw = screen_width();
        let sh = screen_height();
        let pad_h = 30.0f32;
        let header_h = 95.0f32;
        let footer_h = 40.0f32;
        let num_panes = animations.len();
        let avail_w = sw - 2.0 * pad_h;
        let avail_h = sh - header_h - footer_h;

        // Calculate layout for panes (horizontal split)
        let pane_w = avail_w / num_panes as f32;
        let cell_s = (pane_w / cols as f32).min(avail_h / rows as f32).max(1.0);
        let grid_w = cell_s * cols as f32;
        let grid_h = cell_s * rows as f32;
        let gap = if opts.grid {
            (cell_s * 0.04).clamp(1.0, 4.0)
        } else {
            0.0
        };
        let cell_draw = (cell_s - gap).max(1.0);

        // Precompute each grid cell's in-pane offset. These are identical for
        // every pane (only the pane origin `ox`/`oy` differs), so computing them
        // once per frame spares rows*cols*num_panes multiplications/gap adds.
        let mut offset_x = Vec::with_capacity(n);
        let mut offset_y = Vec::with_capacity(n);
        for r in 0..rows {
            for c in 0..cols {
                offset_x.push(c as f32 * cell_s + gap * 0.5);
                offset_y.push(r as f32 * cell_s + gap * 0.5);
            }
        }

        // Shared header (grid + key info, drawn once so narrow panes don't overlap).
        let sound_status = match &animations[0].sound {
            None => "",
            Some(s) if s.is_muted() => "   |   sound: MUTED  (M to unmute)",
            Some(_) => "",
        };
            draw_fitting_text(
            &format!("grid {cols} x {rows}   |   key: {}{sound_status}", key.name()),
            pad_h,
            18.0,
            avail_w,
            26,
            GRAY,
        );

        for (pane_idx, anim) in animations.iter().enumerate() {
            let ox = pad_h + pane_idx as f32 * pane_w + (pane_w - grid_w) / 2.0;
            let oy = header_h + (avail_h - grid_h) / 2.0;

            for r in 0..rows {
                for c in 0..cols {
                    let idx = r * cols + c;
                    let tile = anim.data[idx];
                    let (src_col, src_row) = (tile % cols, tile / cols);
                    let x = ox + offset_x[idx];
                    let y = oy + offset_y[idx];

                    draw_texture_ex(
                        &grid.texture,
                        x,
                        y,
                        WHITE,
                        DrawTextureParams {
                            source: Some(Rect::new(
                                src_col as f32 * grid.cell_w + 0.5,
                                src_row as f32 * grid.cell_h + 0.5,
                                grid.cell_w - 1.0,
                                grid.cell_h - 1.0,
                            )),
                            dest_size: Some(Vec2::new(cell_draw, cell_draw)),
                            ..Default::default()
                        },
                    );
                }
            }

            // Highlight for this pane
            if let Some((a, b)) = highlight[pane_idx] {
                for idx in [a, b] {
                    let x = ox + offset_x[idx];
                    let y = oy + offset_y[idx];
                    draw_rectangle(x, y, cell_draw, cell_draw, Color::from_rgba(255, 224, 60, 90));
                }
            }

            // Per-pane status text
            let status = if anim.done { "  SORTED" } else { "" };
            let algo_name = anim.algo.name();
            draw_fitting_text(algo_name, ox, 38.0, pane_w, 28, WHITE);
            let status_line = format!(
                "step {} / {}{}   {} ev/frame{}",
                anim.cursor,
                anim.events.len(),
                status,
                anim.speed,
                if anim.paused { "  [paused]" } else { "" }
            );
            draw_fitting_text(&status_line, ox, 72.0, pane_w, 16, LIGHTGRAY);
        }

        // Global footer
        draw_text(
            "[Left/Right] algorithm (single mode)   [R] reshuffle   [Space] pause   [Up/Down] speed   [M] mute   [Q/Esc] quit",
            20.0,
            sh - 18.0,
            17.0,
            GRAY,
        );

        next_frame().await;
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_grid;

    const IMG_W: usize = 1920;
    const IMG_H: usize = 1080;

    #[test]
    fn cell_size_derives_grid_from_image_dims() {
        assert_eq!(resolve_grid(Some(128), None, None, IMG_W, IMG_H), (15, 8));
        assert_eq!(resolve_grid(Some(40), None, None, IMG_W, IMG_H), (48, 27));
        assert_eq!(resolve_grid(Some(64), None, None, IMG_W, IMG_H), (30, 16));
    }

    #[test]
    fn cell_size_clamps_to_at_least_one() {
        // Cell larger than the image: fall back to a 1 x 1 grid.
        assert_eq!(resolve_grid(Some(5000), None, None, IMG_W, IMG_H), (1, 1));
    }

    #[test]
    fn cols_rows_defaults() {
        // Unspecified dimensions fall back to 20.
        assert_eq!(resolve_grid(None, None, None, IMG_W, IMG_H), (20, 20));
        assert_eq!(resolve_grid(None, Some(10), None, IMG_W, IMG_H), (10, 20));
        assert_eq!(resolve_grid(None, None, Some(8), IMG_W, IMG_H), (20, 8));
        assert_eq!(resolve_grid(None, Some(10), Some(8), IMG_W, IMG_H), (10, 8));
    }
}