use std::io::{self, Write};
use std::time::{Duration, Instant};

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::{Color, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use rand::{rngs::StdRng, Rng, SeedableRng};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Rgb {
    r: u8,
    g: u8,
    b: u8,
}

#[derive(Clone, Copy)]
enum GlyphSet {
    AsciiSlashes,
    UnicodeSlashes,
    ExpandedAscii,
    BoxDrawing,
    UnicodeCorners,
}

impl GlyphSet {
    fn next(self) -> Self {
        match self {
            GlyphSet::AsciiSlashes => GlyphSet::UnicodeSlashes,
            GlyphSet::UnicodeSlashes => GlyphSet::ExpandedAscii,
            GlyphSet::ExpandedAscii => GlyphSet::BoxDrawing,
            GlyphSet::BoxDrawing => GlyphSet::UnicodeCorners,
            GlyphSet::UnicodeCorners => GlyphSet::AsciiSlashes,
        }
    }

    fn glyphs(self) -> &'static [char] {
        match self {
            GlyphSet::AsciiSlashes => &['\\', '/'],
            GlyphSet::UnicodeSlashes => &['╲', '╱'],
            GlyphSet::ExpandedAscii => &['|', '_', '+', 'X'],
            GlyphSet::BoxDrawing => &['┼', '┌', '┐', '└', '┘', '─', '│'],
            GlyphSet::UnicodeCorners => &['╱', '╲', '╳', '╭', '╮', '╰', '╯'],
        }
    }

    fn biased_pair(self) -> Option<(char, char)> {
        match self {
            GlyphSet::AsciiSlashes => Some(('\\', '/')),
            GlyphSet::UnicodeSlashes => Some(('╲', '╱')),
            _ => None,
        }
    }

    fn is_member(self, ch: char) -> bool {
        self.glyphs().iter().any(|&g| g == ch)
    }
}

#[derive(Clone, Copy)]
enum BiasMode {
    Fair,
    Drift70,
    Drift90,
}

impl BiasMode {
    fn next(self) -> Self {
        match self {
            BiasMode::Fair => BiasMode::Drift70,
            BiasMode::Drift70 => BiasMode::Drift90,
            BiasMode::Drift90 => BiasMode::Fair,
        }
    }
}

fn glyph_label(glyphs: GlyphSet) -> &'static str {
    match glyphs {
        GlyphSet::AsciiSlashes => "slashes (ascii)",
        GlyphSet::UnicodeSlashes => "slashes (unicode)",
        GlyphSet::ExpandedAscii => "expanded ascii",
        GlyphSet::BoxDrawing => "box drawing",
        GlyphSet::UnicodeCorners => "unicode corners",
    }
}

fn bias_label(mode: BiasMode) -> &'static str {
    match mode {
        BiasMode::Fair => "fair 50/50",
        BiasMode::Drift70 => "drift 70/30",
        BiasMode::Drift90 => "drift 90/10",
    }
}

fn gen_label(mode: GenMode) -> &'static str {
    match mode {
        GenMode::Independent => "independent",
        GenMode::Markov => "markov",
    }
}

#[derive(Clone, Copy)]
enum GenMode {
    Independent,
    Markov,
}

impl GenMode {
    fn next(self) -> Self {
        match self {
            GenMode::Independent => GenMode::Markov,
            GenMode::Markov => GenMode::Independent,
        }
    }
}

struct DiffRenderer {
    w: u16,
    h: u16,
    last: Vec<char>,
    last_luma: Vec<u8>,
    last_fg: Vec<Rgb>,
    last_bg: Vec<Rgb>,
}

impl DiffRenderer {
    fn new(w: u16, h: u16) -> Self {
        let len = (w as usize) * (h as usize);
        Self {
            w,
            h,
            last: vec!['\0'; len],
            last_luma: vec![0; len],
            last_fg: vec![Rgb { r: 0, g: 0, b: 0 }; len],
            last_bg: vec![Rgb { r: 0, g: 0, b: 0 }; len],
        }
    }

    fn resize(&mut self, w: u16, h: u16) {
        let len = (w as usize) * (h as usize);
        self.w = w;
        self.h = h;
        self.last = vec!['\0'; len];
        self.last_luma = vec![0; len];
        self.last_fg = vec![Rgb { r: 0, g: 0, b: 0 }; len];
        self.last_bg = vec![Rgb { r: 0, g: 0, b: 0 }; len];
    }

    fn row_luma(&self, y: u16) -> u8 {
        if self.h <= 1 {
            return 255;
        }
        let mid = (self.h - 1) as f32 / 2.0;
        let dist = ((y as f32) - mid).abs();
        let t = (1.0 - (dist / mid)).clamp(0.0, 1.0);
        (t * 255.0).round() as u8
    }

