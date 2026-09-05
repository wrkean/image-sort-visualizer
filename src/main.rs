mod grid;
mod sort;

use std::cmp::Ordering;
use std::env;

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
    --algo NAME     sorting algorithm      (default bubble)
                    one of: bubble, insertion, selection, quick, heap, merge
    --key WHAT      what the cells are sorted by   (default index)
                    index  -> cell's original grid position
                             (sorting reconstructs the original image)
                    luma   -> cell's average brightness
    --speed N       events applied per frame (default: auto-sized so the
                    animation takes roughly ten seconds)
    --seed N        reproducible initial shuffle

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
    cols: usize,
    rows: usize,
    algo: Algo,
    key: KeyMode,
    speed: Option<usize>,
    seed: Option<u64>,
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
            cols: 20,
            rows: 20,
            algo: Algo::Bubble,
            key: KeyMode::Index,
            speed: None,
            seed: None,
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
                    o.cols = need!("--cols").parse().map_err(|_| "invalid --cols value")?;
                }
                "--rows" => {
                    o.rows = need!("--rows").parse().map_err(|_| "invalid --rows value")?;
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

        if o.cols == 0 {
            return Err("--cols must be >= 1".into());
        }
        if o.rows == 0 {
            return Err("--rows must be >= 1".into());
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

    let grid = Grid::new(&img, opts.cols, opts.rows);
    let cols = grid.cols;
    let rows = grid.rows;
    let n = cols * rows;

    let mut base = shuffled(n, opts.seed); // initial parallel-permuted layout
    let mut algo = opts.algo;
    let key = opts.key;

    let mut events = compute_events(algo, &base, &grid, key);
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
            events = compute_events(algo, &base, &grid, key);
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

        let gap = (cell_s * 0.04).clamp(1.0, 4.0);
        let cell_draw = (cell_s - gap).max(1.0);

        for r in 0..rows {
            for c in 0..cols {
                let idx = r * cols + c;
                let tile = data[idx]; // which cell sits at grid position (r, c)
                let (src_col, src_row) = (tile % cols, tile / cols);
                let x = ox + c as f32 * cell_s + gap * 0.5;
                let y = oy + r as f32 * cell_s + gap * 0.5;

                draw_texture_ex(
                    &grid.texture,
                    x,
                    y,
                    WHITE,
                    DrawTextureParams {
                        source: Some(Rect::new(
                            src_col as f32 * grid.cell_w,
                            src_row as f32 * grid.cell_h,
                            grid.cell_w,
                            grid.cell_h,
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