//! Etterna difficulty calculator overlay for Quaver.
//!
//! This application monitors the Quaver "Now Playing" directory for map changes
//! and displays real-time difficulty calculations using the Etterna Calc.

use std::{
    collections::HashMap, env, fs, path::{Path, PathBuf}, time::Duration,
};

use iced::{
    Color, Element, Event, Point, Size, Subscription, Task, Theme, event, futures::{SinkExt, Stream, channel::mpsc::Sender}, mouse::{self, Button}, widget::{center_x, center_y, column, row, scrollable, text}, window::{self, Id, Level, Position, Settings},
};
use minacalc_rs::{Calc, CalcMode, Note, SkillsetScores};
use notify::RecursiveMode;
use notify_debouncer_mini::{Config, DebouncedEvent, new_debouncer_opt};
use serde::Deserialize;
use walkdir::WalkDir;

// ============================================================================
// MODELS
// ============================================================================

/// Quaver hit object representing a single note.
#[derive(Deserialize, Clone)]
struct HitObject {
    #[serde(rename = "StartTime")]
    start_time: Option<u32>,
    #[serde(rename = "Lane")]
    lane: u32
}

/// Quaver map metadata and hit objects.
#[derive(Deserialize, Clone)]
struct Map {
    #[serde(rename = "Title")]
    title: String,
    #[serde(rename = "Artist")]
    artist: String,
    #[serde(rename = "DifficultyName")]
    difficulty_name: String,
    #[serde(rename = "HitObjects")]
    hit_objects: Vec<HitObject>,
    #[serde(rename = "Mode")]
    mode: String
}

/// File change detection results.
struct FileChanges {
    next_mapid: Option<String>,
    next_rate: Option<f32>,
}

/// Application messages.
#[derive(Clone)]
enum Message {
    ChangeText(String),
    UpdateCalc {
        map: Map, 
        key_count: u32,
        rate: f32
    },
    MousePressed(mouse::Button),
    MouseMoved(Point),
    MouseReleased(mouse::Button),
    WindowOpened(Id)
}

/// Main application overlay state.
struct Overlay {
    calc: Calc,
    status_text: String,
    show_status: bool,
    current_map: Option<Map>,
    current_rate: Option<f32>,
    current_score: Option<SkillsetScores>,
    is_dragging: bool,
    window_id: Id,
    previous_mouse_position: Point,
    window_position: Point
}

// ============================================================================
// UI LAYER
// ============================================================================

impl Overlay {
    /// Creates a new overlay instance, initializing the Etterna calculator.
    fn new() -> Self {
        Self {
            calc: Calc::new().expect("Etterna Calc should launch"),
            status_text: String::from("Waiting for map update"),
            show_status: true,
            current_map: None,
            current_rate: None,
            current_score: None,
            is_dragging: false,
            window_id: Id::unique(),
            previous_mouse_position: Point::ORIGIN,
            window_position: Point::ORIGIN
        }
    }

    /// Processes incoming messages and updates internal state.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::ChangeText(text) => {
                self.status_text = text;
                self.show_status = true;
                Task::none()
            },
            Message::UpdateCalc {
                map,
                key_count,
                rate
            } => {
                let (normalized_rate, score) = compute_etterna_difficulty(&self.calc, &map.hit_objects, key_count, rate);
                self.show_status = false;
                self.current_map = Some(map);
                self.current_rate = Some(normalized_rate);
                self.current_score = Some(score);
                Task::none()
            },
            Message::MousePressed(mouse::Button::Left) => {
                self.is_dragging = true;
                Task::none()
            }
            Message::MouseMoved(mouse_position) => {
                if self.is_dragging {
                    let drag_offset = (mouse_position - self.previous_mouse_position)/2.0;
                    self.window_position = self.window_position + drag_offset;
                    return window::move_to(
                        self.window_id, 
                        self.window_position
                    );
                }
                self.previous_mouse_position = mouse_position;
                Task::none()
                
            }
            Message::MouseReleased(mouse::Button::Left) => {
                self.is_dragging = false;
                Task::none()
            },
            Message::WindowOpened(id)  => {
                self.window_id = id;
                Task::none()
            }
            _  => {Task::none()}
        }
    }

    /// Renders the current UI state.
    fn view(&self) -> Element<'_, Message> {
        if self.show_status {
            center_y(scrollable(center_x(text(&self.status_text))).spacing(10)).padding(10).into()
        } else {
            let map = self.current_map.as_ref().expect("map should be set when not showing status");
            let score = self.current_score.as_ref().expect("score should be set when not showing status");
            let rate = self.current_rate.expect("rate should be set when not showing status");
            center_y(scrollable(center_x(
        column![
                    column![
                        center_x(text(format!("{} - {}", map.artist, map.title)).size(20)),
                        center_x(row![text(&map.difficulty_name).size(18), text(format!("{rate:.2}")).size(18).color(color_from_rate(rate))].spacing(10))
                    ].padding(10),
                    center_x(row![
                        column![
                            text("Overall"),
                            text("Stream"),
                            text("Jumpstream"),
                            text("Handstream"),
                            text("Stamina"),
                            text("Jackspeed"),
                            text("Chordjack"),
                            text("Technical")
                        ].spacing(10),
                        column![
                            text(format!("{:.2}", score.overall)).color(color_from_difficulty(score.overall)),
                            text(format!("{:.2}", score.stream)).color(color_from_difficulty(score.stream)),
                            text(format!("{:.2}", score.jumpstream)).color(color_from_difficulty(score.jumpstream)),
                            text(format!("{:.2}", score.handstream)).color(color_from_difficulty(score.handstream)),
                            text(format!("{:.2}", score.stamina)).color(color_from_difficulty(score.stamina)),
                            text(format!("{:.2}", score.jackspeed)).color(color_from_difficulty(score.jackspeed)),
                            text(format!("{:.2}", score.chordjack)).color(color_from_difficulty(score.chordjack)),
                            text(format!("{:.2}", score.technical)).color(color_from_difficulty(score.technical))
                        ].spacing(10)
                    ].spacing(50))
                ]))).into()
        }
    }

    /// Creates a subscription that combine every other subscription.
    fn subscription(&self) -> Subscription<Message> {
        Subscription::batch(vec![
            self.subscription_file_watcher(),
            self.subscription_event()
        ])
    }

    /// Creates the file watcher subscription.
    fn subscription_file_watcher(&self) -> Subscription<Message> {
        Subscription::run(Overlay::file_watcher)
    }

    /// Worker task that monitors file changes and emits messages.
    fn file_watcher() -> impl Stream<Item = Message>  {
        iced::stream::channel(100, file_watcher_task)
    }

    /// Creates an app event subscription.
    fn subscription_event(&self) -> Subscription<Message> {
        event::listen_with(|event, _status, id| {
            match event {
                Event::Mouse(iced::mouse::Event::CursorMoved { position}) => {
                    Some(Message::MouseMoved(position))
                },
                Event::Mouse(iced::mouse::Event::ButtonPressed(Button::Left)) => {
                    Some(Message::MousePressed(Button::Left))
                }
                Event::Mouse(iced::mouse::Event::ButtonReleased(Button::Left)) => {
                    Some(Message::MouseReleased(Button::Left))
                }
                Event::Window(iced::window::Event::Opened { position: _, size: _ }) => {
                    Some(Message::WindowOpened(id))
                }
                _ => None,
            }
        })
    }
}