    fn draw(
        &mut self,
        out: &mut impl Write,
        buf: &[char],
        fg_base: Rgb,
        bg: Rgb,
    ) -> io::Result<()> {
        debug_assert_eq!(buf.len(), (self.w as usize) * (self.h as usize));

        queue!(out, BeginSynchronizedUpdate)?;
        for y in 0..self.h {
            let luma = self.row_luma(y);
            let fg = scale_rgb(fg_base, luma);
            let row = y as usize * self.w as usize;
            for x in 0..self.w {
                let i = row + x as usize;
                let c = buf[i];
                if self.last[i] != c
                    || self.last_luma[i] != luma
                    || self.last_fg[i] != fg
                    || self.last_bg[i] != bg
                {
                    self.last[i] = c;
                    self.last_luma[i] = luma;
                    self.last_fg[i] = fg;
                    self.last_bg[i] = bg;
                    queue!(
                        out,
                        cursor::MoveTo(x, y),
                        SetForegroundColor(Color::Rgb {
                            r: fg.r,
                            g: fg.g,
                            b: fg.b
                        }),
                        SetBackgroundColor(Color::Rgb {
                            r: bg.r,
                            g: bg.g,
                            b: bg.b
                        }),
                        crossterm::style::Print(c)
                    )?;
                }
            }
        }
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        Ok(())
    }

    fn full_clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        queue!(out, BeginSynchronizedUpdate)?;
        queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0))?;
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        self.last.fill('\0');
        self.last_luma.fill(0);
        self.last_fg.fill(Rgb { r: 0, g: 0, b: 0 });
        self.last_bg.fill(Rgb { r: 0, g: 0, b: 0 });
        Ok(())
    }
}

fn idx(w: u16, x: u16, y: u16) -> usize {
    (y as usize) * (w as usize) + (x as usize)
}

fn scroll_up(buf: &mut [char], w: u16, h: u16) {
    let w = w as usize;
    let h = h as usize;
    if w == 0 || h == 0 {
        return;
    }

    let len = w * h;
    buf.copy_within(w..len, 0);

    let last = (h - 1) * w;
    buf[last..last + w].fill(' ');
}

fn bias_prob(mode: BiasMode, t: f32) -> Option<f32> {
    match mode {
        BiasMode::Fair => None,
        BiasMode::Drift70 => Some((0.7 + 0.2 * (t * 0.12).sin()).clamp(0.05, 0.95)),
        BiasMode::Drift90 => Some((0.9 + 0.1 * (t * 0.10).sin()).clamp(0.05, 0.99)),
    }
}

fn sample_glyph_independent(rng: &mut StdRng, glyphs: GlyphSet, bias_p: Option<f32>) -> char {
    if let Some((a, b)) = glyphs.biased_pair() {
        let p = bias_p.unwrap_or(0.5) as f64;
        if rng.gen_bool(p) {
            a
        } else {
            b
        }
    } else {
        let set = glyphs.glyphs();
        set[rng.gen_range(0..set.len())]
    }
}

/// Markov-ish choice:
/// - If left/above exist and are in the current glyph set, prefer them.
/// - For slash pairs, also prefer "continuing" the neighbor.
/// - Otherwise fall back to independent sampling.
fn sample_glyph_markov(
    rng: &mut StdRng,
    glyphs: GlyphSet,
    bias_p: Option<f32>,
    buf: &[char],
    w: u16,
    h: u16,
    x: u16,
    y: u16,
) -> char {
    let mut candidates: [char; 2] = ['\0', '\0'];
    let mut n = 0usize;

    if x > 0 {
        let left = buf[idx(w, x - 1, y)];
        if glyphs.is_member(left) {
            candidates[n] = left;
            n += 1;
        }
    }
    if y > 0 {
        let up = buf[idx(w, x, y - 1)];
        if glyphs.is_member(up) && (n == 0 || candidates[0] != up) {
            candidates[n] = up;
            n += 1;
        }
    }

    // If we have neighbor candidates, pick them with high probability.
    // Tunables:
    // - neighbor_weight: how sticky the patterns are
    // - conflict_flip: when both neighbors exist, chance to *not* match either (adds texture)
    let neighbor_weight: f64 = 0.82;
    let conflict_flip: f64 = 0.12;

    if n == 1 && rng.gen_bool(neighbor_weight) {
        return candidates[0];
    }
    if n == 2 {
        // Usually choose one of them, occasionally fall through.
        if rng.gen_bool(1.0 - conflict_flip) {
            return candidates[rng.gen_range(0..2)];
        }
    }

    // For slash pairs, an extra hint: if left is one of the pair, follow it more often.
    if let Some((a, b)) = glyphs.biased_pair() {
        if x > 0 {
            let left = buf[idx(w, x - 1, y)];
            if left == a || left == b {
                if rng.gen_bool(0.75) {
                    return left;
                }
            }
        }
        // Otherwise use bias probability (drift/fair)
        let p = bias_p.unwrap_or(0.5) as f64;
        return if rng.gen_bool(p) { a } else { b };
    }

    // Generic fallback
    sample_glyph_independent(rng, glyphs, bias_p)
}

