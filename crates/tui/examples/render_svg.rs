//! Renders raw terminal output as an SVG "terminal screenshot".
//!
//! `scripts/screenshots.py` drives the real `anthrex` client in a pseudo-terminal and
//! saves every byte the client wrote; this example replays those bytes through the
//! `vt100` crate (the same parser the client uses for its panes) and draws the final
//! screen cell by cell: colours, bold, italic, underline, inverse and dim, with
//! box-drawing and block characters drawn as shapes so frames join up whatever font
//! the viewer has. The result sits in a rounded window frame with a title bar.
//!
//! ```text
//! cargo run -p anthrex-tui --example render_svg -- \
//!     --rows 40 --cols 140 --title "anthrex" capture.bin out.svg [--text]
//! ```
//!
//! `--text` also prints the screen's text to stdout, for a quick look without a
//! browser. See `docs/screenshots.md`.

use std::fmt::Write as _;
use std::process::ExitCode;

const FONT_SIZE: f32 = 14.0;
const CELL_W: f32 = 8.4;
const CELL_H: f32 = 18.0;
const BASELINE: f32 = 13.5;
const PAD: f32 = 16.0;
const TITLE_H: f32 = 34.0;
const MARGIN: f32 = 24.0;
const BG: Rgb = Rgb(0x1e, 0x1e, 0x2e);
const FG: Rgb = Rgb(0xcd, 0xd6, 0xf4);
const FONT: &str = "ui-monospace, 'SF Mono', SFMono-Regular, Menlo, Monaco, 'Cascadia Mono', \
                    Consolas, 'DejaVu Sans Mono', 'Liberation Mono', monospace";

/// The 16 ANSI colours (Catppuccin Mocha), which anthrex's own default accent matches.
const ANSI: [Rgb; 16] = [
    Rgb(0x45, 0x47, 0x5a),
    Rgb(0xf3, 0x8b, 0xa8),
    Rgb(0xa6, 0xe3, 0xa1),
    Rgb(0xf9, 0xe2, 0xaf),
    Rgb(0x89, 0xb4, 0xfa),
    Rgb(0xf5, 0xc2, 0xe7),
    Rgb(0x94, 0xe2, 0xd5),
    Rgb(0xba, 0xc2, 0xde),
    Rgb(0x58, 0x5b, 0x70),
    Rgb(0xf3, 0x8b, 0xa8),
    Rgb(0xa6, 0xe3, 0xa1),
    Rgb(0xf9, 0xe2, 0xaf),
    Rgb(0x89, 0xb4, 0xfa),
    Rgb(0xf5, 0xc2, 0xe7),
    Rgb(0x94, 0xe2, 0xd5),
    Rgb(0xa6, 0xad, 0xc8),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rgb(u8, u8, u8);

impl Rgb {
    fn hex(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
}

fn indexed(i: u8) -> Rgb {
    match i {
        0..=15 => ANSI[usize::from(i)],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            Rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        _ => {
            let v = 8 + (i - 232) * 10;
            Rgb(v, v, v)
        }
    }
}

fn color(c: vt100::Color, default: Rgb) -> Rgb {
    match c {
        vt100::Color::Default => default,
        vt100::Color::Idx(i) => indexed(i),
        vt100::Color::Rgb(r, g, b) => Rgb(r, g, b),
    }
}

/// One cell, resolved: its text and the colours and attributes it is drawn with.
#[derive(Debug, Clone, PartialEq)]
struct Cell {
    text: String,
    fg: Rgb,
    bg: Rgb,
    bold: bool,
    italic: bool,
    underline: bool,
    dim: bool,
    wide: bool,
}

impl Cell {
    fn same_style(&self, other: &Cell) -> bool {
        (self.fg, self.bold, self.italic, self.underline, self.dim)
            == (
                other.fg,
                other.bold,
                other.italic,
                other.underline,
                other.dim,
            )
    }
}

fn cells(screen: &vt100::Screen, rows: u16, cols: u16) -> Vec<Vec<Option<Cell>>> {
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| {
                    let cell = screen.cell(r, c)?;
                    if cell.is_wide_continuation() {
                        return None;
                    }
                    let (mut fg, mut bg) = (color(cell.fgcolor(), FG), color(cell.bgcolor(), BG));
                    if cell.inverse() {
                        std::mem::swap(&mut fg, &mut bg);
                    }
                    Some(Cell {
                        text: cell.contents().to_string(),
                        fg,
                        bg,
                        bold: cell.bold(),
                        italic: cell.italic(),
                        underline: cell.underline(),
                        dim: cell.dim(),
                        wide: cell.is_wide(),
                    })
                })
                .collect()
        })
        .collect()
}

