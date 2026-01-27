use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen},
};
use rand::{rngs::StdRng, Rng, SeedableRng};
use std::cmp::{max, min};
use std::io::{self, Stdout, Write};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug)]
struct Vec2 {
    x: f64,
    y: f64,
}
impl Vec2 {
    fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
}

#[derive(Clone, Debug)]
struct Terrain {
    /// height in "world units" from bottom (0) upward, sampled per column
    h: Vec<i32>,
    pad_x0: usize,
    pad_x1: usize,
    pad_y: i32,
}

#[derive(Clone, Debug)]
struct Lander {
    pos: Vec2, // world coords: x in [0..w), y in [0..h) where y=0 is ground plane (bottom)
    vel: Vec2,
    angle: f64, // radians; 0 is "up"
    fuel: f64,
    alive: bool,
    landed: bool,
    message: String,
}

#[derive(Clone, Debug)]
struct Game {
    rng: StdRng,
    width: u16,
    height: u16,

    terrain: Terrain,
    lander: Lander,

    // Controls: thrust is a toggle (reliable across terminals), rotation is tap-pulse
    thrust_toggle: bool,
    left_until: Instant,
    right_until: Instant,

    // derived each frame
    rot_left: bool,
    rot_right: bool,

    // timing
    last_frame: Instant,
    accumulator: f64,

    // rendering
    color_index: usize,
}

fn clamp_i32(v: i32, lo: i32, hi: i32) -> i32 {
    max(lo, min(hi, v))
}

fn wrap_x(x: f64, w: f64) -> f64 {
    let mut xx = x;
    while xx < 0.0 {
        xx += w;
    }
    while xx >= w {
        xx -= w;
    }
    xx
}

fn make_terrain(rng: &mut StdRng, w: usize, h: usize) -> Terrain {
    let min_h = (h as f64 * 0.10) as i32;
    let max_h = (h as f64 * 0.45) as i32;

    let mut heights = vec![0i32; w];
    let mut cur = rng.gen_range(min_h..=max_h);
    let mut vel = 0i32;

    for x in 0..w {
        vel += rng.gen_range(-2..=2);
        vel = clamp_i32(vel, -4, 4);
        cur += vel;
        cur = clamp_i32(cur, min_h, max_h);
        heights[x] = cur;
    }

    for _ in 0..3 {
        let mut tmp = heights.clone();
        for x in 1..w - 1 {
            tmp[x] = (heights[x - 1] + heights[x] * 2 + heights[x + 1]) / 4;
        }
        heights = tmp;
    }

    let pad_w = max(8, w / 8);
    let pad_x0 = rng.gen_range(2..(w - pad_w - 2));
    let pad_x1 = pad_x0 + pad_w;

    let mut pad_y = 0i32;
    for x in pad_x0..pad_x1 {
        pad_y += heights[x];
    }
    pad_y /= (pad_x1 - pad_x0) as i32;

    for x in pad_x0..pad_x1 {
        heights[x] = pad_y;
    }

    // blend edges a little
    for i in 0..6 {
        let t = i as f64 / 6.0;
        let left = pad_x0.saturating_sub(1 + i);
        let right = min(w - 1, pad_x1 + i);
        if left > 0 {
            heights[left] =
                (heights[left] as f64 * (1.0 - t) + pad_y as f64 * t).round() as i32;
        }
        if right < w {
            heights[right] =
                (heights[right] as f64 * (1.0 - t) + pad_y as f64 * t).round() as i32;
        }
    }

    Terrain {
        h: heights,
        pad_x0,
        pad_x1,
        pad_y,
    }
}

fn reset_lander(w: u16, h: u16) -> Lander {
    Lander {
        pos: Vec2::new((w as f64) * 0.5, (h as f64) * 0.85),
        vel: Vec2::new(0.0, 0.0),
        angle: 0.0,
        fuel: 100.0,
        alive: true,
        landed: false,
        message: "Land softly on the '=' pad.".to_string(),
    }
}