fn seed_screen(buf: &mut [char], rng: &mut StdRng, glyphs: GlyphSet, bias_mode: BiasMode, t: f32) {
    let bias_p = bias_prob(bias_mode, t);
    for cell in buf.iter_mut() {
        *cell = sample_glyph_independent(rng, glyphs, bias_p);
    }
}

fn scale_rgb(base: Rgb, luma: u8) -> Rgb {
    let k = (luma as f32) / 255.0;
    Rgb {
        r: ((base.r as f32) * k).round() as u8,
        g: ((base.g as f32) * k).round() as u8,
        b: ((base.b as f32) * k).round() as u8,
    }
}

fn draw_hud(out: &mut impl Write, w: u16, h: u16, lines: &[String]) -> io::Result<()> {
    if w == 0 || h == 0 || lines.is_empty() {
        return Ok(());
    }
    let fg = Color::Rgb {
        r: 240,
        g: 240,
        b: 240,
    };
    let bg = Color::Rgb { r: 10, g: 10, b: 10 };
    queue!(out, BeginSynchronizedUpdate)?;
    for (row, line) in lines.iter().enumerate() {
        if row >= h as usize {
            break;
        }
        queue!(
            out,
            cursor::MoveTo(0, row as u16),
            SetForegroundColor(fg),
            SetBackgroundColor(bg)
        )?;
        let mut count = 0usize;
        for ch in line.chars() {
            if count >= w as usize {
                break;
            }
            queue!(out, crossterm::style::Print(ch))?;
            count += 1;
        }
        while count < w as usize {
            queue!(out, crossterm::style::Print(' '))?;
            count += 1;
        }
    }
    queue!(out, EndSynchronizedUpdate)?;
    out.flush()?;
    Ok(())
}

