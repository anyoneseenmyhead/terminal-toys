// src/main.rs
use anyhow::Result;
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    queue,
    style::{Color, ResetColor, SetForegroundColor},
    terminal::{
        self, BeginSynchronizedUpdate, DisableLineWrap, EnableLineWrap, EndSynchronizedUpdate,
        EnterAlternateScreen, LeaveAlternateScreen,
    },
};
use glam::{Mat4, Vec2, Vec3, Vec4};
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::{
    collections::{hash_map::Entry, HashMap},
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, PartialEq)]
struct Cell {
    ch: char,
    fg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Reset,
        }
    }
}

struct Term {
    w: u16,
    h: u16,
}

fn braille_from_bits(bits: u8) -> char {
    // Unicode Braille patterns: U+2800 + bitmask
    // Dot numbering:
    // 1 4
    // 2 5
    // 3 6
    // 7 8
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

fn braille_bit(x: usize, y: usize) -> u8 {
    // x in [0..2), y in [0..4)
    // Map to braille dot bits (LSB dot1). Unicode braille uses bits:
    // dot1=0, dot2=1, dot3=2, dot4=3, dot5=4, dot6=5, dot7=6, dot8=7
    match (x, y) {
        (0, 0) => 1 << 0, // dot 1
        (0, 1) => 1 << 1, // dot 2
        (0, 2) => 1 << 2, // dot 3
        (0, 3) => 1 << 6, // dot 7
        (1, 0) => 1 << 3, // dot 4
        (1, 1) => 1 << 4, // dot 5
        (1, 2) => 1 << 5, // dot 6
        (1, 3) => 1 << 7, // dot 8
        _ => 0,
    }
}

fn bayer4(x: usize, y: usize) -> f32 {
    // 4x4 Bayer matrix normalized to [0,1)
    // Values 0..15
    const B: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];
    (B[y & 3][x & 3] as f32 + 0.5) / 16.0
}

struct MicroBuffer {
    mw: usize, // micro width  = term_w * 2
    mh: usize, // micro height = term_h * 4
    z: Vec<f32>,
    luma: Vec<f32>,
    tint: Vec<Vec3>,
}

impl MicroBuffer {
    fn new(term_w: u16, term_h: u16) -> Self {
        let mw = term_w as usize * 2;
        let mh = term_h as usize * 4;
        let n = mw * mh;
        Self {
            mw,
            mh,
            z: vec![f32::INFINITY; n],
            luma: vec![0.0; n],
            tint: vec![Vec3::ZERO; n],
        }
    }

    fn resize_if_needed(&mut self, term_w: u16, term_h: u16) {
        let mw = term_w as usize * 2;
        let mh = term_h as usize * 4;
        if mw == self.mw && mh == self.mh {
            return;
        }
        self.mw = mw;
        self.mh = mh;
        let n = mw * mh;
        self.z.resize(n, f32::INFINITY);
        self.luma.resize(n, 0.0);
        self.tint.resize(n, Vec3::ZERO);
    }

    fn clear(&mut self) {
        self.z.fill(f32::INFINITY);
        self.luma.fill(0.0);
        self.tint.fill(Vec3::ZERO);
    }

    #[inline]
    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.mw + x
    }

    fn plot(&mut self, x: i32, y: i32, depth: f32, lum: f32, tint: Vec3) {
        if x < 0 || y < 0 {
            return;
        }
        let (x, y) = (x as usize, y as usize);
        if x >= self.mw || y >= self.mh {
            return;
        }
        let i = self.idx(x, y);
        if depth < self.z[i] {
            self.z[i] = depth;
            self.luma[i] = lum.clamp(0.0, 1.0);
            self.tint[i] = tint;
        }
    }
}

#[derive(Clone, Copy)]
struct RenderParams {
    near_clip: f32,
    fog_distance: f32,
    micro_contrast: f32,
    cell_contrast: f32,
    dither_strength: f32,
    exposure_target_luma: f32,
    exposure_adapt: f32,
    temporal_luma_alpha: f32,
}

impl Default for RenderParams {
    fn default() -> Self {
        Self {
            near_clip: 0.06,
            fog_distance: 32.0,
            micro_contrast: 1.10,
            cell_contrast: 1.10,
            dither_strength: 0.78,
            exposure_target_luma: 0.45,
            exposure_adapt: 0.08,
            temporal_luma_alpha: 0.28,
        }
    }
}