fn init_game() -> io::Result<Game> {
    let (w, h) = terminal::size()?;
    let mut rng = StdRng::seed_from_u64(0xC0FFEE_u64 ^ (w as u64) << 16 ^ (h as u64));

    let w2 = w;
    let h2 = h;

    let terrain = make_terrain(&mut rng, w2 as usize, h2 as usize);
    let lander = reset_lander(w2, h2);

    let now = Instant::now();

    Ok(Game {
        rng,
        width: w2,
        height: h2,
        terrain,
        lander,
        thrust_toggle: false,
        left_until: now,
        right_until: now,
        rot_left: false,
        rot_right: false,
        last_frame: now,
        accumulator: 0.0,
        color_index: 0,
    })
}

fn world_to_screen_y(world_y: f64, term_h: u16) -> i32 {
    (term_h as i32 - 1) - world_y.round() as i32
}

fn terrain_height_at(terrain: &Terrain, x: i32) -> i32 {
    let w = terrain.h.len() as i32;
    let mut xx = x % w;
    if xx < 0 {
        xx += w;
    }
    terrain.h[xx as usize]
}

fn is_on_pad(terrain: &Terrain, x: i32) -> bool {
    let w = terrain.h.len() as i32;
    let mut xx = x % w;
    if xx < 0 {
        xx += w;
    }
    (xx as usize) >= terrain.pad_x0 && (xx as usize) < terrain.pad_x1
}

fn update_controls_from_time(g: &mut Game, now: Instant) {
    g.rot_left = now <= g.left_until;
    g.rot_right = now <= g.right_until;
}

fn update_physics(g: &mut Game, dt: f64) {
    if !g.lander.alive || g.lander.landed {
        return;
    }

    // tuned for terminal feel
    let gravity = 12.0;
    let rot_speed = 2.2;
    let thrust_acc = 26.0;
    let fuel_burn = 18.0;
    let drag = 0.08;

    if g.rot_left {
        g.lander.angle -= rot_speed * dt;
    }
    if g.rot_right {
        g.lander.angle += rot_speed * dt;
    }

    while g.lander.angle > std::f64::consts::PI {
        g.lander.angle -= 2.0 * std::f64::consts::PI;
    }
    while g.lander.angle < -std::f64::consts::PI {
        g.lander.angle += 2.0 * std::f64::consts::PI;
    }

    let mut ax = 0.0;
    let mut ay = -gravity;

    if g.thrust_toggle && g.lander.fuel > 0.0 {
        let burn = fuel_burn * dt;
        g.lander.fuel = (g.lander.fuel - burn).max(0.0);

        // angle=0 means thrust straight up (positive y)
        let dir = Vec2::new(g.lander.angle.sin(), g.lander.angle.cos());
        ax += dir.x * thrust_acc;
        ay += dir.y * thrust_acc;
    }

    g.lander.vel.x += ax * dt;
    g.lander.vel.y += ay * dt;

    g.lander.vel.x *= 1.0 - drag * dt;
    g.lander.vel.y *= 1.0 - drag * dt;

    g.lander.pos.x += g.lander.vel.x * dt;
    g.lander.pos.y += g.lander.vel.y * dt;

    g.lander.pos.x = wrap_x(g.lander.pos.x, g.width as f64);

    let lx = g.lander.pos.x.round() as i32;
    let ground = terrain_height_at(&g.terrain, lx) as f64;

    if g.lander.pos.y <= ground {
        let on_pad = is_on_pad(&g.terrain, lx);
        let v_speed = g.lander.vel.y.abs();
        let h_speed = g.lander.vel.x.abs();
        let ang = g.lander.angle.abs();

        let ok_v = v_speed <= 6.5;
        let ok_h = h_speed <= 4.5;
        let ok_a = ang <= 0.35; // ~20 degrees

        g.lander.pos.y = ground;

        if on_pad && ok_v && ok_h && ok_a {
            g.lander.landed = true;
            g.lander.message = format!(
                "Nice landing! (v={:.1}, h={:.1}, a={:.0}°)  Press R to fly again.",
                v_speed,
                h_speed,
                ang.to_degrees()
            );
        } else {
            g.lander.alive = false;
            g.lander.message = format!(
                "Crash! Need: on pad, low speeds, upright. (v={:.1}, h={:.1}, a={:.0}°)  Press R.",
                v_speed,
                h_speed,
                ang.to_degrees()
            );
        }
    }

    let top = (g.height as f64) - 2.0;
    if g.lander.pos.y > top {
        g.lander.pos.y = top;
        g.lander.vel.y = g.lander.vel.y.min(0.0);
    }
}