fn render_frame(
    out: &mut impl Write,
    r: &mut DiffRenderer,
    buf: &[char],
    fg: Rgb,
    bg: Rgb,
    show_help: bool,
    w: u16,
    h: u16,
    glyphs: GlyphSet,
    bias_mode: BiasMode,
    gen_mode: GenMode,
) -> io::Result<()> {
    r.draw(out, buf, fg, bg)?;
    if show_help {
        let line1 = "TenPrint  |  Q/Esc quit  Space pause  H help";
        let line2 = format!(
            "C glyphs: {}  M bias: {}  S mode: {}",
            glyph_label(glyphs),
            bias_label(bias_mode),
            gen_label(gen_mode)
        );
        let line3 = "F/B color  +/- speed  R reset";
        draw_hud(out, w, h, &[line1.to_string(), line2, line3.to_string()])?;
    }
    Ok(())
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;

    let result = run(&mut out);

    execute!(out, EnableLineWrap, cursor::Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    result
}

fn run(out: &mut impl Write) -> io::Result<()> {
    let mut glyphs = GlyphSet::UnicodeSlashes;
    let mut bias_mode = BiasMode::Fair;
    let mut gen_mode = GenMode::Independent;

    let mut paused = false;
    let mut show_help = true;

    let fg_palette = [
        Rgb {
            r: 255,
            g: 255,
            b: 255,
        },
        Rgb {
            r: 120,
            g: 220,
            b: 255,
        },
        Rgb {
            r: 180,
            g: 255,
            b: 160,
        },
        Rgb {
            r: 255,
            g: 200,
            b: 120,
        },
    ];
    let bg_palette = [
        Rgb { r: 0, g: 0, b: 0 },
        Rgb {
            r: 12,
            g: 18,
            b: 28,
        },
        Rgb { r: 24, g: 10, b: 6 },
        Rgb { r: 8, g: 16, b: 10 },
    ];
    let mut fg_idx = 0usize;
    let mut bg_idx = 0usize;

    let mut frame_ms: u64 = 10;
    let mut rng = StdRng::from_entropy();

    let (mut w, mut h) = terminal::size()?;
    if w == 0 || h == 0 {
        return Ok(());
    }

    let mut buf = vec![' '; (w as usize) * (h as usize)];
    let mut r = DiffRenderer::new(w, h);
    r.full_clear(out)?;

    let mut x: u16 = 0;
    let mut y: u16 = 0;

    let mut last_tick = Instant::now();
    let start_time = Instant::now();

    seed_screen(&mut buf, &mut rng, glyphs, bias_mode, 0.0);
    render_frame(
        out,
        &mut r,
        &buf,
        fg_palette[fg_idx],
        bg_palette[bg_idx],
        show_help,
        w,
        h,
        glyphs,
        bias_mode,
        gen_mode,
    )?;
    x = 0;
    y = h.saturating_sub(1);

    loop {
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Resize(nw, nh) => {
                    w = nw.max(1);
                    h = nh.max(1);
                    buf = vec![' '; (w as usize) * (h as usize)];
                    r.resize(w, h);
                    r.full_clear(out)?;
                    seed_screen(
                        &mut buf,
                        &mut rng,
                        glyphs,
                        bias_mode,
                        start_time.elapsed().as_secs_f32(),
                    );
                    render_frame(
                        out,
                        &mut r,
                        &buf,
                        fg_palette[fg_idx],
                        bg_palette[bg_idx],
                        show_help,
                        w,
                        h,
                        glyphs,
                        bias_mode,
                        gen_mode,
                    )?;
                    x = 0;
                    y = h.saturating_sub(1);
                }
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => return Ok(()),
                    KeyCode::Char(' ') => paused = !paused,
                    KeyCode::Char('h') | KeyCode::Char('H') => {
                        show_help = !show_help;
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                    }
                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        rng = StdRng::from_entropy();
                        seed_screen(
                            &mut buf,
                            &mut rng,
                            glyphs,
                            bias_mode,
                            start_time.elapsed().as_secs_f32(),
                        );
                        r.full_clear(out)?;
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                        x = 0;
                        y = h.saturating_sub(1);
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        glyphs = glyphs.next();
                        seed_screen(
                            &mut buf,
                            &mut rng,
                            glyphs,
                            bias_mode,
                            start_time.elapsed().as_secs_f32(),
                        );
                        r.full_clear(out)?;
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                        x = 0;
                        y = h.saturating_sub(1);
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        bias_mode = bias_mode.next();
                        seed_screen(
                            &mut buf,
                            &mut rng,
                            glyphs,
                            bias_mode,
                            start_time.elapsed().as_secs_f32(),
                        );
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                        x = 0;
                        y = h.saturating_sub(1);
                    }
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        gen_mode = gen_mode.next();
                        // no reseed; mode change is more interesting live
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                    }
                    KeyCode::Char('f') | KeyCode::Char('F') => {
                        fg_idx = (fg_idx + 1) % fg_palette.len();
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                    }
                    KeyCode::Char('b') | KeyCode::Char('B') => {
                        bg_idx = (bg_idx + 1) % bg_palette.len();
                        render_frame(
                            out,
                            &mut r,
                            &buf,
                            fg_palette[fg_idx],
                            bg_palette[bg_idx],
                            show_help,
                            w,
                            h,
                            glyphs,
                            bias_mode,
                            gen_mode,
                        )?;
                    }
                    KeyCode::Char('+') | KeyCode::Char('=') => {
                        frame_ms = frame_ms.saturating_sub(1).max(1);
                    }
                    KeyCode::Char('-') | KeyCode::Char('_') => {
                        frame_ms = (frame_ms + 2).min(200);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        let now = Instant::now();
        if now.duration_since(last_tick) < Duration::from_millis(frame_ms) {
            std::thread::sleep(Duration::from_millis(1));
            continue;
        }
        last_tick = now;

        if !paused {
            let t = start_time.elapsed().as_secs_f32();
            let bias_p = bias_prob(bias_mode, t);

            let ch = match gen_mode {
                GenMode::Independent => sample_glyph_independent(&mut rng, glyphs, bias_p),
                GenMode::Markov => sample_glyph_markov(&mut rng, glyphs, bias_p, &buf, w, h, x, y),
            };

            buf[idx(w, x, y)] = ch;

            x += 1;
            if x >= w {
                x = 0;
                y += 1;

                if y >= h {
                    scroll_up(&mut buf, w, h);
                    y = h.saturating_sub(1);
                }
            }
        }

        render_frame(
            out,
            &mut r,
            &buf,
            fg_palette[fg_idx],
            bg_palette[bg_idx],
            show_help,
            w,
            h,
            glyphs,
            bias_mode,
            gen_mode,
        )?;
    }
}