struct TemporalState {
    luma_hist: Vec<f32>,
    exposure: f32,
}

impl TemporalState {
    fn new() -> Self {
        Self {
            luma_hist: Vec::new(),
            exposure: 1.0,
        }
    }

    fn resize_if_needed(&mut self, n: usize) {
        if self.luma_hist.len() != n {
            self.luma_hist = vec![0.0; n];
            self.exposure = 1.0;
        }
    }

    fn stabilize_microbuffer(&mut self, mb: &mut MicroBuffer, params: &RenderParams) {
        self.resize_if_needed(mb.luma.len());

        // Exposure adapts slowly to avoid abrupt frame-level contrast shifts.
        let mut sum = 0.0f32;
        let mut count = 0usize;
        for &l in &mb.luma {
            if l > 0.001 {
                sum += l;
                count += 1;
            }
        }
        let avg_scene_luma = if count > 0 {
            sum / count as f32
        } else {
            params.exposure_target_luma
        };
        let target_exposure =
            (params.exposure_target_luma / (avg_scene_luma + 1e-4)).clamp(0.75, 1.30);
        self.exposure += (target_exposure - self.exposure) * params.exposure_adapt;

        for i in 0..mb.luma.len() {
            let boosted = (mb.luma[i] * self.exposure).clamp(0.0, 1.0);
            let prev = self.luma_hist[i];
            let smoothed = prev + (boosted - prev) * params.temporal_luma_alpha;
            self.luma_hist[i] = smoothed;
            mb.luma[i] = smoothed;
        }
    }
}

#[derive(Clone, Copy)]
struct Tri {
    a: Vec3, // screen x,y in micro coords; z = depth (camera-space z)
    b: Vec3,
    c: Vec3,
    lum: f32,
    tint: Vec3,
}

fn edge(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    (p.x - a.x) * (b.y - a.y) - (p.y - a.y) * (b.x - a.x)
}

fn raster_tri(mb: &mut MicroBuffer, t: &Tri) {
    // Bounding box
    let ax = t.a.x;
    let ay = t.a.y;
    let bx = t.b.x;
    let by = t.b.y;
    let cx = t.c.x;
    let cy = t.c.y;

    let minx = ax.min(bx).min(cx).floor() as i32;
    let maxx = ax.max(bx).max(cx).ceil() as i32;
    let miny = ay.min(by).min(cy).floor() as i32;
    let maxy = ay.max(by).max(cy).ceil() as i32;

    let a2 = Vec2::new(ax, ay);
    let b2 = Vec2::new(bx, by);
    let c2 = Vec2::new(cx, cy);

    let area = edge(c2, a2, b2);
    if area.abs() < 1e-6 {
        return;
    }

    // Clamp to buffer
    let minx = minx.max(0).min(mb.mw as i32 - 1);
    let maxx = maxx.max(0).min(mb.mw as i32 - 1);
    let miny = miny.max(0).min(mb.mh as i32 - 1);
    let maxy = maxy.max(0).min(mb.mh as i32 - 1);

    for y in miny..=maxy {
        for x in minx..=maxx {
            // Pixel center
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);

            let w0 = edge(p, b2, c2);
            let w1 = edge(p, c2, a2);
            let w2 = edge(p, a2, b2);

            // Same winding
            if (w0 >= 0.0 && w1 >= 0.0 && w2 >= 0.0) || (w0 <= 0.0 && w1 <= 0.0 && w2 <= 0.0) {
                let inv = 1.0 / area;
                let l0 = w0 * inv;
                let l1 = w1 * inv;
                let l2 = w2 * inv;

                // Depth interpolate (camera-space z)
                let depth = t.a.z * l0 + t.b.z * l1 + t.c.z * l2;

                // Dithered luma decision happens later; here we store a continuous luma
                mb.plot(x, y, depth, t.lum, t.tint);
            }
        }
    }
}

struct Camera {
    pos: Vec3,
    yaw: f32,
    pitch: f32,
    fov_y: f32,
}

impl Camera {
    fn view(&self) -> Mat4 {
        let rot = Mat4::from_rotation_y(self.yaw) * Mat4::from_rotation_x(self.pitch);
        let trans = Mat4::from_translation(-self.pos);
        rot * trans
    }

