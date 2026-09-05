mod cache;
mod grid;
mod sort;

use std::cmp::Ordering;
use std::env;
use std::path::PathBuf;

use grid::Grid;
use macroquad::prelude::*;
use ::rand::seq::SliceRandom;
use ::rand::SeedableRng;
use sort::{Algo, Event};

const USAGE: &str = "\
Sorting visualizer for images.

Given an image, the image is split into a grid of cells, the cells are
shuffled randomly, and then a sorting algorithm rearranges them back —
you watch the shuffle collapse into a sorted arrangement.

USAGE:
    sort <image> [options]

OPTIONS:
    --cols N        number of cell columns (default 20)
    --rows N        number of cell rows    (default 20)
    --cell N        alternative to --cols/--rows: each cell is N x N source
                    pixels. The grid size is derived by dividing the image
                    width and height by N (e.g. --cell 32 -> 32x32 px cells)
                    cannot be combined with --cols/--rows
    --algo NAME     sorting algorithm      (default bubble)
                    one of: bubble, insertion, selection, quick, heap, merge
    --key WHAT      what the cells are sorted by   (default index)
                    index  -> cell's original grid position
                             (sorting reconstructs the original image)
                    luma   -> cell's average brightness
    --speed N       events applied per frame (default: auto-sized so the
                    animation takes roughly ten seconds)
    --seed N        reproducible initial shuffle
    --grid on|off   draw the grid lines between cells   (default on)
                    off makes cells flush, so the boundaries are invisible
    --cache-dir DIR where seed-based visualizations are cached (default:
                    $XDG_CACHE_HOME or ~/.cache/sort_visualizer)
    --no-cache      disable reading/writing the on-disk cache (only relevant
                    with --seed; without a seed nothing is cached anyway)

CONTROLS:
    Left/Right      switch sorting algorithm
    R               reshuffle (fresh random permutation)
    Space           pause / resume
    Up/Down         increase / decrease animation speed
    Q / Esc         quit
";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum KeyMode {
    Index,
    Luma,
}

impl KeyMode {
    fn from_name(s: &str) -> Option<KeyMode> {
        match s.trim().to_lowercase().as_str() {
            "index" | "idx" | "position" | "original" => Some(KeyMode::Index),
            "luma" | "luminance" | "brightness" | "intensity" => Some(KeyMode::Luma),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            KeyMode::Index => "original position",
            KeyMode::Luma => "luminance",
        }
    }
}

struct Options {
    path: Option<String>,
    cols: Option<usize>,
    rows: Option<usize>,
    cell: Option<usize>,
    algo: Algo,
    key: KeyMode,
    speed: Option<usize>,
    seed: Option<u64>,
    grid: bool,
    cache_dir: Option<PathBuf>,
    no_cache: bool,
}

