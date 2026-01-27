use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEvent, KeyModifiers},
    execute, queue,
    style::{Color, Print, ResetColor, SetForegroundColor},
    terminal::{self, Clear, ClearType, DisableLineWrap, EnableLineWrap, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, VecDeque},
    io::{self, Write},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Cell {
    Empty,
    Wall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Alg {
    BFS,
    Dijkstra,
    AStar,
}

#[derive(Clone, Debug)]
struct SearchState {
    alg: Alg,
    running: bool,
    finished: bool,
    found: bool,

    // Per-node data for rendering
    visited: Vec<bool>,
    in_frontier: Vec<bool>,
    dist: Vec<u32>,
    prev: Vec<Option<usize>>,

    // Frontier containers
    bfs_q: VecDeque<usize>,
    heap: BinaryHeap<Node>,
}

#[derive(Clone, Copy, Debug)]
struct Node {
    idx: usize,
    // For heap ordering
    f: u32,
    g: u32,
}

impl Ord for Node {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse for min-heap behavior: smaller f, then smaller g
        other
            .f
            .cmp(&self.f)
            .then_with(|| other.g.cmp(&self.g))
            .then_with(|| other.idx.cmp(&self.idx))
    }
}
impl PartialOrd for Node {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.idx == other.idx && self.f == other.f && self.g == other.g
    }
}
impl Eq for Node {}

struct App {
    w: usize,
    h: usize,
    cells: Vec<Cell>,

    cursor_x: usize,
    cursor_y: usize,

    start: usize,
    end: usize,

    search: Option<SearchState>,

    // UI
    status: String,
    last_tick: Instant,
    tick_ms: u64,

    // NEW: wall draw mode
    draw_walls: bool,
}

fn main() -> io::Result<()> {
    let mut stdout = io::stdout();
    terminal::enable_raw_mode()?;
    execute!(stdout, EnterAlternateScreen, DisableLineWrap, cursor::Hide)?;
    let res = run(&mut stdout);
    // Always restore terminal
    execute!(stdout, ResetColor, cursor::Show, EnableLineWrap, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;
    res
}

fn run(stdout: &mut io::Stdout) -> io::Result<()> {
    let (tw, th) = terminal::size()?;
    let mut app = App::new(tw as usize, th as usize);
    app.set_status("Arrows move | Space wall-draw toggle | X toggle wall | S start | E end | B BFS | D Dijkstra | A A* | R reset search | C clear | Q quit");

    loop {
        // Render
        app.render(stdout)?;

        // Drive search animation at a steady tick
        let now = Instant::now();
        let due = now.duration_since(app.last_tick).as_millis() as u64 >= app.tick_ms;

        // Input
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(k) => {
                    if handle_key(&mut app, k) {
                        return Ok(());
                    }
                }
                Event::Resize(nw, nh) => {
                    app.resize(nw as usize, nh as usize);
                }
                _ => {}
            }
        }

        // Search step
        if due {
            app.last_tick = now;
            app.step_search();
        }
    }
}