    fn proj(&self, aspect: f32) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, aspect, 0.05, 200.0)
    }
}

#[derive(Clone, Copy)]
struct Cuboid {
    center: Vec3,
    half: Vec3,
    // base brightness bias (rare books can be brighter)
    emit: f32,
    albedo: Vec3,
}

fn face_luma(normal: Vec3, depth_avg: f32, emit: f32, params: &RenderParams) -> f32 {
    let light_dir = Vec3::new(-0.25, 0.65, -0.72).normalize();
    let lambert = normal.dot(light_dir).max(0.0);
    let rim = (1.0 - normal.z.abs()).powf(1.6) * 0.16;
    let base = 0.14 + 0.82 * lambert + rim + emit * 1.05;
    let fog = (1.0 - (depth_avg / params.fog_distance))
        .clamp(0.0, 1.0)
        .powf(1.05);
    (base * fog).clamp(0.0, 1.0)
}

fn face_tint(base: Vec3, normal: Vec3) -> Vec3 {
    let top_light = 0.88 + 0.22 * normal.y.max(0.0);
    let side_cool = 0.90 + 0.10 * normal.x.abs();
    (base * top_light * side_cool).clamp(Vec3::splat(0.0), Vec3::splat(1.0))
}

fn clip_quad_near(face: &[Vec3; 4], near: f32, out: &mut [Vec3; 6]) -> usize {
    let mut n = 0usize;
    let mut prev = face[3];
    let mut prev_in = prev.z <= -near;

    for &curr in face {
        let curr_in = curr.z <= -near;
        if prev_in != curr_in {
            let denom = curr.z - prev.z;
            if denom.abs() > 1e-6 {
                let t = (-near - prev.z) / denom;
                out[n] = prev + (curr - prev) * t.clamp(0.0, 1.0);
                n += 1;
            }
        }
        if curr_in {
            out[n] = curr;
            n += 1;
        }
        prev = curr;
        prev_in = curr_in;
    }
    n
}

fn raster_cuboid(cam: &Mat4, proj: &Mat4, mb: &mut MicroBuffer, c: &Cuboid, params: &RenderParams) {
    // 8 corners in local space
    let h = c.half;
    let p = c.center;

    let corners = [
        p + Vec3::new(-h.x, -h.y, -h.z),
        p + Vec3::new(h.x, -h.y, -h.z),
        p + Vec3::new(h.x, h.y, -h.z),
        p + Vec3::new(-h.x, h.y, -h.z),
        p + Vec3::new(-h.x, -h.y, h.z),
        p + Vec3::new(h.x, -h.y, h.z),
        p + Vec3::new(h.x, h.y, h.z),
        p + Vec3::new(-h.x, h.y, h.z),
    ];

    // Faces as quads (indices into corners)
    // Each face: (i0,i1,i2,i3) CCW in object space
    let faces: [([usize; 4], Vec3); 6] = [
        ([0, 1, 2, 3], Vec3::new(0.0, 0.0, -1.0)), // back
        ([4, 5, 6, 7], Vec3::new(0.0, 0.0, 1.0)),  // front
        ([0, 4, 7, 3], Vec3::new(-1.0, 0.0, 0.0)), // left
        ([1, 5, 6, 2], Vec3::new(1.0, 0.0, 0.0)),  // right
        ([3, 2, 6, 7], Vec3::new(0.0, 1.0, 0.0)),  // top
        ([0, 1, 5, 4], Vec3::new(0.0, -1.0, 0.0)), // bottom
    ];

    let vw = *cam;
    let vp = *proj;
    let mut clipped = [Vec3::ZERO; 6];
    let mut proj_poly = [Vec3::ZERO; 6];

    for (quad, n_obj) in faces {
        // Transform normal into camera space (rotation part only)
        let n4 = vw * Vec4::new(n_obj.x, n_obj.y, n_obj.z, 0.0);
        let n = Vec3::new(n4.x, n4.y, n4.z).normalize();

        // Backface cull in camera space: we want faces pointing toward camera (negative z in RH view)
        // In camera space, camera looks down -Z in right-handed clip conventions used by glam's perspective_rh.
        // We can approximate by checking normal.z < 0 (facing camera).
        if n.z >= 0.0 {
            continue;
        }

        // Build face polygon in camera space, then clip against near plane
        let mut face_cam = [Vec3::ZERO; 4];
        for (i, &ci) in quad.iter().enumerate() {
            let v = corners[ci];
            let v_cam4 = vw * v.extend(1.0);
            face_cam[i] = Vec3::new(v_cam4.x, v_cam4.y, v_cam4.z);
        }

        let clipped_n = clip_quad_near(&face_cam, params.near_clip, &mut clipped);
        if clipped_n < 3 {
            continue;
        }

        let mut zsum = 0.0f32;
        let mut proj_n = 0usize;
        for &v_cam in clipped.iter().take(clipped_n) {
            zsum += -v_cam.z;

            let clip = vp * v_cam.extend(1.0);
            if clip.w.abs() < 1e-6 {
                proj_n = 0;
                break;
            }
            let ndc = clip / clip.w;
            let sx = (ndc.x * 0.5 + 0.5) * (mb.mw as f32 - 1.0);
            let sy = (1.0 - (ndc.y * 0.5 + 0.5)) * (mb.mh as f32 - 1.0);
            proj_poly[proj_n] = Vec3::new(sx, sy, -v_cam.z);
            proj_n += 1;
        }
        if proj_n < 3 {
            continue;
        }
        let zavg = zsum / proj_n as f32;

        // Lighting + fog
        let lum = face_luma(n, zavg, c.emit, params);
        let tint = face_tint(c.albedo, n);

        // Fan triangulation after clipping.
        for i in 1..(proj_n - 1) {
            let tri = Tri {
                a: proj_poly[0],
                b: proj_poly[i],
                c: proj_poly[i + 1],
                lum,
                tint,
            };
            raster_tri(mb, &tri);
        }
    }
}

