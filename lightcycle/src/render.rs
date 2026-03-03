use std::env;
use std::io::{Stdout, Write};

use crossterm::cursor::MoveTo;
use crossterm::queue;
use crossterm::style::{Color, PrintStyledContent, Stylize, style};
use crossterm::terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate};

use crate::sim::{Coord, Round, RoundOutcome, WALL_CELL};

#[derive(Clone, Copy, Debug)]
pub struct RenderConfig {
    pub no_color: bool,
    pub glow: bool,
}

#[derive(Clone, Debug)]
pub struct HudData {
    pub fps: f32,
    pub speed: u32,
    pub alive: usize,
    pub wins: u64,
    pub losses: u64,
    pub seed: u64,
    pub controlled_cycle: Option<u16>,
    pub paused: bool,
    pub waiting_for_start: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct CellVisual {
    ch: char,
    fg: Option<Color>,
    bg: Option<Color>,
}

impl Default for CellVisual {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: None,
            bg: None,
        }
    }
}

pub struct Renderer {
    width: u16,
    height: u16,
    prev: Vec<CellVisual>,
    truecolor: bool,
}

impl Renderer {
    pub fn new(width: u16, height: u16) -> Self {
        let truecolor = env::var("COLORTERM")
            .map(|v| {
                let lower = v.to_ascii_lowercase();
                lower.contains("truecolor") || lower.contains("24bit")
            })
            .unwrap_or(false);
        Self {
            width,
            height,
            prev: vec![CellVisual::default(); (width as usize) * (height as usize)],
            truecolor,
        }
    }

    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        self.prev = vec![CellVisual::default(); (width as usize) * (height as usize)];
    }

    pub fn draw(
        &mut self,
        out: &mut Stdout,
        round: Option<&Round>,
        hud: &HudData,
        show_walls: bool,
        cfg: RenderConfig,
        banner: Option<RoundOutcome>,
    ) -> std::io::Result<()> {
        let mut current =
            vec![CellVisual::default(); (self.width as usize) * (self.height as usize)];

        if let Some(r) = round {
            self.draw_playfield(&mut current, r, show_walls, cfg);
            self.draw_border(&mut current, r, cfg);
            if let Some(outcome) = banner {
                self.draw_banner(&mut current, r, outcome, cfg);
            }
            if hud.waiting_for_start {
                self.draw_start_prompt(&mut current, r, cfg);
            }
        } else {
            self.draw_centered_text(
                &mut current,
                self.height / 2,
                "Terminal too small for playfield",
                cfg,
            );
        }

        self.draw_hud(&mut current, hud, cfg);

        queue!(out, BeginSynchronizedUpdate)?;
        for y in 0..self.height {
            for x in 0..self.width {
                let idx = y as usize * self.width as usize + x as usize;
                if current[idx] == self.prev[idx] {
                    continue;
                }
                let cv = current[idx];
                let mut styled = style(cv.ch);
                if let Some(fg) = cv.fg {
                    styled = styled.with(fg);
                }
                if let Some(bg) = cv.bg {
                    styled = styled.on(bg);
                }
                queue!(out, MoveTo(x, y), PrintStyledContent(styled))?;
            }
        }
        queue!(out, EndSynchronizedUpdate)?;
        out.flush()?;
        self.prev = current;
        Ok(())
    }

    fn draw_playfield(
        &self,
        frame: &mut [CellVisual],
        round: &Round,
        show_walls: bool,
        cfg: RenderConfig,
    ) {
        for y in 0..round.grid.height {
            for x in 0..round.grid.width {
                let term = round.playfield.to_terminal(Coord { x, y });
                if !self.in_term(term) {
                    continue;
                }
                let idx = self.term_idx(term);
                let cell = round.grid.get(Coord { x, y });
                frame[idx] = if cell == WALL_CELL {
                    if show_walls {
                        CellVisual {
                            ch: '▓',
                            fg: self.wall_color(cfg),
                            bg: None,
                        }
                    } else {
                        CellVisual::default()
                    }
                } else if cell != 0 {
                    let color = self.cycle_color((cell - 1) as usize, cfg);
                    CellVisual {
                        ch: '█',
                        fg: color,
                        bg: None,
                    }
                } else {
                    CellVisual::default()
                };
            }
        }

        if cfg.glow {
            for c in round.cycles.iter().filter(|c| c.alive) {
                for d in [
                    Coord { x: -1, y: 0 },
                    Coord { x: 1, y: 0 },
                    Coord { x: 0, y: -1 },
                    Coord { x: 0, y: 1 },
                    Coord { x: -1, y: -1 },
                    Coord { x: -1, y: 1 },
                    Coord { x: 1, y: -1 },
                    Coord { x: 1, y: 1 },
                ] {
                    let p = Coord {
                        x: c.pos.x + d.x,
                        y: c.pos.y + d.y,
                    };
                    if round.grid.get(p) != 0 {
                        continue;
                    }
                    let t = round.playfield.to_terminal(p);
                    if !self.in_term(t) {
                        continue;
                    }
                    let idx = self.term_idx(t);
                    if frame[idx].ch == ' ' {
                        frame[idx] = CellVisual {
                            ch: '·',
                            fg: self.glow_color(c.color_idx, c.is_player, cfg),
                            bg: None,
                        };
                    }
                }
            }
        }

        for c in round.cycles.iter().filter(|c| c.alive) {
            let term = round.playfield.to_terminal(c.pos);
            if !self.in_term(term) {
                continue;
            }
            let idx = self.term_idx(term);
            frame[idx] = CellVisual {
                ch: '●',
                fg: self.head_color(c.color_idx, c.is_player, cfg),
                bg: None,
            };
        }
    }

    fn draw_border(&self, frame: &mut [CellVisual], round: &Round, cfg: RenderConfig) {
        let left = round.playfield.x - 1;
        let right = round.playfield.x + round.playfield.width;
        let top = round.playfield.y - 1;
        let bottom = round.playfield.y + round.playfield.height;
        let border = self.border_color(cfg);

        for x in left..=right {
            self.put_if_in_term(frame, Coord { x, y: top }, '─', border);
            self.put_if_in_term(frame, Coord { x, y: bottom }, '─', border);
        }
        for y in top..=bottom {
            self.put_if_in_term(frame, Coord { x: left, y }, '│', border);
            self.put_if_in_term(frame, Coord { x: right, y }, '│', border);
        }

        self.put_if_in_term(frame, Coord { x: left, y: top }, '┌', border);
        self.put_if_in_term(frame, Coord { x: right, y: top }, '┐', border);
        self.put_if_in_term(frame, Coord { x: left, y: bottom }, '└', border);
        self.put_if_in_term(
            frame,
            Coord {
                x: right,
                y: bottom,
            },
            '┘',
            border,
        );
    }

    fn draw_hud(&self, frame: &mut [CellVisual], hud: &HudData, cfg: RenderConfig) {
        let controlled = match hud.controlled_cycle {
            Some(id) => format!("C{id}"),
            None => "AI-only".to_string(),
        };
        let pause = if hud.paused { " PAUSED" } else { "" };
        let start = if hud.waiting_for_start {
            " START:WASD/Arrows"
        } else {
            ""
        };
        let text = format!(
            "FPS:{:>4.1} Speed:{} Alive:{} W:{} L:{} Seed:{} Ctrl:{}{}{}",
            hud.fps, hud.speed, hud.alive, hud.wins, hud.losses, hud.seed, controlled, pause, start
        );

        for (x, ch) in text.chars().take(self.width as usize).enumerate() {
            frame[x] = CellVisual {
                ch,
                fg: self.hud_color(cfg),
                bg: None,
            };
        }
    }

    fn draw_banner(
        &self,
        frame: &mut [CellVisual],
        round: &Round,
        outcome: RoundOutcome,
        cfg: RenderConfig,
    ) {
        let text = match outcome {
            RoundOutcome::Victory => "VICTORY",
            RoundOutcome::Defeat => "DEFEAT",
        };
        let y = (round.playfield.y + round.playfield.height / 2).max(1) as u16;
        self.draw_centered_text(frame, y, text, cfg);
    }

    fn draw_start_prompt(&self, frame: &mut [CellVisual], round: &Round, cfg: RenderConfig) {
        let text = "Press WASD or Arrow Key to start";
        let y = (round.playfield.y + 1).max(1) as u16;
        self.draw_centered_text(frame, y, text, cfg);
    }

    fn draw_centered_text(&self, frame: &mut [CellVisual], y: u16, text: &str, cfg: RenderConfig) {
        if y >= self.height {
            return;
        }
        let len = text.chars().count() as i32;
        let x0 = ((self.width as i32 - len) / 2).max(0) as u16;
        for (i, ch) in text.chars().enumerate() {
            let x = x0.saturating_add(i as u16);
            if x >= self.width {
                break;
            }
            let idx = self.term_idx(Coord {
                x: i32::from(x),
                y: i32::from(y),
            });
            frame[idx] = CellVisual {
                ch,
                fg: self.banner_color(cfg),
                bg: None,
            };
        }
    }

    fn in_term(&self, c: Coord) -> bool {
        c.x >= 0 && c.y >= 0 && c.x < self.width as i32 && c.y < self.height as i32
    }

    fn put_if_in_term(&self, frame: &mut [CellVisual], c: Coord, ch: char, fg: Option<Color>) {
        if !self.in_term(c) {
            return;
        }
        let idx = self.term_idx(c);
        frame[idx] = CellVisual { ch, fg, bg: None };
    }

    fn term_idx(&self, c: Coord) -> usize {
        c.y as usize * self.width as usize + c.x as usize
    }

    fn wall_color(&self, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            None
        } else if self.truecolor {
            Some(Color::Rgb {
                r: 70,
                g: 80,
                b: 100,
            })
        } else {
            Some(Color::AnsiValue(240))
        }
    }

    fn hud_color(&self, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            None
        } else if self.truecolor {
            Some(Color::Rgb {
                r: 170,
                g: 190,
                b: 220,
            })
        } else {
            Some(Color::AnsiValue(153))
        }
    }

    fn banner_color(&self, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            None
        } else if self.truecolor {
            Some(Color::Rgb {
                r: 255,
                g: 230,
                b: 120,
            })
        } else {
            Some(Color::AnsiValue(220))
        }
    }

    fn border_color(&self, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            None
        } else if self.truecolor {
            Some(Color::Rgb {
                r: 120,
                g: 140,
                b: 175,
            })
        } else {
            Some(Color::AnsiValue(110))
        }
    }

    fn head_color(&self, idx: usize, is_player: bool, cfg: RenderConfig) -> Option<Color> {
        let base = self.cycle_color(idx, cfg);
        if cfg.no_color {
            return base;
        }
        if is_player {
            if self.truecolor {
                return Some(Color::Rgb {
                    r: 100,
                    g: 255,
                    b: 255,
                });
            }
            return Some(Color::AnsiValue(51));
        }
        base
    }

    fn glow_color(&self, idx: usize, is_player: bool, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            return None;
        }
        if is_player {
            if self.truecolor {
                return Some(Color::Rgb {
                    r: 70,
                    g: 170,
                    b: 170,
                });
            }
            return Some(Color::AnsiValue(44));
        }

        let table_rgb = [
            (150, 100, 40),
            (140, 70, 140),
            (70, 150, 70),
            (95, 80, 150),
            (70, 110, 160),
        ];
        let table_ansi = [130, 90, 71, 98, 67];
        let i = idx % table_rgb.len();
        if self.truecolor {
            let (r, g, b) = table_rgb[i];
            Some(Color::Rgb { r, g, b })
        } else {
            Some(Color::AnsiValue(table_ansi[i]))
        }
    }

    fn cycle_color(&self, idx: usize, cfg: RenderConfig) -> Option<Color> {
        if cfg.no_color {
            return None;
        }

        if idx == 0 {
            if self.truecolor {
                return Some(Color::Rgb {
                    r: 0,
                    g: 240,
                    b: 255,
                });
            }
            return Some(Color::AnsiValue(51));
        }

        let table_rgb = [
            (255, 140, 40),
            (255, 60, 190),
            (80, 230, 120),
            (165, 110, 255),
            (80, 170, 255),
        ];
        let table_ansi = [208, 201, 83, 141, 75];
        let i = (idx - 1) % table_rgb.len();
        if self.truecolor {
            let (r, g, b) = table_rgb[i];
            Some(Color::Rgb { r, g, b })
        } else {
            Some(Color::AnsiValue(table_ansi[i]))
        }
    }
}