fn handle_key(app: &mut App, k: KeyEvent) -> bool {
    // Quit on Ctrl+C as well
    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
        return true;
    }

    match k.code {
        KeyCode::Char('q') | KeyCode::Char('Q') => return true,

        KeyCode::Left => app.move_cursor(-1, 0),
        KeyCode::Right => app.move_cursor(1, 0),
        KeyCode::Up => app.move_cursor(0, -1),
        KeyCode::Down => app.move_cursor(0, 1),

        // NEW: Space toggles wall draw mode
        KeyCode::Char(' ') => {
            app.draw_walls = !app.draw_walls;
            if app.draw_walls {
                app.set_status(
                    "Wall draw: ON | Arrows paint walls | Space to exit | X toggles a single wall",
                );

                // Optional: paint current cell immediately when turning on
                let idx = app.idx(app.cursor_x, app.cursor_y);
                if idx != app.start && idx != app.end && app.cells[idx] != Cell::Wall {
                    app.cells[idx] = Cell::Wall;
                    app.invalidate_search();
                }
            } else {
                app.set_status("Wall draw: OFF | Space to enter draw mode | X toggles a single wall");
            }
        }

        // NEW: X toggles a single wall (old behavior)
        KeyCode::Char('x') | KeyCode::Char('X') => {
            let idx = app.idx(app.cursor_x, app.cursor_y);
            if idx != app.start && idx != app.end {
                app.cells[idx] = match app.cells[idx] {
                    Cell::Empty => Cell::Wall,
                    Cell::Wall => Cell::Empty,
                };
                app.invalidate_search();
            }
        }

        KeyCode::Char('s') | KeyCode::Char('S') => {
            let idx = app.idx(app.cursor_x, app.cursor_y);
            if app.cells[idx] != Cell::Wall && idx != app.end {
                app.start = idx;
                app.invalidate_search();
            }
        }
        KeyCode::Char('e') | KeyCode::Char('E') => {
            let idx = app.idx(app.cursor_x, app.cursor_y);
            if app.cells[idx] != Cell::Wall && idx != app.start {
                app.end = idx;
                app.invalidate_search();
            }
        }

        KeyCode::Char('r') | KeyCode::Char('R') => {
            app.search = None;
            app.set_status("Search cleared. Press B, D, or A to run again.");
        }
        KeyCode::Char('c') | KeyCode::Char('C') => {
            for i in 0..app.cells.len() {
                app.cells[i] = Cell::Empty;
            }
            // Keep start/end inside bounds and distinct
            if app.start == app.end {
                app.end = app.end.saturating_add(1).min(app.cells.len().saturating_sub(1));
            }
            app.search = None;
            app.draw_walls = false;
            app.set_status("Cleared walls + search. Wall draw: OFF.");
        }

        KeyCode::Char('b') | KeyCode::Char('B') => app.start_search(Alg::BFS),
        KeyCode::Char('d') | KeyCode::Char('D') => app.start_search(Alg::Dijkstra),
        KeyCode::Char('a') | KeyCode::Char('A') => app.start_search(Alg::AStar),

        _ => {}
    }
    false
}

impl App {
    fn new(term_w: usize, term_h: usize) -> Self {
        // Leave room for a 2-line HUD at bottom
        let hud_h = 2usize;
        let h = term_h.saturating_sub(hud_h).max(5);
        let w = term_w.max(10);

        let mut cells = vec![Cell::Empty; w * h];

        // Default start/end
        let start = (h / 2) * w + (w / 4);
        let end = (h / 2) * w + (3 * w / 4).min(w - 1);

        // Ensure empty
        cells[start] = Cell::Empty;
        cells[end] = Cell::Empty;

        Self {
            w,
            h,
            cells,
            cursor_x: w / 2,
            cursor_y: h / 2,
            start,
            end,
            search: None,
            status: String::new(),
            last_tick: Instant::now(),
            tick_ms: 10, // animation speed
            draw_walls: false,
        }
    }

    fn resize(&mut self, term_w: usize, term_h: usize) {
        let hud_h = 2usize;
        let new_h = term_h.saturating_sub(hud_h).max(5);
        let new_w = term_w.max(10);

        if new_w == self.w && new_h == self.h {
            return;
        }

        let mut new_cells = vec![Cell::Empty; new_w * new_h];

        // Copy overlap region
        let copy_w = self.w.min(new_w);
        let copy_h = self.h.min(new_h);
        for y in 0..copy_h {
            for x in 0..copy_w {
                new_cells[y * new_w + x] = self.cells[y * self.w + x];
            }
        }

        self.w = new_w;
        self.h = new_h;
        self.cells = new_cells;

        self.cursor_x = self.cursor_x.min(self.w - 1);
        self.cursor_y = self.cursor_y.min(self.h - 1);

        // Clamp start/end into bounds
        self.start = self.start.min(self.cells.len().saturating_sub(1));
        self.end = self.end.min(self.cells.len().saturating_sub(1));
        if self.start == self.end {
            self.end = (self.end + 1).min(self.cells.len() - 1);
        }

        self.invalidate_search();
        self.draw_walls = false;
        self.set_status("Resized. Search cleared. Wall draw: OFF.");
    }

    fn idx(&self, x: usize, y: usize) -> usize {
        y * self.w + x
    }

    fn xy(&self, idx: usize) -> (usize, usize) {
        (idx % self.w, idx / self.w)
    }

