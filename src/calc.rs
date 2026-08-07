//! This file has the code responsible for difficulty calculation

use std::{
    collections::HashMap, fs, path::{Path, PathBuf}, time::Duration,
};
use iced::futures::{SinkExt, channel::mpsc::Sender};
use minacalc_rs::{Calc, CalcMode, Note, SkillsetScores};
use notify::RecursiveMode;
use notify_debouncer_mini::{Config, DebouncedEvent, new_debouncer_opt};
use rfd::{FileDialog, MessageDialog};
use serde::Deserialize;
use walkdir::WalkDir;

use crate::gui::Message;

// ============================================================================
// MODELS
// ============================================================================

/// Quaver hit object representing a single note.
#[derive(Deserialize, Clone)]
pub struct HitObject {
    #[serde(rename = "StartTime")]
    start_time: Option<u32>,
    #[serde(rename = "Lane")]
    lane: u32
}

/// Quaver map metadata and hit objects.
#[derive(Deserialize, Clone)]
pub struct Map {
    #[serde(rename = "Title")]
    pub title: String,
    #[serde(rename = "Artist")]
    pub artist: String,
    #[serde(rename = "DifficultyName")]
    pub difficulty_name: String,
    #[serde(rename = "HitObjects")]
    pub hit_objects: Vec<HitObject>,
    #[serde(rename = "Mode")]
    mode: String
}

/// File change detection results.
struct FileChanges {
    next_mapid: Option<String>,
    next_rate: Option<f32>,
}

// ============================================================================
// FILE MONITORING & ASYNC TASK
// ============================================================================

/// Main async task for file monitoring and map updates.
pub async fn file_watcher_task(mut output: Sender<Message>) {
    // Get quaver path
    let mut quaver_path = get_quaver_default_installation_path();
    
    if !quaver_path.exists() {
        MessageDialog::new()
            .set_title("Quaver installation not found")
            .set_description("Quaver installation could not be found.\nPlease select the root directory of your Quaver installation.")
            .show();
        quaver_path = FileDialog::new()
            .set_title("Select Quaver Installation Folder")
            .pick_folder()
            .unwrap();
    }
    
    let now_playing_path = quaver_path.join("Data").join("Temp").join("Now Playing");
    let mapid_path = now_playing_path.join("mapid.txt");
    let mods_path = now_playing_path.join("mods.txt");
    let songs_path = quaver_path.join("Songs");

    // Setup file watcher
    let (tx, rx) = std::sync::mpsc::channel();
    let backend_config = notify::Config::default()
        .with_poll_interval(Duration::from_millis(250));
    let debouncer_config = Config::default()
        .with_timeout(Duration::from_millis(1000))
        .with_notify_config(backend_config);

    let mut debouncer =
        match new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, tx) {
            Ok(d) => d,
            Err(e) => {
                MessageDialog::new()
                    .set_title("File Watcher Problem")
                    .set_description(format!("Could not create a file watcher to observe files change: {e}"))
                    .set_level(rfd::MessageLevel::Error)
                    .show();
                std::process::exit(2);
            }
        };
    debouncer.watcher().watch(&mapid_path, RecursiveMode::NonRecursive).unwrap();
    debouncer.watcher().watch(&mods_path, RecursiveMode::NonRecursive).unwrap();

    // Track last state to know what changed
    let mut current_mapid: Option<String> = None;
    let mut current_map: Option<Map> = None;
    let mut current_rate: Option<f32> = None;

    // On file update
    loop {
        match  rx.try_recv() {
            Ok(Ok(events)) => {
                let mut should_redraw = false;
                let changes = detect_relevant_changes(&events, &mapid_path, &mods_path);

                // If mapid changed -> re-read and parse the .qua file
                if let Some(ref new_mapid) = changes.next_mapid
                    && Some(new_mapid) != current_mapid.as_ref() {
                    match find_and_parse_map(&songs_path, new_mapid) {
                        Ok(map) => {
                            current_map = Some(map);
                            current_mapid = Some(new_mapid.clone());
                            should_redraw = true;
                        }
                        Err(err) => {
                            let _ = output.send(Message::ChangeText(format!("Map error: {err}"))).await;
                            continue;
                        }
                    }
                }

                if let Some(new_rate) = changes.next_rate
                    && Some(new_rate) != current_rate {
                    current_rate = Some(new_rate);
                    should_redraw = true;
                }

                // Only proceed if something actually changed
                if !should_redraw {
                    continue;
                }

                // Get the current map (either just loaded or previously loaded)
                let map = match current_map.as_ref() {
                    Some(m) => m.clone(),
                    None => continue,
                };

                // Find keys of map
                let key_count = match parse_key_count(&map.mode) {
                    Ok(count) if count >= 2 => count,
                    Ok(_) => {
                        let _ = output.send(Message::ChangeText(String::from("Cannot calculate difficulty for less than 2K"))).await;
                        continue;
                    }
                    Err(err) => {
                        let _ = output.send(Message::ChangeText(format!("Mode error: {err}"))).await;
                        continue;
                    }
                };

                let rate = current_rate.unwrap_or(1.0);
                let _ = output.send(Message::UpdateCalc {
                    map,
                    key_count,
                    rate
                }).await;
            },
            Ok(Err(e)) => {eprintln!("File watcher error: {e:?}");},
            Err(_) => tokio::time::sleep(Duration::from_millis(500)).await
        }
    }
}

// ============================================================================
// HELPER FUNCTIONS - FILE OPERATIONS
// ============================================================================

