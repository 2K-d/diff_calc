//! Etterna difficulty calculator overlay for Quaver.
//!
//! This application monitors the Quaver "Now Playing" directory for map changes
//! and displays real-time difficulty calculations using the Etterna Calc.

#![windows_subsystem = "windows"]

mod calc;
mod gui;
use crate::gui::launch_gui;

/// Runs the Etterna difficulty overlay application.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    launch_gui()?;
    Ok(())
}