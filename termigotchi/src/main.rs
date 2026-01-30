mod app;
mod config;
mod input;
mod model;
mod render;
mod sim;
mod storage;

use anyhow::Result;

fn main() -> Result<()> {
    app::run()
}