    // UPDATED: paints walls while moving if draw_walls is on
    fn move_cursor(&mut self, dx: i32, dy: i32) {
        let nx = (self.cursor_x as i32 + dx).clamp(0, (self.w - 1) as i32) as usize;
        let ny = (self.cursor_y as i32 + dy).clamp(0, (self.h - 1) as i32) as usize;

        self.cursor_x = nx;
        self.cursor_y = ny;

        if self.draw_walls {
            let idx = self.idx(self.cursor_x, self.cursor_y);
            if idx != self.start && idx != self.end && self.cells[idx] != Cell::Wall {
                self.cells[idx] = Cell::Wall;
                self.invalidate_search();
            }
        }
    }

    fn set_status(&mut self, s: impl Into<String>) {
        self.status = s.into();
    }

    fn invalidate_search(&mut self) {
        self.search = None;
    }

    fn start_search(&mut self, alg: Alg) {
        if self.start == self.end {
            self.set_status("Start and End are the same. Move one of them.");
            return;
        }
        if self.cells[self.start] == Cell::Wall || self.cells[self.end] == Cell::Wall {
            self.set_status("Start/End must be on empty cells.");
            return;
        }

        let n = self.cells.len();
        let mut st = SearchState {
            alg,
            running: true,
            finished: false,
            found: false,
            visited: vec![false; n],
            in_frontier: vec![false; n],
            dist: vec![u32::MAX; n],
            prev: vec![None; n],
            bfs_q: VecDeque::new(),
            heap: BinaryHeap::new(),
        };

        st.dist[self.start] = 0;

        match alg {
            Alg::BFS => {
                st.bfs_q.push_back(self.start);
                st.in_frontier[self.start] = true;
            }
            Alg::Dijkstra => {
                st.heap.push(Node {
                    idx: self.start,
                    f: 0,
                    g: 0,
                });
                st.in_frontier[self.start] = true;
            }
            Alg::AStar => {
                let h = self.manhattan(self.start, self.end);
                st.heap.push(Node {
                    idx: self.start,
                    f: h,
                    g: 0,
                });
                st.in_frontier[self.start] = true;
            }
        }

        self.search = Some(st);
        self.set_status(match alg {
            Alg::BFS => "Running BFS (press R to reset search)",
            Alg::Dijkstra => "Running Dijkstra (press R to reset search)",
            Alg::AStar => "Running A* (press R to reset search)",
        });
    }

    fn manhattan(&self, a: usize, b: usize) -> u32 {
        let (ax, ay) = self.xy(a);
        let (bx, by) = self.xy(b);
        ax.abs_diff(bx) as u32 + ay.abs_diff(by) as u32
    }

    fn step_search(&mut self) {
        // Grab a few immutable things up front (no borrow of self.search yet)
        let w = self.w;
        let h = self.h;
        let end = self.end;

        // Helper closures that do not borrow self
        let xy = |idx: usize| (idx % w, idx / w);
        let manhattan = |a: usize, b: usize| -> u32 {
            let (ax, ay) = xy(a);
            let (bx, by) = xy(b);
            ax.abs_diff(bx) as u32 + ay.abs_diff(by) as u32
        };
        let neighbors = |idx: usize| -> [Option<usize>; 4] {
            let (x, y) = xy(idx);
            let up = if y > 0 { Some((y - 1) * w + x) } else { None };
            let down = if y + 1 < h { Some((y + 1) * w + x) } else { None };
            let left = if x > 0 { Some(y * w + (x - 1)) } else { None };
            let right = if x + 1 < w { Some(y * w + (x + 1)) } else { None };
            [up, down, left, right]
        };

        let Some(st) = &mut self.search else { return };
        if !st.running || st.finished {
            return;
        }

        // Pop next
        let cur = match st.alg {
            Alg::BFS => st.bfs_q.pop_front(),
            Alg::Dijkstra | Alg::AStar => st.heap.pop().map(|n| n.idx),
        };

        let Some(cur) = cur else {
            st.running = false;
            st.finished = true;
            st.found = false;
            self.set_status("No path found. (R to reset search)");
            return;
        };

        if st.visited[cur] {
            // For heaps, we may pop stale entries
            return;
        }

        st.in_frontier[cur] = false;
        st.visited[cur] = true;

        if cur == end {
            st.running = false;
            st.finished = true;
            st.found = true;
            self.set_status("Path found! (R to reset search)");
            return;
        }

        let base_g = st.dist[cur];

        for nb in neighbors(cur).into_iter().flatten() {
            if self.cells[nb] == Cell::Wall {
                continue;
            }
            if st.visited[nb] {
                continue;
            }

            let new_g = base_g.saturating_add(1);

            match st.alg {
                Alg::BFS => {
                    if st.dist[nb] == u32::MAX {
                        st.dist[nb] = new_g;
                        st.prev[nb] = Some(cur);
                        st.bfs_q.push_back(nb);
                        st.in_frontier[nb] = true;
                    }
                }
                Alg::Dijkstra => {
                    if new_g < st.dist[nb] {
                        st.dist[nb] = new_g;
                        st.prev[nb] = Some(cur);
                        st.heap.push(Node {
                            idx: nb,
                            f: new_g,
                            g: new_g,
                        });
                        st.in_frontier[nb] = true;
                    }
                }
                Alg::AStar => {
                    if new_g < st.dist[nb] {
                        st.dist[nb] = new_g;
                        st.prev[nb] = Some(cur);
                        let h = manhattan(nb, end);
                        let f = new_g.saturating_add(h);
                        st.heap.push(Node { idx: nb, f, g: new_g });
                        st.in_frontier[nb] = true;
                    }
                }
            }
        }
    }