impl Options {
    fn parse() -> Result<Options, String> {
        let args: Vec<String> = env::args().skip(1).collect();

        if args.iter().any(|a| a == "-h" || a == "--help") {
            print!("{USAGE}");
            std::process::exit(0);
        }

        let mut o = Options {
            path: None,
            cols: None,
            rows: None,
            cell: None,
            algo: Algo::Bubble,
            key: KeyMode::Index,
            speed: None,
            seed: None,
            grid: true,
            cache_dir: None,
            no_cache: false,
        };

        let mut i = 0;
        macro_rules! need {
            ($opt:expr) => {{
                i += 1;
                args.get(i)
                    .cloned()
                    .ok_or_else(|| format!("missing value for {}", $opt))?
            }};
        }

        while i < args.len() {
            let arg = args[i].as_str();
            match arg {
                "--cols" => {
                    o.cols = Some(need!("--cols").parse().map_err(|_| "invalid --cols value")?);
                }
                "--rows" => {
                    o.rows = Some(need!("--rows").parse().map_err(|_| "invalid --rows value")?);
                }
                "--cell" => {
                    o.cell = Some(need!("--cell").parse().map_err(|_| "invalid --cell value")?);
                }
                "--algo" => {
                    let a = need!("--algo");
                    o.algo = Algo::from_name(&a)
                        .ok_or_else(|| format!("unknown algorithm '{a}'"))?;
                }
                "--key" => {
                    let k = need!("--key");
                    o.key =
                        KeyMode::from_name(&k).ok_or_else(|| format!("unknown key mode '{k}'"))?;
                }
                "--speed" => {
                    o.speed = Some(need!("--speed").parse().map_err(|_| "invalid --speed value")?);
                }
                "--seed" => {
                    o.seed = Some(need!("--seed").parse().map_err(|_| "invalid --seed value")?);
                }
                "--grid" => {
                    let g = need!("--grid");
                    o.grid =
                        parse_bool(&g).ok_or_else(|| format!("invalid --grid value '{g}' (use on/off)"))?;
                }
                "--cache-dir" => {
                    o.cache_dir = Some(PathBuf::from(need!("--cache-dir")));
                }
                "--no-cache" => {
                    o.no_cache = true;
                }
                s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
                _ => {
                    if o.path.is_some() {
                        return Err(format!("multiple image paths given ('{}')", arg));
                    }
                    o.path = Some(arg.to_string());
                }
            }
            i += 1;
        }

        if o.cols == Some(0) {
            return Err("--cols must be >= 1".into());
        }
        if o.rows == Some(0) {
            return Err("--rows must be >= 1".into());
        }
        if o.cell == Some(0) {
            return Err("--cell must be >= 1".into());
        }
        if o.cell.is_some() && (o.cols.is_some() || o.rows.is_some()) {
            return Err("--cell cannot be combined with --cols/--rows".into());
        }
        if o.speed == Some(0) {
            return Err("--speed must be >= 1".into());
        }
        let path = o.path.clone().ok_or_else(|| "missing image path".to_string())?;
        if !std::path::Path::new(&path).exists() {
            return Err(format!("image file '{path}' does not exist"));
        }
        Ok(o)
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

/// Auto-pick a playback speed so the whole animation takes ~10 s at 60 fps.
fn auto_speed(total_events: usize) -> usize {
    (total_events / 600).clamp(1, 1_000_000)
}

/// Parse a human-readable boolean (used for `--grid`).
fn parse_bool(s: &str) -> Option<bool> {
    match s.trim().to_lowercase().as_str() {
        "on" | "true" | "yes" | "1" => Some(true),
        "off" | "false" | "no" | "0" => Some(false),
        _ => None,
    }
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

#[macroquad::main(window_conf)]
async fn main() {
    let opts = match Options::parse() {
        Ok(o) => o,
        Err(e) => {
            eprintln!("error: {e}\n\n{USAGE}");
            return;
        }
    };

    let path = opts.path.as_ref().unwrap(); // guaranteed Some after a successful parse
    let img = match image::open(path) {
        Ok(img) => img,
        Err(e) => {
            eprintln!("error: failed to load '{path}': {e}");
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

    let mut algo = opts.algo;
    let key = opts.key;

    // ----- seed-based on-disk cache of the precomputed animation ----------
    // Everything downstream of the seed is deterministic, so with `--seed` we
    // can persist the shuffled base + recorded events and reload them on a
    // later run instead of re-running the sort.
    let cache_active = opts.seed.is_some() && !opts.no_cache;
    let seed_val = opts.seed.unwrap_or(0);
    let key_idx = match key {
        KeyMode::Index => 0,
        KeyMode::Luma => 1,
    };
    let image_hash = if cache_active {
        cache::fnv1a(img.to_rgba8().as_raw())
    } else {
        0
    };
    let cache_dir = opts.cache_dir.clone().unwrap_or_else(cache::default_dir);

    // The current `base` is the seeded (cacheable) permutation only while the
    // user hasn't hit `R`, which reshuffles with OS randomness. Used to avoid
    // contaminating the cache after a reshuffle.
    let mut seeded_base = cache_active;

    let make_header = |algo: Algo| -> cache::CacheHeader {
        cache::CacheHeader {
            seed: seed_val,
            image_hash,
            cols: cols as u32,
            rows: rows as u32,
            algo: cache::algo_code(algo),
            key: key_idx,
            n: n as u32,
        }
    };

    let (mut base, mut events) = if cache_active {
        let header = make_header(algo);
        let path = cache::cache_path(&cache_dir, &header);
        match cache::load(&path, &header) {
            Some(v) => {
                eprintln!("[cache] hit   {}", path.display());
                (v.base, v.events)
            }
            None => {
                let b = shuffled(n, Some(seed_val));
                let e = compute_events(algo, &b, &grid, key);
                match cache::save(&path, &header, &b, &e) {
                    Ok(()) => eprintln!("[cache] wrote {}", path.display()),
                    Err(err) => eprintln!("[cache] warning: failed to write cache: {err}"),
                }
                (b, e)
            }
        }
    } else {
        let b = shuffled(n, opts.seed);
        let e = compute_events(algo, &b, &grid, key);
        (b, e)
    };

    let mut data = base.clone(); // visible array, mutated by replayed events
    let mut cursor = 0usize;
    let mut highlight: Option<(usize, usize)> = None;
    let mut speed = opts.speed.unwrap_or_else(|| auto_speed(events.len()));
    let mut paused = false;

    loop {
        // ---------------------------------------------------------- input
        if is_key_down(KeyCode::Escape) || is_key_down(KeyCode::Q) {
            break;
        }
        if is_key_pressed(KeyCode::Space) {
            paused = !paused;
        }
        if is_key_pressed(KeyCode::R) {
            base = shuffled(n, None); // fresh random each time, seed only applies to startup
            seeded_base = false; // no longer the seeded permutation: don't touch the cache
            events = compute_events(algo, &base, &grid, key);
            data = base.clone();
            cursor = 0;
            highlight = None;
            if opts.speed.is_none() {
                speed = auto_speed(events.len());
            }
        }
        if is_key_pressed(KeyCode::Left) || is_key_pressed(KeyCode::Right) {
            if is_key_pressed(KeyCode::Left) {
                algo = algo.prev();
            }
            if is_key_pressed(KeyCode::Right) {
                algo = algo.next();
            }
            if seeded_base {
                // The current base is the seeded permutation, so the new
                // algorithm's events can (and already may) come from the cache.
                let header = make_header(algo);
                let path = cache::cache_path(&cache_dir, &header);
                match cache::load(&path, &header) {
                    Some(v) => {
                        base = v.base;
                        events = v.events;
                        eprintln!("[cache] hit   {}", path.display());
                    }
                    None => {
                        events = compute_events(algo, &base, &grid, key);
                        match cache::save(&path, &header, &base, &events) {
                            Ok(()) => eprintln!("[cache] wrote {}", path.display()),
                            Err(err) => eprintln!("[cache] warning: failed to write cache: {err}"),
                        }
                    }
                }
            } else {
                events = compute_events(algo, &base, &grid, key);
            }
            data = base.clone();
            cursor = 0;
            highlight = None;
            if opts.speed.is_none() {
                speed = auto_speed(events.len());
            }
        }
        if is_key_pressed(KeyCode::Up) {
            speed = speed.saturating_mul(2).max(1);
        }
        if is_key_pressed(KeyCode::Down) {
            speed = speed.saturating_div(2).max(1);
        }

        // -------------------------------------------------------- advance
        if !paused {
            let remaining = events.len() - cursor;
            let steps = speed.min(remaining);
            for _ in 0..steps {
                match events[cursor] {
                    Event::Cmp { a, b } => highlight = Some((a, b)),
                    Event::Swap { a, b } => {
                        data.swap(a, b);
                        highlight = Some((a, b));
                    }
                    Event::Set { idx, value } => {
                        data[idx] = value;
                        highlight = Some((idx, idx));
                    }
                }
                cursor += 1;
            }
        }
        let done = cursor >= events.len();

        // ---------------------------------------------------------- render
        clear_background(Color::from_rgba(16, 18, 24, 255));

        let sw = screen_width();
        let sh = screen_height();
        let pad_h = 30.0f32; // left/right screen padding
        let header_h = 95.0f32; // reserved for the status text
        let footer_h = 40.0f32;
        let avail_w = sw - 2.0 * pad_h;
        let avail_h = sh - header_h - footer_h;
        let cell_s = (avail_w / cols as f32).min(avail_h / rows as f32).max(1.0);
        let grid_w = cell_s * cols as f32;
        let grid_h = cell_s * rows as f32;
        let ox = (sw - grid_w) / 2.0;
        let oy = header_h + (avail_h - grid_h) / 2.0;

        // Gap between cells shows the background through it as "grid lines".
        // With `--grid off` the cells are drawn flush, so the grid is invisible.
        let gap = if opts.grid {
            (cell_s * 0.04).clamp(1.0, 4.0)
        } else {
            0.0
        };
        let cell_draw = (cell_s - gap).max(1.0);

        for r in 0..rows {
            for c in 0..cols {
                let idx = r * cols + c;
                let tile = data[idx]; // which cell sits at grid position (r, c)
                let (src_col, src_row) = (tile % cols, tile / cols);
                let x = ox + c as f32 * cell_s + gap * 0.5;
                let y = oy + r as f32 * cell_s + gap * 0.5;

                // Half-texel inset keeps linear filtering from bleeding into the
                // neighbouring cell, so flush cells form a seamless image.
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

        // Highlight the cells currently being compared / written.
        if let Some((a, b)) = highlight {
            for idx in [a, b] {
                let (r, c) = (idx / cols, idx % cols);
                let x = ox + c as f32 * cell_s + gap * 0.5;
                let y = oy + r as f32 * cell_s + gap * 0.5;
                draw_rectangle(x, y, cell_draw, cell_draw, Color::from_rgba(255, 224, 60, 90));
            }
        }

        let status = if done { "  SORTED" } else { "" };
        draw_text(
            format!(
                "{}   |   grid {cols} x {rows}   |   key: {}",
                algo.name(),
                key.name()
            ),
            20.0,
            38.0,
            27.0,
            WHITE,
        );
        draw_text(
            format!(
                "step {cursor} / {}{status}   |   {speed} events/frame{}",
                events.len(),
                if paused { "   [paused]" } else { "" }
            ),
            20.0,
            72.0,
            19.0,
            LIGHTGRAY,
        );
        draw_text(
            "[Left/Right] algorithm   [R] reshuffle   [Space] pause   [Up/Down] speed   [Q/Esc] quit",
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