use crate::config::Settings;
use crate::model::{GameState, LifeStage, Mood, Scene};
use crossterm::{
    cursor,
    execute, queue,
    style::{Color, Print, ResetColor, SetBackgroundColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, Clear, ClearType, DisableLineWrap, EnableLineWrap,
        EndSynchronizedUpdate, EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use std::io::{self, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Cell {
    pub(crate) ch: char,
    pub(crate) fg: Color,
    pub(crate) bg: Color,
    pub(crate) bold: bool,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::White,
            bg: Color::Black,
            bold: false,
        }
    }
}

pub(crate) struct CellBuffer {
    pub(crate) w: u16,
    pub(crate) h: u16,
    pub(crate) cells: Vec<Cell>,
}

impl CellBuffer {
    pub(crate) fn new(w: u16, h: u16) -> Self {
        Self {
            w,
            h,
            cells: vec![Cell::default(); (w as usize) * (h as usize)],
        }
    }
    pub(crate) fn idx(&self, x: u16, y: u16) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }
    pub(crate) fn set(&mut self, x: u16, y: u16, c: Cell) {
        if x < self.w && y < self.h {
            let i = self.idx(x, y);
            self.cells[i] = c;
        }
    }
    pub(crate) fn clear(&mut self, bg: Color) {
        for c in &mut self.cells {
            c.ch = ' ';
            c.fg = Color::White;
            c.bg = bg;
            c.bold = false;
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Pixel {
    pub(crate) r: u8,
    pub(crate) g: u8,
    pub(crate) b: u8,
    pub(crate) a: u8,
}

pub(crate) struct PixelCanvas {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) px: Vec<Pixel>,
}

impl PixelCanvas {
    pub(crate) fn new(w: u32, h: u32) -> Self {
        Self {
            w,
            h,
            px: vec![Pixel::default(); (w as usize) * (h as usize)],
        }
    }
    pub(crate) fn idx(&self, x: u32, y: u32) -> usize {
        (y as usize) * (self.w as usize) + (x as usize)
    }
    pub(crate) fn clear(&mut self, p: Pixel) {
        self.px.fill(p);
    }
    fn blend_over(&mut self, x: i32, y: i32, src: Pixel) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as u32, y as u32);
        if x >= self.w || y >= self.h {
            return;
        }
        let i = self.idx(x, y);
        let dst = self.px[i];

        let sa = src.a as f32 / 255.0;
        let da = dst.a as f32 / 255.0;

        let out_a = sa + da * (1.0 - sa);
        if out_a <= 1e-6 {
            self.px[i] = Pixel::default();
            return;
        }

        let blend = |sc: u8, dc: u8| -> u8 {
            let sc = sc as f32 / 255.0;
            let dc = dc as f32 / 255.0;
            let out = (sc * sa + dc * da * (1.0 - sa)) / out_a;
            (out.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
        };

        self.px[i] = Pixel {
            r: blend(src.r, dst.r),
            g: blend(src.g, dst.g),
            b: blend(src.b, dst.b),
            a: (out_a.clamp(0.0, 1.0) * 255.0 + 0.5) as u8,
        };
    }
}

pub(crate) struct Terminal {
    pub(crate) out: io::Stdout,
    pub(crate) cols: u16,
    pub(crate) rows: u16,
    pub(crate) prev: CellBuffer,
    pub(crate) cur: CellBuffer,
    pub(crate) canvas: PixelCanvas,
}

impl Terminal {
    pub(crate) fn begin() -> anyhow::Result<Self> {
        let mut out = io::stdout();
        execute!(
            out,
            EnterAlternateScreen,
            cursor::Hide,
            DisableLineWrap,
            terminal::Clear(ClearType::All)
        )?;
        terminal::enable_raw_mode()?;

        let (cols, rows) = terminal::size()?;
        let prev = CellBuffer::new(cols, rows);
        let cur = CellBuffer::new(cols, rows);

        // Braille: 2×4 pixels per cell
        let canvas = PixelCanvas::new(cols as u32 * 2, rows as u32 * 4);

        Ok(Self {
            out,
            cols,
            rows,
            prev,
            cur,
            canvas,
        })
    }

    pub(crate) fn end(&mut self) -> anyhow::Result<()> {
        queue!(
            self.out,
            BeginSynchronizedUpdate,
            ResetColor,
            Clear(ClearType::All),
            cursor::Show,
            EnableLineWrap,
            EndSynchronizedUpdate,
            LeaveAlternateScreen
        )?;
        self.out.flush()?;
        terminal::disable_raw_mode()?;
        Ok(())
    }

    pub(crate) fn resize_if_needed(&mut self) -> anyhow::Result<bool> {
        let (c, r) = terminal::size()?;
        if c == self.cols && r == self.rows {
            return Ok(false);
        }
        self.cols = c;
        self.rows = r;
        self.prev = CellBuffer::new(c, r);
        self.cur = CellBuffer::new(c, r);
        self.canvas = PixelCanvas::new(c as u32 * 2, r as u32 * 4);
        Ok(true)
    }

    pub(crate) fn present(&mut self, diff_only: bool) -> anyhow::Result<()> {
        queue!(self.out, BeginSynchronizedUpdate)?;

        let mut last_fg = None;
        let mut last_bg = None;

        for y in 0..self.rows {
            for x in 0..self.cols {
                let i = self.cur.idx(x, y);
                let c = self.cur.cells[i];
                if diff_only && c == self.prev.cells[i] {
                    continue;
                }

                queue!(self.out, cursor::MoveTo(x, y))?;

                if last_fg != Some(c.fg) {
                    queue!(self.out, SetForegroundColor(c.fg))?;
                    last_fg = Some(c.fg);
                }
                if last_bg != Some(c.bg) {
                    queue!(self.out, SetBackgroundColor(c.bg))?;
                    last_bg = Some(c.bg);
                }

                queue!(self.out, Print(c.ch))?;
            }
        }

        queue!(self.out, ResetColor, EndSynchronizedUpdate)?;
        self.out.flush()?;
        self.prev.cells.copy_from_slice(&self.cur.cells);
        Ok(())
    }
}

/* -----------------------------
   Braille encoding: 2×4 pixels -> U+2800..U+28FF
------------------------------ */

fn braille_bit(dx: u32, dy: u32) -> u8 {
    // Dot mapping:
    // (0,0)=1 (0,1)=2 (0,2)=4 (0,3)=64
    // (1,0)=8 (1,1)=16 (1,2)=32 (1,3)=128
    match (dx, dy) {
        (0, 0) => 0x01,
        (0, 1) => 0x02,
        (0, 2) => 0x04,
        (0, 3) => 0x40,
        (1, 0) => 0x08,
        (1, 1) => 0x10,
        (1, 2) => 0x20,
        (1, 3) => 0x80,
        _ => 0x00,
    }
}

pub(crate) fn canvas_to_cells(
    canvas: &PixelCanvas,
    out: &mut CellBuffer,
    enable_color: bool,
    bg: Color,
) {
    let cols = out.w as u32;
    let rows = out.h as u32;

    for cy in 0..rows {
        for cx in 0..cols {
            let px0 = cx * 2;
            let py0 = cy * 4;

            let mut mask: u8 = 0;
            let mut sum_r: u32 = 0;
            let mut sum_g: u32 = 0;
            let mut sum_b: u32 = 0;
            let mut ink_count: u32 = 0;

            for dy in 0..4 {
                for dx in 0..2 {
                    let x = px0 + dx;
                    let y = py0 + dy;
                    if x >= canvas.w || y >= canvas.h {
                        continue;
                    }
                    let p = canvas.px[canvas.idx(x, y)];
                    let a = p.a as u32;

                    // threshold: treat alpha as ink
                    if a >= 32 {
                        mask |= braille_bit(dx, dy);
                        sum_r += p.r as u32;
                        sum_g += p.g as u32;
                        sum_b += p.b as u32;
                        ink_count += 1;
                    }
                }
            }

            let ch = char::from_u32(0x2800 + (mask as u32)).unwrap_or(' ');

            let fg = if enable_color && ink_count > 0 {
                let r = (sum_r / ink_count) as u8;
                let g = (sum_g / ink_count) as u8;
                let b = (sum_b / ink_count) as u8;
                Color::Rgb { r, g, b }
            } else {
                Color::White
            };

            out.set(
                cx as u16,
                cy as u16,
                Cell {
                    ch,
                    fg,
                    bg,
                    bold: false,
                },
            );
        }
    }
}

/* -----------------------------
   Layered skin renderer (anchors + z-order)
------------------------------ */

pub(crate) struct Renderer;

impl Renderer {
    pub(crate) fn draw_pet(
        canvas: &mut PixelCanvas,
        st: &GameState,
        viewport: Viewport,
        offset: (i32, i32),
    ) {
        let center_x = viewport.x + viewport.w / 2 + offset.0;
        let center_y = viewport.y + viewport.h / 2 + offset.1;

        let mood_col = match st.pet.mood {
            Mood::Happy => Pixel {
                r: 140,
                g: 240,
                b: 200,
                a: 220,
            },
            Mood::Okay => Pixel {
                r: 150,
                g: 170,
                b: 240,
                a: 210,
            },
            Mood::Sad => Pixel {
                r: 130,
                g: 150,
                b: 220,
                a: 200,
            },
            Mood::Angry => Pixel {
                r: 255,
                g: 90,
                b: 90,
                a: 225,
            },
            Mood::Sick => Pixel {
                r: 140,
                g: 240,
                b: 130,
                a: 220,
            },
            Mood::Sleepy => Pixel {
                r: 160,
                g: 140,
                b: 255,
                a: 210,
            },
            Mood::Bored => Pixel {
                r: 200,
                g: 200,
                b: 200,
                a: 205,
            },
        };

        let radius = match st.pet.stage {
            LifeStage::Egg => 10,
            LifeStage::Baby => 14,
            LifeStage::Child => 18,
            LifeStage::Teen => 20,
            LifeStage::Adult => 22,
            LifeStage::Elder => 24,
        };

        for y in -radius..=radius {
            for x in -radius..=radius {
                let d2 = (x * x + y * y) as f32;
                let r2 = (radius * radius) as f32;
                if d2 > r2 {
                    continue;
                }
                let dist = (d2 / r2).sqrt();
                let t = 1.0 - dist;
                let a = (mood_col.a as f32 * (0.3 + 0.7 * t)).clamp(0.0, 255.0) as u8;
                canvas.blend_over(center_x + x, center_y + y, Pixel { a, ..mood_col });
            }
        }

        let eye = Pixel {
            r: 5,
            g: 5,
            b: 8,
            a: 245,
        };
        if st.pet.flags.sleeping {
            for x in -5..=5 {
                canvas.blend_over(center_x - 6 + x, center_y - radius / 5, eye);
                canvas.blend_over(center_x + 6 + x, center_y - radius / 5, eye);
            }
            for x in -4..=4 {
                canvas.blend_over(center_x - 6 + x, center_y - radius / 5 + 1, eye);
                canvas.blend_over(center_x + 6 + x, center_y - radius / 5 + 1, eye);
            }
        } else {
            let left_x = center_x - radius / 3;
            let right_x = center_x + radius / 3;
            let eye_y = center_y - radius / 5;
            for dy in -1..=1 {
                for dx in -1..=1 {
                    canvas.blend_over(left_x + dx, eye_y + dy, eye);
                    canvas.blend_over(right_x + dx, eye_y + dy, eye);
                }
            }
        }

        if st.pet.flags.sick {
            let p = Pixel {
                r: 140,
                g: 255,
                b: 160,
                a: 200,
            };
            for i in 0..8 {
                canvas.blend_over(center_x + radius - i, center_y - radius + i, p);
            }
        }

        if st.pet.flags.attention_call {
            let alert = Pixel {
                r: 255,
                g: 80,
                b: 80,
                a: 230,
            };
            let y = center_y - radius - 6;
            for dy in 0..6 {
                canvas.blend_over(center_x, y + dy, alert);
            }
            canvas.blend_over(center_x, y + 7, alert);
            canvas.blend_over(center_x - 1, y + 7, alert);
            canvas.blend_over(center_x + 1, y + 7, alert);
        }
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Viewport {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: i32,
    pub(crate) h: i32,
}

pub(crate) fn pet_bounce_offset_subpx(st: &GameState) -> (i32, i32) {
    let t = st.sim_ticks as f32 * 0.1;
    let x = (t.cos() * 3.0) as i32;
    let y = (t.sin() * 2.0) as i32;
    (x, y)
}

pub(crate) fn pet_bounce_offset_cells(st: &GameState) -> (i32, i32) {
    let (sx, sy) = pet_bounce_offset_subpx(st);
    let x = (sx as f32 / 2.0).round() as i32;
    let y = (sy as f32 / 4.0).round() as i32;
    (x, y)
}

/* -----------------------------
   UI overlay (text + meters)
------------------------------ */

pub(crate) fn draw_text(buf: &mut CellBuffer, x: u16, y: u16, s: &str, fg: Color, bg: Color) {
    for (i, ch) in s.chars().enumerate() {
        let xx = x.saturating_add(i as u16);
        if xx >= buf.w || y >= buf.h {
            break;
        }
        buf.set(
            xx,
            y,
            Cell {
                ch,
                fg,
                bg,
                bold: false,
            },
        );
    }
}

fn bar(value01: f32, width: usize) -> String {
    let v = value01.clamp(0.0, 1.0);
    let fill = (v * width as f32 + 0.5) as usize;
    let mut s = String::new();
    s.push('[');
    for i in 0..width {
        s.push(if i < fill { '█' } else { ' ' });
    }
    s.push(']');
    s
}

pub(crate) fn ui_overlay(buf: &mut CellBuffer, st: &GameState, settings: &Settings) {
    let bg = Color::Black;
    let fg = Color::White;

    let title = format!(
        "Termigotchi  |  {} ({:?})  |  Mood: {:?}",
        st.pet.name, st.pet.stage, st.pet.mood
    );
    draw_text(buf, 1, 0, &title, fg, bg);

    let m = st.pet.meters;

    let lines = [
        ("Hunger", m.hunger),
        ("Happy ", m.happiness),
        ("Health", m.health),
        ("Energy", m.energy),
        ("Hyg   ", m.hygiene),
    ];

    for (i, (name, val)) in lines.iter().enumerate() {
        let b = bar(*val / 100.0, 14);
        let s = format!("{name}: {b} {:>5.1}", val);
        draw_text(buf, 1, 2 + i as u16, &s, fg, bg);
    }

    let flags = format!(
        "Flags: sleep={} sick={} dirty={} poop={} call={}",
        st.pet.flags.sleeping as u8,
        st.pet.flags.sick as u8,
        st.pet.flags.dirty as u8,
        st.pet.flags.has_poop as u8,
        st.pet.flags.attention_call as u8
    );
    draw_text(buf, 1, 8, &flags, fg, bg);

    let help = match st.scene {
        Scene::Main => {
            "Keys: q quit | f feed | p play | c clean | m medicine | s sleep | tab settings | h help"
        }
        Scene::Settings => "Settings: ↑↓ select | enter apply | esc back | tab back | h help",
        Scene::Help => "Help: esc back | h close | q quit",
        Scene::Rename => "Rename: type name | enter save | esc cancel",
        Scene::Recap(_) => "Recap: any key to continue",
        Scene::Dead => "Dead: n new game | q quit",
    };
    draw_text(buf, 1, buf.h.saturating_sub(1), help, fg, bg);

    if matches!(st.scene, Scene::Settings) {
        draw_settings(buf, st, settings);
    }
}

/* -----------------------------
   Settings UI
------------------------------ */

pub(crate) fn draw_settings(buf: &mut CellBuffer, st: &GameState, settings: &Settings) {
    let bg = Color::Black;
    let fg = Color::White;
    let hi = Color::Yellow;

    let start_x = 1;
    let start_y = 11;

    draw_text(buf, start_x, start_y, "Settings", fg, bg);

    let selected = st.settings_cursor;
    let render_mode = if settings.enable_braille {
        "Braille"
    } else {
        "ASCII"
    };
    let line = format!(
        "{} Render: {}",
        if selected == 0 { ">" } else { " " },
        render_mode
    );
    draw_text(
        buf,
        start_x,
        start_y + 2,
        &line,
        if selected == 0 { hi } else { fg },
        bg,
    );

    let mut name_display = st.pet.name.clone();
    if name_display.len() > 16 {
        name_display.truncate(15);
        name_display.push_str("...");
    }
    let name_line = format!(
        "{} Name: {}",
        if selected == 1 { ">" } else { " " },
        name_display
    );
    draw_text(
        buf,
        start_x,
        start_y + 3,
        &name_line,
        if selected == 1 { hi } else { fg },
        bg,
    );
}

pub(crate) fn draw_pet_ascii(buf: &mut CellBuffer, st: &GameState, cx: i32, cy: i32) {
    let bg = Color::Black;
    let fg = Color::White;

    let (w, h) = (15i32, 9i32);
    let x0 = cx - w / 2;
    let y0 = cy - h / 2;

    let mut grid = [
        "       _____     ",
        "     /       \\   ",
        "    /  o   o  \\  ",
        "   |     ^     | ",
        "   |   \\___/   | ",
        "    \\         /  ",
        "     \\_______/   ",
        "                 ",
        "                 ",
    ];

    // mood tweak: mouth shape
    if matches!(st.pet.mood, Mood::Sad | Mood::Sick | Mood::Angry) {
        grid[4] = "   |   /___\\   | ";
    } else if matches!(st.pet.mood, Mood::Happy) {
        grid[4] = "   |   \\___/   | ";
    }

    for (yy, line) in grid.iter().enumerate() {
        let y = y0 + yy as i32;
        if y < 0 || y >= buf.h as i32 {
            continue;
        }
        let mut x = x0;
        for ch in line.chars() {
            if x >= 0 && x < buf.w as i32 {
                buf.set(
                    x as u16,
                    y as u16,
                    Cell {
                        ch,
                        fg,
                        bg,
                        bold: false,
                    },
                );
            }
            x += 1;
        }
    }

    if st.pet.flags.attention_call {
        let ax = cx;
        let ay = y0 - 2;
        if ay >= 0 && ay < buf.h as i32 && ax >= 0 && ax < buf.w as i32 {
            buf.set(
                ax as u16,
                ay as u16,
                Cell {
                    ch: '!',
                    fg: Color::Red,
                    bg,
                    bold: true,
                },
            );
        }
    }
}