    fn render(&self, stdout: &mut io::Stdout) -> io::Result<()> {
        queue!(stdout, cursor::MoveTo(0, 0))?;
    
        // Precompute path cells if found
        let mut on_path = vec![false; self.cells.len()];
        if let Some(st) = &self.search {
            if st.finished && st.found {
                let mut cur = self.end;
                while cur != self.start {
                    on_path[cur] = true;
                    if let Some(p) = st.prev[cur] {
                        cur = p;
                    } else {
                        break;
                    }
                }
                on_path[self.start] = true;
            }
        }
    
        // Draw grid
        for y in 0..self.h {
            for x in 0..self.w {
                let idx = self.idx(x, y);
    
                // Layer order: cursor, start/end, walls, path, visited/frontier, empty
                let (ch, color) = if x == self.cursor_x && y == self.cursor_y {
                    ('@', Color::Yellow)
                } else if idx == self.start {
                    ('S', Color::Green)
                } else if idx == self.end {
                    ('E', Color::Red)
                } else if self.cells[idx] == Cell::Wall {
                    ('#', Color::DarkGrey)
                } else if on_path[idx] {
                    ('*', Color::Cyan)
                } else if let Some(st) = &self.search {
                    if st.in_frontier[idx] {
                        ('o', Color::Magenta)
                    } else if st.visited[idx] {
                        ('·', Color::Blue)
                    } else {
                        ('.', Color::DarkGrey)
                    }
                } else {
                    ('.', Color::DarkGrey)
                };
    
                queue!(stdout, SetForegroundColor(color), Print(ch), ResetColor)?;
            }
            queue!(stdout, Print("\r\n"))?;
        }
    
        // HUD (pad/truncate + clear lines to prevent garble)
        let (tw, _) = terminal::size()?;
        let tw = tw as usize;
    
        let alg_label = match self.search.as_ref().map(|s| s.alg) {
            Some(Alg::BFS) => "BFS",
            Some(Alg::Dijkstra) => "Dijkstra",
            Some(Alg::AStar) => "A*",
            None => "None",
        };
    
        let mode = if self.draw_walls { "ON" } else { "OFF" };
    
        let info = format!(
            "Alg: {} | Grid: {}x{} | Cursor: ({},{}) | Start: {} | End: {} | Wall draw: {}",
            alg_label, self.w, self.h, self.cursor_x, self.cursor_y, self.start, self.end, mode
        );
    
        fn fit_line(s: &str, width: usize) -> String {
            let mut out = s.to_string();
            if out.len() > width {
                out.truncate(width);
            } else if out.len() < width {
                out.push_str(&" ".repeat(width - out.len()));
            }
            out
        }
    
        let info = fit_line(&info, tw);
        let status = fit_line(&self.status, tw);
    
        queue!(
            stdout,
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::White),
            Print(info),
            ResetColor,
            Print("\r\n"),
            Clear(ClearType::CurrentLine),
            SetForegroundColor(Color::DarkGrey),
            Print(status),
            ResetColor
        )?;
    
        stdout.flush()?;
        Ok(())
    }
    }