#[derive(Hash, PartialEq, Eq, Clone, Copy)]
struct ChunkKey {
    layer: u8,
    id: i64,
}

#[derive(Clone)]
struct Chunk {
    cuboids: Vec<Cuboid>,
}

fn gen_chunk(seed: u64, layer: u8, id: i64) -> Chunk {
    // Chunk spans along z
    let chunk_len = 10.0;
    let z0 = id as f32 * chunk_len;

    let mut rng = ChaCha8Rng::seed_from_u64(seed ^ ((layer as u64) << 56) ^ (id as u64));

    // Corridor params
    let corridor_half = 2.2 + layer as f32 * 0.5;
    let shelf_levels = match layer {
        0 => 2,
        1 => 3,
        _ => 4,
    };

    let mut cuboids = Vec::new();
    let shelf_tint = Vec3::new(0.55, 0.47, 0.37);
    let rib_tint = Vec3::new(0.45, 0.47, 0.50);
    let rail_tint = Vec3::new(0.50, 0.50, 0.56);
    let arch_tint = Vec3::new(0.56, 0.54, 0.50);
    let book_palette = [
        Vec3::new(0.86, 0.22, 0.24),
        Vec3::new(0.20, 0.56, 0.87),
        Vec3::new(0.26, 0.72, 0.45),
        Vec3::new(0.90, 0.64, 0.18),
        Vec3::new(0.58, 0.43, 0.83),
        Vec3::new(0.79, 0.34, 0.58),
        Vec3::new(0.14, 0.62, 0.62),
        Vec3::new(0.84, 0.44, 0.26),
    ];

    // Make shelf planks (long thin boxes) and books sitting on them
    for si in 0..shelf_levels {
        let y = -0.4 + si as f32 * 0.65;

        // Shelf planks left and right
        for &side in &[-1.0f32, 1.0f32] {
            let x = side * corridor_half;
            // plank
            cuboids.push(Cuboid {
                center: Vec3::new(x, y - 0.18, z0 + chunk_len * 0.5),
                half: Vec3::new(0.65, 0.03, chunk_len * 0.5),
                emit: 0.0,
                albedo: shelf_tint,
            });

            // Vertical posts that break up long shelves
            for post_i in 0..4 {
                let pz = z0 + (post_i as f32 + 1.0) * (chunk_len / 5.0);
                cuboids.push(Cuboid {
                    center: Vec3::new(x, y + 0.18, pz),
                    half: Vec3::new(0.035, 0.35, 0.03),
                    emit: 0.0,
                    albedo: shelf_tint,
                });
            }

            // Books along the plank
            let mut z = z0;
            while z < z0 + chunk_len {
                let w = rng.gen_range(0.04..0.10);
                let h = rng.gen_range(0.18..0.42);
                let d = rng.gen_range(0.06..0.12);

                let rare = rng.gen_bool(0.007);
                let emit = if rare { 0.35 } else { 0.0 };
                let mut book_tint = book_palette[rng.gen_range(0..book_palette.len())];
                let tint_jitter = Vec3::new(
                    rng.gen_range(0.90..1.10),
                    rng.gen_range(0.90..1.10),
                    rng.gen_range(0.90..1.10),
                );
                book_tint = (book_tint * tint_jitter).clamp(Vec3::splat(0.0), Vec3::splat(1.0));

                // Slightly inset from shelf edge
                let bx = x - side * rng.gen_range(0.08..0.16);

                cuboids.push(Cuboid {
                    center: Vec3::new(bx, y + h * 0.5, z + d),
                    half: Vec3::new(w * 0.5, h * 0.5, d * 0.5),
                    emit,
                    albedo: book_tint,
                });

                // Small horizontal stack on some positions for silhouette variation.
                if rng.gen_bool(0.12) {
                    let stack_h = rng.gen_range(0.02..0.05);
                    let stack_d = rng.gen_range(0.05..0.10);
                    cuboids.push(Cuboid {
                        center: Vec3::new(
                            bx - side * 0.04,
                            y + h + stack_h * 0.5 + 0.01,
                            z + d + rng.gen_range(-0.01..0.01),
                        ),
                        half: Vec3::new(w * 0.55, stack_h * 0.5, stack_d * 0.5),
                        emit: emit * 0.6,
                        albedo: (book_tint * 0.90).clamp(Vec3::splat(0.0), Vec3::splat(1.0)),
                    });
                }

                z += w + rng.gen_range(0.01..0.035);
            }
        }
    }

    // Add subtle wall ribs for depth
    for k in 0..rng.gen_range(6..12) {
        let z = z0 + (k as f32 + 0.5) * (chunk_len / 12.0);
        let y = rng.gen_range(-0.9..1.3);
        let rib_h = rng.gen_range(0.35..0.75);
        let rib_w = rng.gen_range(0.03..0.07);

        for &side in &[-1.0f32, 1.0f32] {
            let x = side * (corridor_half + 0.35);
            cuboids.push(Cuboid {
                center: Vec3::new(x, y, z),
                half: Vec3::new(rib_w, rib_h, 0.03),
                emit: 0.0,
                albedo: rib_tint,
            });
        }
    }

    // Floor and ceiling runners add stronger motion parallax.
    for lane in 0..3 {
        let lane_ofs = lane as f32 - 1.0;
        let lx = lane_ofs * (corridor_half * 0.32);

        cuboids.push(Cuboid {
            center: Vec3::new(lx, -0.95, z0 + chunk_len * 0.5),
            half: Vec3::new(0.08, 0.02, chunk_len * 0.5),
            emit: 0.0,
            albedo: rail_tint,
        });
        cuboids.push(Cuboid {
            center: Vec3::new(lx, 1.55, z0 + chunk_len * 0.5),
            half: Vec3::new(0.06, 0.02, chunk_len * 0.5),
            emit: 0.0,
            albedo: rail_tint,
        });
    }

    // Occasional crossing arches to deepen corridor rhythm.
    for arch_i in 0..rng.gen_range(2..4) {
        let az = z0 + (arch_i as f32 + 0.7) * (chunk_len / 3.2);
        cuboids.push(Cuboid {
            center: Vec3::new(0.0, 1.25, az),
            half: Vec3::new(corridor_half + 0.35, 0.03, 0.05),
            emit: 0.0,
            albedo: arch_tint,
        });
        for &side in &[-1.0f32, 1.0f32] {
            cuboids.push(Cuboid {
                center: Vec3::new(side * (corridor_half + 0.35), 0.55, az),
                half: Vec3::new(0.03, 0.70, 0.05),
                emit: 0.0,
                albedo: arch_tint,
            });
        }
    }

    Chunk { cuboids }
}

