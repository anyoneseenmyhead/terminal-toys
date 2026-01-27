# Publish

A curated collection of terminal-based Rust applications and simulations.  
Each subdirectory is a standalone Cargo crate designed to run directly in a modern terminal.

These projects focus on:
- real-time terminal rendering
- ASCII and Braille-based graphics
- interactive simulations and games
- experimentation with terminal performance, input, and animation

This repository is intended as both a playground and a portfolio of terminal UI work.

---

## Requirements

- Rust (stable)
- A UTF-8 capable terminal  
- Recommended terminals: Kitty, Alacritty, Windows Terminal

Some projects use Unicode Braille characters and benefit from truecolor support.

---

## Running a project

```sh
cd <project>
cargo run --release
```

---

## Building a project

```sh
cd <project>
cargo build --release
```

---

## Projects

### Aquarium  
An interactive, animated aquarium rendered in the terminal.
![Aquarium screenshot](screenshots/aquarium.png)

### Ascii Raymarch  
A real-time ASCII raymarching experiment in the console.

### Aurora  
A simulation of aurora borealis-style light curtains using terminal graphics.

### Boids  
A classic boids flocking simulation adapted for terminal rendering.

### Cmatrix2  
A Matrix-inspired falling-glyph visualization.

### Fluidlite Braille  
A fluid simulation rendered using Unicode Braille for higher vertical resolution.

### Fountain  
A particle fountain simulation rendered in the terminal.

### Frogger  
A playable Frogger-style arcade game in the console.

### Grayscott  
A reaction-diffusion (Gray–Scott) simulation visualized in the terminal.

### Lunarlander  
A terminal-based lunar lander game with physics and input control.

### Mazewalker  
A maze navigation and exploration simulation.

### Newton  
A physics-based Newton’s cradle style simulation.

### Orrery  
A planetary or orbital system visualization rendered in the terminal.

### Pipes  
A dynamic pipe-routing animation inspired by classic screensavers.

### Planetarium  
A rotating planet and celestial visualization with informational overlays.

### Plasmaglobe  
A plasma-style energy globe effect adapted to terminal graphics.

### Retrowave  
An infinite synthwave-style road and horizon animation.

### Starfield  
A 3D starfield flight simulation rendered in the terminal.

### Tenprint  
A continuously generating maze based on the classic 10 PRINT algorithm.

### Termpath  
A terminal-based pathfinding or traversal visualization.

### Terrarium Braille  
A cellular ecosystem simulation rendered with Braille characters.

### Unsinkable  
A buoyancy and stability simulation rendered in the terminal.

### Voronoi  
A Voronoi diagram and region growth visualization.

### Weather  
A terminal-based weather visualization and data display tool.

---

## Notes

- Each project is independent. There is no shared build system.
- Many projects are experimental and may prioritize visual output over strict realism.
- Expect terminal-specific behavior and performance differences.
