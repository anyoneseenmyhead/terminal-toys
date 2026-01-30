use crate::config::{load_settings, project_paths, save_settings_atomic, Settings};
use crate::input::{collect_input_nonblocking, map_event_to_action};
use crate::model::{GameState, Rules, SaveFile, Scene, SAVE_VERSION};
use crate::render::{
    canvas_to_cells, draw_pet_ascii, draw_text, pet_bounce_offset_cells,
    pet_bounce_offset_subpx, ui_overlay, Cell, Renderer, Terminal, Viewport,
};
use crate::sim::{catch_up, PlayerAction};
use crate::storage::{load_or_init_save, save_atomic};
use std::cmp::{max, min};
use std::time::{Duration, Instant};

pub(crate) struct App {
    settings: Settings,
    rules: Rules,
    state: GameState,
    paths: crate::config::Paths,
    term: Terminal,
    should_quit: bool,
    autosave_at: Instant,
}

impl App {
    fn init() -> anyhow::Result<Self> {
        let paths = project_paths()?;
        let mut settings = load_settings(&paths.settings_path);
        let rules = Rules::default();

        let (mut state, loaded_last_seen) = load_or_init_save(&paths.save_path, &settings)?;

        // offline catch-up
        if let Some(last_seen) = loaded_last_seen {
            let now = chrono::Utc::now();
            let summary = catch_up(&mut state, last_seen, now, &rules);
            if summary.has_anything() {
                state.scene = Scene::Recap(summary);
            }
        }

        // ensure deterministic seed exists
        if settings.seed == 0 {
            settings.seed = 0xC0FFEE_u64;
        }

        let term = Terminal::begin()?;

        Ok(Self {
            settings,
            rules,
            state,
            paths,
            term,
            should_quit: false,
            autosave_at: Instant::now() + Duration::from_secs(10),
        })
    }

    fn run(&mut self) -> anyhow::Result<()> {
        let fps = self.settings.fps_cap.max(10).min(240);
        let frame_dt = Duration::from_secs_f32(1.0 / fps as f32);
        let sim_step = Duration::from_millis(self.rules.tick_step_ms);

        let mut last_frame = Instant::now();
        let mut sim_accum = Duration::ZERO;

        while !self.should_quit {
            let _resized = self.term.resize_if_needed()?;

            // input
            let events = collect_input_nonblocking(frame_dt)?;
            for ev in events {
                if let Some(action) = map_event_to_action(&self.state.scene, ev) {
                    match action {
                        PlayerAction::Quit => {
                            self.should_quit = true;
                            break;
                        }
                        PlayerAction::SettingsToggle => {
                            if self.state.settings_cursor == 0 {
                                self.settings.enable_braille = !self.settings.enable_braille;
                            } else if self.state.settings_cursor == 1 {
                                self.state.apply(PlayerAction::RenameOpen);
                            }
                        }
                        _ => self.state.apply(action),
                    }
                } else {
                    // For recap: any key continues
                    if matches!(self.state.scene, Scene::Recap(_)) {
                        self.state.scene = Scene::Main;
                    }
                }
            }

            // sim fixed-step
            let now = Instant::now();
            let real_dt = now.saturating_duration_since(last_frame);
            last_frame = now;
            sim_accum = sim_accum.saturating_add(real_dt);

            while sim_accum >= sim_step {
                self.state.tick_fixed_step(&self.rules);
                sim_accum = sim_accum.saturating_sub(sim_step);
                if self.state.pet.flags.dead && !matches!(self.state.scene, Scene::Dead) {
                    self.state.scene = Scene::Dead;
                }
            }

            // render
            self.render_frame()?;

            // autosave
            if Instant::now() >= self.autosave_at {
                self.save_now()?;
                self.autosave_at = Instant::now() + Duration::from_secs(10);
            }

            // frame cap
            spin_sleep(frame_dt, Instant::now());
        }

        self.save_now()?;
        self.term.end()?;
        save_settings_atomic(&self.paths.settings_path, &self.settings)?;
        Ok(())
    }

