# Terminal Toys

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

##Installing Rust and Cargo

All projects in this repository are written in Rust and use Cargo, Rust’s build system and package manager.

#Windows

Download and run rustup-init.exe from:
https://www.rust-lang.org/tools/install

During installation, choose the default options.

Open Command Prompt or PowerShell and verify:

rustc --version
cargo --version


If both commands print version numbers, installation succeeded.

Linux

Most Linux users should install Rust using rustup (recommended).

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh


Follow the prompts and accept the default installation.

Then restart your shell and verify:

rustc --version
cargo --version

macOS

Install Command Line Tools if you do not already have them:

xcode-select --install


Install Rust using rustup:

curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh


Restart your terminal and verify:

rustc --version
cargo --version

Updating Rust

If Rust is already installed, update it with:

rustup update

Troubleshooting

If cargo is not found, ensure ~/.cargo/bin is in your PATH.

On Windows, restart your terminal after installation.

These projects assume stable Rust; nightly is not required.

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

![Raymarch screenshot](screenshots/raymarch.png)

### Aurora  
A simulation of aurora borealis-style light curtains using terminal graphics.

![Aurora screenshot](screenshots/aurora.png)




### Boids  
A classic boids flocking simulation adapted for terminal rendering.

![Boids screenshot](screenshots/boids.png)


### Cmatrix2  
A Matrix-inspired falling-glyph visualization.

![Cmatrix2 screenshot](screenshots/cmatrix2.png)


### Fluidlite Braille  
A fluid simulation rendered using Unicode Braille for higher vertical resolution.

![Fluidlite Braille screenshot](screenshots/fluidlite.png)


### Fountain  
A particle fountain simulation rendered in the terminal.

![Fountain screenshot](screenshots/fountain.png)


### Frogger  
A playable Frogger-style arcade game in the console.

![Frogger screenshot](screenshots/frogger.png)


### Grayscott  
A reaction-diffusion (Gray–Scott) simulation visualized in the terminal.

![Grayscott screenshot](screenshots/grayscott.png)


### Lunarlander  
A terminal-based lunar lander game with physics and input control.

![Lunarlander screenshot](screenshots/lunarlander.png)


### Mazewalker  
A maze navigation and exploration simulation.

![Mazewalker screenshot](screenshots/mazewalker.png)


### Newton  
A physics-based Newton’s cradle style simulation.

![Newton screenshot](screenshots/newton.png)


### Orrery  
A planetary or orbital system visualization rendered in the terminal.

![Orrery screenshot](screenshots/orrery.png)


### Pipes  
A dynamic pipe-routing animation inspired by classic screensavers.

![Pipes screenshot](screenshots/pipes.png)


### Planetarium  
A rotating planet and celestial visualization with informational overlays.

![Planetarium screenshot](screenshots/planetarium.png)


### Plasmaglobe  
A plasma-style energy globe effect adapted to terminal graphics.

![Plasmaglobe screenshot](screenshots/plasmaglobe.png)


### Retrowave  
An infinite synthwave-style road and horizon animation.

![Retrowave screenshot](screenshots/retrowave.png)


### Starfield  
A 3D starfield flight simulation rendered in the terminal.

![Starfield screenshot](screenshots/starfield.png)


### Tenprint  
A continuously generating maze based on the classic 10 PRINT algorithm.

![Tenprint screenshot](screenshots/tenprint.png)


### Termpath  
A terminal-based pathfinding or traversal visualization.

![Termpath screenshot](screenshots/termpath.png)


### Terrarium Braille  
A cellular ecosystem simulation rendered with Braille characters.

![Terrarium Braille screenshot](screenshots/terrarium.png)


### Unsinkable  
A buoyancy and stability simulation rendered in the terminal.

![Unsinkable screenshot](screenshots/unsinkable.png)


### Voronoi  
A Voronoi diagram and region growth visualization.

![Voronoi screenshot](screenshots/voronoi.png)


### Weather  
A terminal-based weather visualization and data display tool.

![Weather screenshot](screenshots/weather.png)


---

## Notes

- Each project is independent. There is no shared build system.
- Many projects are experimental and may prioritize visual output over strict realism.
- Expect terminal-specific behavior and performance differences.