/// A box-drawing character's arms: up, down, left, right; 0 none, 1 light, 2 heavy.
/// `None` for anything not drawn as lines (it falls back to the font).
fn arms(ch: char) -> Option<[u8; 4]> {
    Some(match ch {
        '─' => [0, 0, 1, 1],
        '━' => [0, 0, 2, 2],
        '│' => [1, 1, 0, 0],
        '┃' => [2, 2, 0, 0],
        '┌' => [0, 1, 0, 1],
        '┐' => [0, 1, 1, 0],
        '└' => [1, 0, 0, 1],
        '┘' => [1, 0, 1, 0],
        '├' => [1, 1, 0, 1],
        '┤' => [1, 1, 1, 0],
        '┬' => [0, 1, 1, 1],
        '┴' => [1, 0, 1, 1],
        '┼' => [1, 1, 1, 1],
        '┏' => [0, 2, 0, 2],
        '┓' => [0, 2, 2, 0],
        '┗' => [2, 0, 0, 2],
        '┛' => [2, 0, 2, 0],
        '┣' => [2, 2, 0, 2],
        '┫' => [2, 2, 2, 0],
        '┳' => [0, 2, 2, 2],
        '┻' => [2, 0, 2, 2],
        '╋' => [2, 2, 2, 2],
        '╴' => [0, 0, 1, 0],
        '╵' => [1, 0, 0, 0],
        '╶' => [0, 0, 0, 1],
        '╷' => [0, 1, 0, 0],
        _ => return None,
    })
}

fn stroke(weight: u8) -> f32 {
    if weight == 2 { 2.4 } else { 1.2 }
}

/// The SVG path of a box-drawing character in the cell at `(x, y)`, with its stroke
/// width, or `None` when the font should draw it.
fn box_path(ch: char, x: f32, y: f32) -> Option<(String, f32)> {
    let (cx, cy) = (x + CELL_W / 2.0, y + CELL_H / 2.0);
    let r = CELL_W / 2.0;
    let rounded = |d: &str| Some((d.to_string(), 1.2));
    match ch {
        '╭' => {
            return rounded(&format!(
                "M{cx} {b}V{s}A{r} {r} 0 0 1 {e} {cy}H{right}",
                b = y + CELL_H,
                s = cy + r,
                e = cx + r,
                right = x + CELL_W
            ));
        }
        '╮' => {
            return rounded(&format!(
                "M{x} {cy}H{s}A{r} {r} 0 0 1 {cx} {e}V{b}",
                s = cx - r,
                e = cy + r,
                b = y + CELL_H
            ));
        }
        '╯' => {
            return rounded(&format!(
                "M{cx} {y}V{s}A{r} {r} 0 0 1 {e} {cy}H{x}",
                s = cy - r,
                e = cx - r
            ));
        }
        '╰' => {
            return rounded(&format!(
                "M{cx} {y}V{s}A{r} {r} 0 0 0 {e} {cy}H{right}",
                s = cy - r,
                e = cx + r,
                right = x + CELL_W
            ));
        }
        _ => {}
    }
    let [up, down, left, right] = arms(ch)?;
    let weight = up.max(down).max(left).max(right);
    let mut d = String::new();
    if up > 0 || down > 0 {
        let top = if up > 0 { y } else { cy };
        let bottom = if down > 0 { y + CELL_H } else { cy };
        let _ = write!(d, "M{cx} {top}V{bottom}");
    }
    if left > 0 || right > 0 {
        let start = if left > 0 { x } else { cx };
        let end = if right > 0 { x + CELL_W } else { cx };
        let _ = write!(d, "M{start} {cy}H{end}");
    }
    Some((d, stroke(weight)))
}