    fn render_frame(&mut self) -> anyhow::Result<()> {
        let bg = crossterm::style::Color::Black;
        self.term.cur.clear(bg);

        if self.settings.enable_braille {
            self.term.canvas.clear(crate::render::Pixel {
                r: 0,
                g: 0,
                b: 0,
                a: 0,
            });

            // Reserve left panel for text; pet viewport on right.
            let cols = self.term.cols as i32;
            let rows = self.term.rows as i32;

            let panel_w_cells = min(max(26, cols / 3), cols - 10);
            let pet_x_cells = panel_w_cells;
            let pet_w_cells = cols - pet_x_cells;
            let pet_h_cells = rows;

            // Pixel viewport in braille subpixels
            let vp = Viewport {
                x: pet_x_cells * 2,
                y: 0,
                w: pet_w_cells * 2,
                h: pet_h_cells * 4,
            };
            let bounce = pet_bounce_offset_subpx(&self.state);
            Renderer::draw_pet(&mut self.term.canvas, &self.state, vp, bounce);

            canvas_to_cells(
                &self.term.canvas,
                &mut self.term.cur,
                self.settings.enable_color,
                bg,
            );
        } else {
            // ASCII fallback: layered sprite approximation
            let cols = self.term.cols as i32;
            let rows = self.term.rows as i32;
            let panel_w_cells = min(max(26, cols / 3), cols - 10);
            let pet_x_cells = panel_w_cells;
            let pet_w_cells = cols - pet_x_cells;
            let pet_h_cells = rows;
            let vx = pet_x_cells + pet_w_cells / 2;
            let vy = pet_h_cells / 2;
            let bounce = pet_bounce_offset_cells(&self.state);
            draw_pet_ascii(&mut self.term.cur, &self.state, vx + bounce.0, vy + bounce.1);
        }

        // UI overlay on top
        ui_overlay(&mut self.term.cur, &self.state, &self.settings);

        // Recap overlay
        if let Scene::Recap(ref s) = self.state.scene {
            self.draw_center_box(
                "While you were away…",
                &format!(
                    "Simulated {} ticks\nMin hunger: {:.1}\nMin happiness: {:.1}\nMin health: {:.1}\nAttention calls: {}\nSick: {}\nDied: {}\n\nPress any key",
                    s.ticks_simulated,
                    s.hunger_min,
                    s.happiness_min,
                    s.health_min,
                    s.attention_calls,
                    s.became_sick,
                    s.died
                ),
            )?;
        }

        // Help overlay
        if let Scene::Help = self.state.scene {
            self.draw_center_box(
                "How to play",
                "Goal: keep your pet healthy and happy as it ages.\n\
    Watch the meters on the left; low stats hurt mood.\n\n\
    F Feed: +hunger/+happiness, may create poop.\n\
    P Play: +happiness, -energy, increases hunger.\n\
    C Clean: removes dirt/poop.\n\
    M Medicine: cures sickness.\n\
    S Sleep: toggle rest to regain energy.\n\
    D Discipline: clears attention calls, boosts discipline.\n\n\
    Neglect (dirty/sick/low stats) drains health over time.\n\
    Tab opens Settings (toggle render, rename pet).\n\n\
    Esc or H to close help.",
            )?;
        }

        // Rename overlay
        if let Scene::Rename = self.state.scene {
            let mut preview = self.state.name_edit.clone();
            if preview.len() < 18 {
                preview.push('_');
            }
            self.draw_center_box(
                "Rename pet",
                &format!(
                    "Type a name (max 18 chars).\n\nName: {}\n\nEnter save | Esc cancel | Backspace delete",
                    preview
                ),
            )?;
        }

        // Dead overlay
        if let Scene::Dead = self.state.scene {
            self.draw_center_box(
                "Your Termigotchi has passed on.",
                "Press N for new game, or Q to quit.",
            )?;
        }

        self.term.present(true)?;
        Ok(())
    }

    fn draw_center_box(&mut self, title: &str, body: &str) -> anyhow::Result<()> {
        let w = self.term.cols;
        let h = self.term.rows;

        let bw = min(60, w.saturating_sub(4));
        let bh = min(18, h.saturating_sub(4));

        let x0 = (w - bw) / 2;
        let y0 = (h - bh) / 2;

        // border
        for x in x0..x0 + bw {
            self.term.cur.set(
                x,
                y0,
                Cell {
                    ch: '─',
                    fg: crossterm::style::Color::White,
                    bg: crossterm::style::Color::Black,
                    bold: false,
                },
            );
            self.term.cur.set(
                x,
                y0 + bh - 1,
                Cell {
                    ch: '─',
                    fg: crossterm::style::Color::White,
                    bg: crossterm::style::Color::Black,
                    bold: false,
                },
            );
        }
        for y in y0..y0 + bh {
            self.term.cur.set(
                x0,
                y,
                Cell {
                    ch: '│',
                    fg: crossterm::style::Color::White,
                    bg: crossterm::style::Color::Black,
                    bold: false,
                },
            );
            self.term.cur.set(
                x0 + bw - 1,
                y,
                Cell {
                    ch: '│',
                    fg: crossterm::style::Color::White,
                    bg: crossterm::style::Color::Black,
                    bold: false,
                },
            );
        }
        self.term.cur.set(
            x0,
            y0,
            Cell {
                ch: '┌',
                fg: crossterm::style::Color::White,
                bg: crossterm::style::Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0 + bw - 1,
            y0,
            Cell {
                ch: '┐',
                fg: crossterm::style::Color::White,
                bg: crossterm::style::Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0,
            y0 + bh - 1,
            Cell {
                ch: '└',
                fg: crossterm::style::Color::White,
                bg: crossterm::style::Color::Black,
                bold: false,
            },
        );
        self.term.cur.set(
            x0 + bw - 1,
            y0 + bh - 1,
            Cell {
                ch: '┘',
                fg: crossterm::style::Color::White,
                bg: crossterm::style::Color::Black,
                bold: false,
            },
        );

        // title
        draw_text(
            &mut self.term.cur,
            x0 + 2,
            y0 + 1,
            title,
            crossterm::style::Color::White,
            crossterm::style::Color::Black,
        );

        // body
        let mut yy = y0 + 3;
        for line in body.lines() {
            if yy >= y0 + bh - 1 {
                break;
            }
            draw_text(
                &mut self.term.cur,
                x0 + 2,
                yy,
                line,
                crossterm::style::Color::White,
                crossterm::style::Color::Black,
            );
            yy += 1;
        }

        Ok(())
    }

    fn save_now(&self) -> anyhow::Result<()> {
        let now = chrono::Utc::now();
        let save = SaveFile {
            version: SAVE_VERSION,
            last_seen_utc: now,
            state: self.state.clone(),
        };
        save_atomic(&self.paths.save_path, &save)?;
        Ok(())
    }
}

pub(crate) fn run() -> anyhow::Result<()> {
    let mut app = App::init()?;
    app.run()?;
    Ok(())
}

/* -----------------------------
   Frame pacing helper
------------------------------ */

fn spin_sleep(target: Duration, now: Instant) {
    let end = now + target;
    loop {
        let t = Instant::now();
        if t >= end {
            break;
        }
        let left = end - t;
        if left > Duration::from_millis(2) {
            std::thread::sleep(Duration::from_millis(1));
        } else {
            std::hint::spin_loop();
        }
    }
}