fn lerp_vec3(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

fn apply_contrast(v: f32, contrast: f32) -> f32 {
    ((v - 0.5) * contrast + 0.5).clamp(0.0, 1.0)
}

fn depth_tint(depth_norm: f32) -> Vec3 {
    let near = Vec3::new(1.00, 0.74, 0.42);
    let mid = Vec3::new(0.75, 0.82, 1.00);
    let far = Vec3::new(0.42, 0.56, 0.95);

    if depth_norm < 0.5 {
        lerp_vec3(near, mid, depth_norm * 2.0)
    } else {
        lerp_vec3(mid, far, (depth_norm - 0.5) * 2.0)
    }
}

fn rgb_from_luma_depth(luma: f32, depth_norm: f32, local_tint: Vec3) -> Color {
    let depth = depth_tint(depth_norm);
    let tint = lerp_vec3(depth, local_tint, 0.55);
    let light = (0.06 + 0.94 * luma.clamp(0.0, 1.0)).powf(1.02);
    let rgb = tint * light;

    Color::Rgb {
        r: (rgb.x.clamp(0.0, 1.0) * 255.0) as u8,
        g: (rgb.y.clamp(0.0, 1.0) * 255.0) as u8,
        b: (rgb.z.clamp(0.0, 1.0) * 255.0) as u8,
    }
}

fn main() -> Result<()> {
    let mut stdout = io::stdout();

    terminal::enable_raw_mode()?;
    queue!(stdout, EnterAlternateScreen, cursor::Hide, DisableLineWrap)?;
    terminal::Clear(terminal::ClearType::All);
    stdout.flush()?;

    let mut prev: Vec<Cell> = Vec::new();
    let mut curr: Vec<Cell> = Vec::new();

    let mut term = {
        let (w, h) = terminal::size()?;
        Term { w, h }
    };

    let mut mb = MicroBuffer::new(term.w, term.h);
    let mut temporal = TemporalState::new();
    let params = RenderParams::default();

    // World
    let seed = 0xA11CE_u64;
    let mut chunks: HashMap<ChunkKey, Chunk> = HashMap::new();

    let mut cam = Camera {
        pos: Vec3::new(0.0, 0.25, 0.0),
        yaw: 0.0,
        pitch: 0.0,
        fov_y: 58f32.to_radians(),
    };

    let t0 = Instant::now();
    let mut last = Instant::now();

    // Motion
    let mut z_travel = 0.0f32;

    // Frame cap
    let frame_dt = Duration::from_millis(16);

    'outer: loop {
        // Events
        while event::poll(Duration::from_millis(0))? {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break 'outer,
                    _ => {}
                },
                Event::Resize(w, h) => {
                    term.w = w;
                    term.h = h;
                }
                _ => {}
            }
        }

        // Timing
        let now = Instant::now();
        let dt = (now - last).as_secs_f32().min(0.05);
        last = now;
        let t = (now - t0).as_secs_f32();

        // Resize buffers
        let (w, h) = terminal::size()?;
        if w != term.w || h != term.h {
            term.w = w;
            term.h = h;
        }
        mb.resize_if_needed(term.w, term.h);

        let n_cells = term.w as usize * term.h as usize;
        if prev.len() != n_cells {
            prev = vec![Cell::default(); n_cells];
            curr = vec![Cell::default(); n_cells];
        }

        // Camera motion
        z_travel += dt * 1.35;
        cam.pos.z = -z_travel;
        cam.yaw = 0.04 * (t * 0.35).sin();
        cam.pitch = 0.02 * (t * 0.55).sin();
        cam.pos.y = 0.28 + 0.06 * (t * 1.1).sin();

        // Build matrices
        let view = cam.view();
        let proj = cam.proj(term.w as f32 / term.h as f32);

        // Determine needed chunks based on forward motion along -Z.
        let chunk_len = 10.0f32;
        let cam_chunk = (cam.pos.z / chunk_len).floor() as i64;
        let keep_forward = 14i64;
        let keep_back = 3i64;
        let chunk_min = cam_chunk - keep_forward;
        let chunk_max = cam_chunk + keep_back;

        // Drop old chunks occasionally (simple sweep)
        chunks.retain(|k, _| k.id >= chunk_min && k.id <= chunk_max);

        // 3 layers for richness (0 near, 1 mid, 2 far)
        for layer in 0u8..=2u8 {
            for cid in chunk_min..=chunk_max {
                let key = ChunkKey { layer, id: cid };
                if let Entry::Vacant(v) = chunks.entry(key) {
                    v.insert(gen_chunk(seed, layer, cid));
                }
            }
        }

        // Render into microbuffer
        mb.clear();

        // Painter order: far to near (higher layer first), z-buffer still enforces correctness
        for layer in (0u8..=2u8).rev() {
            for cid in chunk_min..=chunk_max {
                if let Some(ch) = chunks.get(&ChunkKey { layer, id: cid }) {
                    for cub in &ch.cuboids {
                        raster_cuboid(&view, &proj, &mut mb, cub, &params);
                    }
                }
            }
        }

        // Temporal anti-flicker and adaptive exposure on luma.
        temporal.stabilize_microbuffer(&mut mb, &params);

        // Convert microbuffer -> terminal cells (Braille + per-cell grayscale)
        let tw = term.w as usize;
        let th = term.h as usize;

        for y in 0..th {
            for x in 0..tw {
                let mut bits: u8 = 0;
                let mut lum_acc = 0.0f32;
                let mut depth_acc = 0.0f32;
                let mut depth_n = 0.0f32;
                let mut tint_acc = Vec3::ZERO;
                let mut tint_w = 0.0f32;

                // 2x4 micro pixels per cell
                for sy in 0..4 {
                    for sx in 0..2 {
                        let mx = x * 2 + sx;
                        let my = y * 4 + sy;
                        let i = mb.idx(mx, my);
                        let lum = apply_contrast(mb.luma[i], params.micro_contrast);
                        lum_acc += lum;
                        let depth = mb.z[i];
                        if depth.is_finite() {
                            depth_acc += depth;
                            depth_n += 1.0;
                            tint_acc += mb.tint[i] * (0.15 + lum * 0.85);
                            tint_w += 1.0;
                        }

                        // Ordered dithering for a soft gradient
                        let thr = 0.5 + (bayer4(mx, my) - 0.5) * params.dither_strength;
                        if lum > thr {
                            bits |= braille_bit(sx, sy);
                        }
                    }
                }

                let avg = apply_contrast(lum_acc / 8.0, params.cell_contrast);
                let depth_avg = if depth_n > 0.0 {
                    depth_acc / depth_n
                } else {
                    30.0
                };
                let depth_norm = ((depth_avg - 1.0) / 26.0).clamp(0.0, 1.0);
                let local_tint = if tint_w > 0.0 {
                    (tint_acc / tint_w).clamp(Vec3::splat(0.0), Vec3::splat(1.0))
                } else {
                    Vec3::new(0.7, 0.74, 0.82)
                };
                let ch = braille_from_bits(bits);
                let fg = if bits == 0 {
                    Color::Reset
                } else {
                    rgb_from_luma_depth(0.05 + 0.95 * avg, depth_norm, local_tint)
                };

                curr[y * tw + x] = Cell { ch, fg };
            }
        }

        // Diff flush
        queue!(stdout, BeginSynchronizedUpdate)?;
        let mut active_fg = Color::Reset;
        for y in 0..th {
            for x in 0..tw {
                let i = y * tw + x;
                let c = curr[i];
                if c != prev[i] {
                    queue!(stdout, cursor::MoveTo(x as u16, y as u16))?;
                    if c.fg != active_fg {
                        if c.fg != Color::Reset {
                            queue!(stdout, SetForegroundColor(c.fg))?;
                        } else {
                            queue!(stdout, ResetColor)?;
                        }
                        active_fg = c.fg;
                    }
                    queue!(stdout, crossterm::style::Print(c.ch))?;
                    prev[i] = c;
                }
            }
        }
        queue!(stdout, ResetColor, EndSynchronizedUpdate)?;
        stdout.flush()?;

        // Frame cap
        let elapsed = now.elapsed();
        if elapsed < frame_dt {
            std::thread::sleep(frame_dt - elapsed);
        }
    }

    // Cleanup
    queue!(
        stdout,
        ResetColor,
        EnableLineWrap,
        cursor::Show,
        LeaveAlternateScreen
    )?;
    stdout.flush()?;
    terminal::disable_raw_mode()?;
    Ok(())
}