fn write_text_line(buf: &mut [u8], w: usize, y: usize, text: &str) {
    if y >= buf.len() / w {
        return;
    }
    let bytes = text.as_bytes();
    let start = y * w;
    let maxlen = w.saturating_sub(1);
    let len = min(bytes.len(), maxlen);
    buf[start..start + len].copy_from_slice(&bytes[..len]);
}

fn render(g: &Game, out: &mut Stdout) -> io::Result<()> {
    let w = g.width as usize;
    let h = g.height as usize;

    let mut buf = vec![b' '; w * h];

    // terrain
    for x in 0..w {
        let th = g.terrain.h[x] as f64;
        let sy = world_to_screen_y(th, g.height);
        for y in sy..(g.height as i32) {
            if y >= 0 && (y as usize) < h {
                buf[(y as usize) * w + x] = b'#';
            }
        }
    }

    // pad
    let pad_y_sy = world_to_screen_y(g.terrain.pad_y as f64, g.height);
    if pad_y_sy >= 0 && (pad_y_sy as usize) < h {
        for x in g.terrain.pad_x0..g.terrain.pad_x1 {
            if x < w {
                buf[(pad_y_sy as usize) * w + x] = b'=';
            }
        }
    }

    // lander
    let lx = g.lander.pos.x.round() as i32;
    let ly = g.lander.pos.y.round() as f64;
    let lsy = world_to_screen_y(ly, g.height);

    if lsy >= 0 && (lsy as usize) < h {
        let x = ((lx % (w as i32)) + (w as i32)) % (w as i32);
        let x = x as usize;

        let deg = g.lander.angle.to_degrees();
        let glyph = if deg.abs() < 15.0 {
            b'A'
        } else if deg > 0.0 {
            b'/'
        } else {
            b'\\'
        };

        buf[(lsy as usize) * w + x] = glyph;

        if g.thrust_toggle && g.lander.fuel > 0.0 && (lsy as usize + 1) < h {
            buf[(lsy as usize + 1) * w + x] = b'v';
        }
    }

    // HUD
    let alt = {
        let ground = terrain_height_at(&g.terrain, g.lander.pos.x.round() as i32) as f64;
        (g.lander.pos.y - ground).max(0.0)
    };

    let hud1 = format!(
        "Fuel {:>6.1} | Alt {:>6.1} | Vx {:>6.2} | Vy {:>6.2} | Angle {:>6.1}° | Thrust {}",
        g.lander.fuel,
        alt,
        g.lander.vel.x,
        g.lander.vel.y,
        g.lander.angle.to_degrees(),
        if g.thrust_toggle { "ON" } else { "OFF" }
    );
    let hud2 = "[←/→ or Z/X tap rotate] [Space/Up toggle thrust] [C color] [R restart] [Q quit]";

    write_text_line(&mut buf, w, 0, &hud1);
    write_text_line(&mut buf, w, 1, hud2);

    let msg_y = min(h.saturating_sub(2), 3);
    write_text_line(&mut buf, w, msg_y, &g.lander.message);

    let mut frame = String::with_capacity((w + 1) * h);
    for y in 0..h {
        let start = y * w;
        let end = start + w;
        frame.push_str(&String::from_utf8_lossy(&buf[start..end]));
        if y + 1 < h {
            frame.push('\r');
            frame.push('\n');
        }
    }

    let colors = [
        Color::Green,
        Color::Yellow,
        Color::Cyan,
        Color::Blue,
        Color::Red,
        Color::Magenta,
        Color::White,
        Color::Grey,
        Color::DarkGreen,
        Color::DarkYellow,
        Color::DarkCyan,
        Color::DarkBlue,
        Color::DarkRed,
        Color::DarkMagenta,
        Color::DarkGrey,
    ];
    let color = colors[g.color_index % colors.len()];

    execute!(
        out,
        cursor::MoveTo(0, 0),
        SetForegroundColor(color),
        Print(frame),
        ResetColor
    )?;
    out.flush()?;
    Ok(())
}