// ============================================================================
// FILE MONITORING & ASYNC TASK
// ============================================================================

/// Main async task for file monitoring and map updates.
async fn file_watcher_task(mut output: Sender<Message>) {
    // Get quaver path
    let quaver_path = get_quaver_installation_path();
    
    if !quaver_path.exists() {
        eprintln!("Quaver installation could not be found, please give a correct path as argument of this program.");
        std::process::exit(1);
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
                eprintln!("Failed to initialize file watcher: {e}");
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

/// Gets the Quaver installation path from command-line args or system defaults.
fn get_quaver_installation_path() -> PathBuf {
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

    let mut args = env::args().skip(1);
    args.next().map(PathBuf::from).unwrap_or(quaver_installation_default)
}

// ============================================================================
// HELPER FUNCTIONS - DIFFICULTY CALCULATION
// ============================================================================

/// Computes Etterna difficulty rating and skillset scores for a map.
///
/// Converts Quaver hit objects to Etterna note format and calculates
/// difficulty across all playback rates.
fn compute_etterna_difficulty(calc: &Calc, hit_objects: &[HitObject], key_count: u32, rate: f32) -> (f32, SkillsetScores) {
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

// ============================================================================
// HELPER FUNCTIONS - STYLE UTILITIES
// ============================================================================

/// Maps difficulty value to a color gradient: blue -> green -> red.
///
/// - 0-20: Blue to Green
/// - 20-40: Green to Red
fn color_from_difficulty(value: f32) -> Color {
    let normalized = (value.clamp(0.0, 40.0) / 40.0).clamp(0.0, 1.0);
    gradient_color(normalized)
}

/// Maps rate value to a color gradient: blue -> green -> red.
///
/// - 0.7x: Blue
/// - 1.0x: Green
/// - 2.0x: Red
fn color_from_rate(rate: f32) -> Color {
    let normalized = ((rate.clamp(0.7, 2.0) - 0.7) / 1.3).clamp(0.0, 1.0);
    gradient_color(normalized)
}

/// Generates a color along the blue -> green -> red gradient.
///
/// Normalized value should be in [0, 1]:
/// - 0.0 = Blue (0, 0, 1)
/// - 0.5 = Green (0, 1, 0)
/// - 1.0 = Red (1, 0, 0)
fn gradient_color(normalized: f32) -> Color {
    if normalized <= 0.5 {
        // Blue to Green: [0, 0.5]
        let t = normalized / 0.5;
        Color::from_rgb(0.0, t, 1.0 - t)
    } else {
        // Green to Red: [0.5, 1.0]
        let t = (normalized - 0.5) / 0.5;
        Color::from_rgb(t, 1.0 - t, 0.0)
    }
}

/// Calculates the bottom-left corner position for the overlay window.
fn bottom_left_position(window_size: Size<f32>, monitor_size: Size<f32>) -> Point<f32> {
    Point::new(0.0, monitor_size.height - window_size.height)
}

// ============================================================================
// APPLICATION ENTRY POINT
// ============================================================================

/// Runs the Etterna difficulty overlay application.
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window_settings = Settings {
        size: Size::new(400.0, 450.0),
        position: Position::SpecificWith(bottom_left_position),
        resizable: false,
        decorations: false,
        level: Level::AlwaysOnTop,
        ..Settings::default()
    };

    let _ = iced::application(Overlay::new, Overlay::update, Overlay::view)
        .window(window_settings)
        .subscription(Overlay::subscription)
        .theme(Theme::CatppuccinMocha)
        .run();
    Ok(())
}