/// A block element's rectangle relative to its cell (x, y, w, h as fractions) and its
/// opacity, or `None`.
fn block(ch: char) -> Option<(f32, f32, f32, f32, f32)> {
    Some(match ch {
        '█' => (0.0, 0.0, 1.0, 1.0, 1.0),
        '▀' => (0.0, 0.0, 1.0, 0.5, 1.0),
        '▄' => (0.0, 0.5, 1.0, 0.5, 1.0),
        '▌' => (0.0, 0.0, 0.5, 1.0, 1.0),
        '▐' => (0.5, 0.0, 0.5, 1.0, 1.0),
        '░' => (0.0, 0.0, 1.0, 1.0, 0.25),
        '▒' => (0.0, 0.0, 1.0, 1.0, 0.5),
        '▓' => (0.0, 0.0, 1.0, 1.0, 0.75),
        _ => return None,
    })
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if (c as u32) < 0x20 => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The whole SVG document for a `rows` × `cols` grid of resolved cells.
fn render(grid: &[Vec<Option<Cell>>], rows: u16, cols: u16, title: &str) -> String {
    let term_w = f32::from(cols) * CELL_W;
    let term_h = f32::from(rows) * CELL_H;
    let win_w = term_w + 2.0 * PAD;
    let win_h = term_h + TITLE_H + PAD;
    let (width, height) = (win_w + 2.0 * MARGIN, win_h + 2.0 * MARGIN);
    let (ox, oy) = (MARGIN + PAD, MARGIN + TITLE_H);

    let mut svg = String::new();
    let _ = write!(
        svg,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" \
         viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"{t}\">\
         <title>{t}</title>\
         <defs><filter id=\"shadow\" x=\"-5%\" y=\"-5%\" width=\"110%\" height=\"115%\">\
         <feDropShadow dx=\"0\" dy=\"6\" stdDeviation=\"8\" flood-color=\"#000\" \
         flood-opacity=\"0.35\"/></filter>\
         <clipPath id=\"term\"><rect x=\"{ox}\" y=\"{oy}\" width=\"{term_w}\" \
         height=\"{term_h}\"/></clipPath></defs>",
        t = escape(title)
    );
    let _ = write!(
        svg,
        "<rect x=\"{MARGIN}\" y=\"{MARGIN}\" width=\"{win_w}\" height=\"{win_h}\" rx=\"10\" \
         fill=\"{bg}\" stroke=\"#45475a\" stroke-width=\"1\" filter=\"url(#shadow)\"/>\
         <path d=\"M{MARGIN} {tb}V{r0}A10 10 0 0 1 {r1} {MARGIN}H{r2}A10 10 0 0 1 {right} \
         {r0}V{tb}Z\" fill=\"#181825\"/>\
         <line x1=\"{MARGIN}\" y1=\"{tb}\" x2=\"{right}\" y2=\"{tb}\" stroke=\"#313244\"/>",
        bg = BG.hex(),
        tb = MARGIN + TITLE_H - 8.0,
        r0 = MARGIN + 10.0,
        r1 = MARGIN + 10.0,
        r2 = MARGIN + win_w - 10.0,
        right = MARGIN + win_w,
    );
    for (i, dot) in ["#f38ba8", "#f9e2af", "#a6e3a1"].iter().enumerate() {
        let _ = write!(
            svg,
            "<circle cx=\"{}\" cy=\"{}\" r=\"6\" fill=\"{dot}\"/>",
            MARGIN + 20.0 + 20.0 * i as f32,
            MARGIN + (TITLE_H - 8.0) / 2.0
        );
    }
    let _ = write!(
        svg,
        "<text x=\"{}\" y=\"{}\" text-anchor=\"middle\" fill=\"#a6adc8\" \
         font-family=\"-apple-system, BlinkMacSystemFont, 'Segoe UI', Helvetica, Arial, \
         sans-serif\" font-size=\"13\">{}</text>",
        MARGIN + win_w / 2.0,
        MARGIN + (TITLE_H - 8.0) / 2.0 + 4.5,
        escape(title)
    );

    svg.push_str("<g clip-path=\"url(#term)\">");
    // Backgrounds: one rectangle per run of equal, non-default background.
    for (r, row) in grid.iter().enumerate() {
        let y = oy + r as f32 * CELL_H;
        let mut c = 0;
        while c < row.len() {
            let Some(bg) = row[c].as_ref().map(|cell| cell.bg).filter(|bg| *bg != BG) else {
                c += 1;
                continue;
            };
            let start = c;
            while c < row.len()
                && row[c].as_ref().map_or_else(
                    || c > 0 && row[c - 1].as_ref().is_some_and(|p| p.bg == bg),
                    |cell| cell.bg == bg,
                )
            {
                c += 1;
            }
            let _ = write!(
                svg,
                "<rect x=\"{}\" y=\"{y}\" width=\"{}\" height=\"{CELL_H}\" fill=\"{}\"/>",
                ox + start as f32 * CELL_W,
                (c - start) as f32 * CELL_W + 0.4,
                bg.hex()
            );
        }
    }

    // Lines and blocks drawn as shapes.
    for (r, row) in grid.iter().enumerate() {
        let y = oy + r as f32 * CELL_H;
        for (c, cell) in row.iter().enumerate() {
            let Some(cell) = cell else { continue };
            let Some(ch) = single_char(&cell.text) else {
                continue;
            };
            let x = ox + c as f32 * CELL_W;
            let opacity = if cell.dim { 0.6 } else { 1.0 };
            if let Some((d, width)) = box_path(ch, x, y) {
                let _ = write!(
                    svg,
                    "<path d=\"{d}\" fill=\"none\" stroke=\"{}\" stroke-width=\"{width}\" \
                     stroke-linecap=\"square\" opacity=\"{opacity}\"/>",
                    cell.fg.hex()
                );
            } else if let Some((bx, by, bw, bh, alpha)) = block(ch) {
                let _ = write!(
                    svg,
                    "<rect x=\"{}\" y=\"{}\" width=\"{}\" height=\"{}\" fill=\"{}\" \
                     opacity=\"{}\"/>",
                    x + bx * CELL_W,
                    y + by * CELL_H,
                    bw * CELL_W + 0.2,
                    bh * CELL_H + 0.2,
                    cell.fg.hex(),
                    alpha * opacity
                );
            }
        }
    }

    // Text: one <text> per run of equally styled glyphs, each glyph at its own cell.
    let _ = write!(
        svg,
        "<g font-family=\"{FONT}\" font-size=\"{FONT_SIZE}\" xml:space=\"preserve\">"
    );
    for (r, row) in grid.iter().enumerate() {
        let y = oy + r as f32 * CELL_H + BASELINE;
        let mut c = 0;
        while c < row.len() {
            let Some(first) = row[c].as_ref().filter(|cell| drawn_as_text(cell)) else {
                c += 1;
                continue;
            };
            let mut xs = Vec::new();
            let mut text = String::new();
            while c < row.len() {
                match row[c].as_ref() {
                    Some(cell) if drawn_as_text(cell) && cell.same_style(first) => {
                        xs.push(format!("{:.1}", ox + c as f32 * CELL_W));
                        // One x per character: a cell holding a combining sequence
                        // takes the first one only.
                        let mut chars = cell.text.chars();
                        if let Some(ch) = chars.next() {
                            text.push(ch);
                        }
                        for extra in chars {
                            text.push(extra);
                            xs.push(format!("{:.1}", ox + c as f32 * CELL_W));
                        }
                        c += 1;
                    }
                    None if c > 0 && row[c - 1].as_ref().is_some_and(|p| p.wide) => c += 1,
                    _ => break,
                }
            }
            let mut attrs = format!("fill=\"{}\"", first.fg.hex());
            if first.bold {
                attrs.push_str(" font-weight=\"bold\"");
            }
            if first.italic {
                attrs.push_str(" font-style=\"italic\"");
            }
            if first.underline {
                attrs.push_str(" text-decoration=\"underline\"");
            }
            if first.dim {
                attrs.push_str(" opacity=\"0.6\"");
            }
            let _ = write!(
                svg,
                "<text x=\"{}\" y=\"{y}\" {attrs}>{}</text>",
                xs.join(" "),
                escape(&text)
            );
        }
    }
    svg.push_str("</g></g></svg>\n");
    svg
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    chars.next().is_none().then_some(ch)
}

/// Whether a cell is drawn with the font: it has visible text that is neither a
/// box-drawing line nor a block.
fn drawn_as_text(cell: &Cell) -> bool {
    if cell.text.trim().is_empty() {
        return false;
    }
    match single_char(&cell.text) {
        Some(ch) => box_path(ch, 0.0, 0.0).is_none() && block(ch).is_none(),
        None => true,
    }
}

struct Args {
    rows: u16,
    cols: u16,
    title: String,
    input: String,
    output: String,
    text: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut args = std::env::args().skip(1);
    let (mut rows, mut cols, mut title, mut text) = (40, 140, "anthrex".to_string(), false);
    let mut paths = Vec::new();
    while let Some(arg) = args.next() {
        let mut value = |name: &str| args.next().ok_or(format!("{name} needs a value"));
        match arg.as_str() {
            "--rows" => {
                rows = value("--rows")?
                    .parse()
                    .map_err(|e| format!("--rows: {e}"))?
            }
            "--cols" => {
                cols = value("--cols")?
                    .parse()
                    .map_err(|e| format!("--cols: {e}"))?
            }
            "--title" => title = value("--title")?,
            "--text" => text = true,
            other if other.starts_with("--") => return Err(format!("unknown option {other}")),
            other => paths.push(other.to_string()),
        }
    }
    let [input, output]: [String; 2] = paths
        .try_into()
        .map_err(|_| "usage: render_svg [--rows N] [--cols N] [--title T] [--text] IN OUT")?;
    Ok(Args {
        rows,
        cols,
        title,
        input,
        output,
        text,
    })
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(message) => {
            eprintln!("render_svg: {message}");
            return ExitCode::from(2);
        }
    };
    let bytes = match std::fs::read(&args.input) {
        Ok(bytes) => bytes,
        Err(error) => {
            eprintln!("render_svg: {}: {error}", args.input);
            return ExitCode::FAILURE;
        }
    };
    let mut parser = vt100::Parser::new(args.rows, args.cols, 0);
    parser.process(&bytes);
    let screen = parser.screen();
    if args.text {
        println!("{}", screen.contents());
    }
    let grid = cells(screen, args.rows, args.cols);
    let svg = render(&grid, args.rows, args.cols, &args.title);
    if let Err(error) = std::fs::write(&args.output, svg) {
        eprintln!("render_svg: {}: {error}", args.output);
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}