fn handle_input(g: &mut Game) -> io::Result<bool> {
    // Rotation is a short pulse per tap (doesn't rely on key repeat)
    const ROT_PULSE_MS: u64 = 90;

    while event::poll(Duration::from_millis(0))? {
        match event::read()? {
            Event::Key(k) => {
                let is_pressish = k.kind == KeyEventKind::Press || k.kind == KeyEventKind::Repeat;
                if !is_pressish {
                    continue;
                }

                let now = Instant::now();

                match k.code {
                    KeyCode::Char('q') | KeyCode::Char('Q') => return Ok(false),

                    KeyCode::Char('r') | KeyCode::Char('R') => {
                        g.terrain = make_terrain(&mut g.rng, g.width as usize, g.height as usize);
                        g.lander = reset_lander(g.width, g.height);
                        g.thrust_toggle = false;
                        g.left_until = now;
                        g.right_until = now;
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        g.color_index = g.color_index.wrapping_add(1);
                    }

                    // Toggle thrust (reliable across terminals)
                    KeyCode::Up | KeyCode::Char(' ') => {
                        g.thrust_toggle = !g.thrust_toggle;
                    }

                    // Tap rotation pulses
                    KeyCode::Left | KeyCode::Char('z') | KeyCode::Char('Z') => {
                        g.left_until = now + Duration::from_millis(ROT_PULSE_MS);
                    }
                    KeyCode::Right | KeyCode::Char('x') | KeyCode::Char('X') => {
                        g.right_until = now + Duration::from_millis(ROT_PULSE_MS);
                    }

                    _ => {}
                }
            }
            Event::Resize(w, h) => {
                let now = Instant::now();
                g.width = w;
                g.height = h;
                g.terrain = make_terrain(&mut g.rng, g.width as usize, g.height as usize);
                g.lander = reset_lander(g.width, g.height);
                g.thrust_toggle = false;
                g.left_until = now;
                g.right_until = now;
            }
            _ => {}
        }
    }

    Ok(true)
}

fn main() -> io::Result<()> {
    let mut out = io::stdout();

    terminal::enable_raw_mode()?;
    execute!(out, EnterAlternateScreen, cursor::Hide)?;

    let res = run(&mut out);

    execute!(out, cursor::Show, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    res
}

fn run(out: &mut Stdout) -> io::Result<()> {
    let mut g = init_game()?;

    let dt_fixed = 1.0 / 60.0;
    let frame_cap = Duration::from_millis(16);

    loop {
        let now = Instant::now();
        let dt = (now - g.last_frame).as_secs_f64().min(0.05);
        g.last_frame = now;
        g.accumulator += dt;

        if !handle_input(&mut g)? {
            break;
        }

        update_controls_from_time(&mut g, now);

        while g.accumulator >= dt_fixed {
            update_physics(&mut g, dt_fixed);
            g.accumulator -= dt_fixed;
        }

        render(&g, out)?;
        std::thread::sleep(frame_cap);
    }

    Ok(())
}