/// Detects which files changed and extracts their new values.
fn detect_relevant_changes(events: &[DebouncedEvent], mapid_path: &Path, mods_path: &Path) -> FileChanges {
    let mapid_changed = events.iter().any(|e| e.path == mapid_path);
    let mods_changed = events.iter().any(|e| e.path == mods_path);

    // Read file contents only if that file changed.
    let next_mapid = if mapid_changed {
        fs::read_to_string(mapid_path)
            .ok()
            .map(|s| s.trim().to_string())
    } else {
        None
    };

    let next_rate = if mods_changed {
        fs::read_to_string(mods_path)
            .ok()
            .and_then(|s| parse_rate_from_mods(&s).ok())
    } else {
        None
    };

    FileChanges {
        next_mapid,
        next_rate,
    }
}

/// Finds and parses a map file by ID.
fn find_and_parse_map(songs_path: &PathBuf, map_id: &str) -> Result<Map, String> {
    let qua_path = find_qua_file(songs_path, map_id)
        .ok_or_else(|| format!("Could not find map with ID: {map_id}"))?;

    read_and_parse_map(&qua_path)
        .ok_or_else(|| format!("Failed to parse map: {}", qua_path.display()))
}

/// Locates a .qua file by map ID using directory traversal.
fn find_qua_file(songs_path: &PathBuf, map_id: &str) -> Option<PathBuf> {
    let file_searched = format!("{map_id}.qua");
    WalkDir::new(songs_path)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().is_file() && entry.file_name().to_str() == Some(&file_searched))
        .map(|entry| entry.into_path())
}

/// Parses a YAML .qua file into a Map struct.
fn read_and_parse_map(qua_path: &Path) -> Option<Map> {
    let yaml_text = fs::read_to_string(qua_path).ok()?;
    serde_yaml::from_str::<Map>(&yaml_text).ok()
}

// ============================================================================
// HELPER FUNCTIONS - PARSING & VALIDATION
// ============================================================================

/// Extracts key count from Quaver mode string (e.g., "Keys4" -> 4).
fn parse_key_count(mode: &str) -> Result<u32, String> {
    mode.strip_prefix("Keys")
        .ok_or_else(|| format!("Invalid mode format: {mode}"))?
        .parse::<u32>()
        .map_err(|_| format!("Could not parse key count from: {mode}"))
}

/// Extracts playback rate from mods string (e.g., "1.5x" -> 1.5).
fn parse_rate_from_mods(mods: &str) -> Result<f32, String> {
    mods.split_once('x')
        .ok_or_else(|| "No 'x' found in mods string".to_string())?
        .0
        .trim()
        .parse::<f32>()
        .map_err(|_| "Could not parse rate as f32".to_string())
}

/// Gets the Quaver installation path from system defaults.
fn get_quaver_default_installation_path() -> PathBuf {
    // Get quaver installation path
    #[cfg(target_os = "windows")]
    let quaver_installation_default = PathBuf::from("C:\\Program Files (x86)\\Steam\\steamapps\\common\\Quaver");

    #[cfg(target_os = "linux")]
    let quaver_installation_default = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from("."))
        .join(".local").join("share").join("Steam").join("steamapps").join("common").join("Quaver");

    #[cfg(target_os = "macos")]
    let quaver_installation_default = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or(PathBuf::from("."))
        .join("Library").join("Application Support").join("Steam").join("steamapps").join("common").join("Quaver");

    quaver_installation_default
}

// ============================================================================
// HELPER FUNCTIONS - DIFFICULTY CALCULATION
// ============================================================================

/// Computes Etterna difficulty rating and skillset scores for a map.
///
/// Converts Quaver hit objects to Etterna note format and calculates
/// difficulty across all playback rates.
pub fn compute_etterna_difficulty(calc: &Calc, hit_objects: &[HitObject], key_count: u32, rate: f32) -> (f32, SkillsetScores) {
    // Convert Quaver hit objects to Etterna note format
    let notes = convert_to_etterna_notes(hit_objects);

    // Calculate difficulty across all rates
    let scores = calc
        .calc_all_rates(&notes, key_count, CalcMode::Msd)
        .expect("Etterna difficulty calculation failed");

    // Normalize rate to nearest Etterna rate step (0.7 + 0.1*n)
    let normalized_rate = normalize_rate(rate);

    let rate_index = ((normalized_rate - 0.7) / 0.1).round() as usize;

    // Retrieve score for the closest rate
    let score = scores
        .rates
        .get(rate_index)
        .copied()
        .unwrap_or_else(|| scores.rates[0]);

    (normalized_rate, score)
}

/// Converts Quaver hit objects to Etterna note format.
///
/// Aggregates simultaneous hits into chord bitmasks and sorts by time.
fn convert_to_etterna_notes(hit_objects: &[HitObject]) -> Vec<Note> {
    let mut notes_by_time: HashMap<u32, u32> = HashMap::new();

    // Aggregate lanes into bitmasks for each timestamp
    for obj in hit_objects {
        let start_time = obj.start_time.unwrap_or(0);
        let lane_bitmask = 1u32 << (obj.lane.saturating_sub(1));
        *notes_by_time.entry(start_time).or_insert(0) |= lane_bitmask;
    }

    // Convert to Note format and sort
    let mut notes: Vec<Note> = notes_by_time
        .into_iter()
        .map(|(start_time, lanes)| Note {
            row_time: start_time as f32 / 1000.0,
            notes: lanes,
        })
        .collect();

    notes.sort_by(|a, b| a.row_time.partial_cmp(&b.row_time).unwrap_or(std::cmp::Ordering::Equal));
    notes
}

/// Normalizes playback rate to nearest Etterna rate step.
///
/// Etterna rates are: 0.7, 0.8, 0.9, 1.0, 1.1, ..., 2.0
fn normalize_rate(rate: f32) -> f32 {
    ((rate.clamp(0.7, 2.0) - 0.7) / 0.1).round() * 0.1 + 0.7
}