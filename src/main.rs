use std::{collections::HashMap, env, fs, path::PathBuf, time::Duration};
use serde::Deserialize;
use minacalc_rs::{Calc, CalcMode, Note, SkillsetScores};
use notify::{RecursiveMode};
use notify_debouncer_mini::{new_debouncer_opt, Config};
use walkdir::WalkDir;
use iced::{Color, Element, Point, Size, Subscription, Theme, font::Style, futures::{SinkExt, Stream}, widget::{
    center_x, center_y, column, row, scrollable, text,
}, window::{Level, Position, Settings}};

// QUA MODEL
#[allow(non_snake_case)]
#[derive(Deserialize, Clone)]
struct HitObject {
    StartTime: Option<u32>,
    Lane: u32
}

#[allow(non_snake_case)]
#[derive(Deserialize, Clone)]
struct Map {
    Title: String,
    Artist: String,
    DifficultyName: String,
    HitObjects: Vec<HitObject>,
    Mode: String
}

struct Overlay {
    calc: Calc,
    text: String,
    show_text: bool,
    map: Option<Map>,
    rate: Option<f32>,
    score: Option<SkillsetScores>
}

#[derive(Clone)]
enum Message {
    ChangeText(String),
    UpdateCalc(Map, u32, f32),
}

impl Overlay {
    fn new() -> Self {
        Self {
            calc: Calc::new().expect("Etterna Calc should launch"),
            text: String::from("Waiting for map update"),
            show_text: true,
            map: None,
            rate: None,
            score: None
        }
    }

    fn update(&mut self, message: Message) {
        match message {
            Message::ChangeText(text) => {
                self.text = text;
                self.show_text = true;
            },
            Message::UpdateCalc(map, key, rate) => {
                let (rate, score) = compute_etterna_difficulty(&self.calc, &map.HitObjects, key, rate);
                self.show_text = false;
                self.map = Some(map);
                self.rate = Some(rate);
                self.score = Some(score);
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.show_text {
            return column![
                center_y(scrollable(center_x(text(&self.text))).spacing(10)).padding(10)
            ].into()
        } else {
            let map = self.map.clone().unwrap();
            let score = self.score.unwrap();
            let rate = self.rate.unwrap();
            return center_y(scrollable(center_x(
        column![
                    column![
                        center_x(text(format!("{} - {}", map.Artist, map.Title)).size(20)),
                        center_x(row![text(map.DifficultyName).size(18), text(format!("{:.2}", rate)).size(18).color(color_rate(rate))].spacing(10))
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
                            text(format!("{:.2}", score.overall)).color(color_diff(score.overall)),
                            text(format!("{:.2}", score.stream)).color(color_diff(score.stream)),
                            text(format!("{:.2}", score.jumpstream)).color(color_diff(score.jumpstream)),
                            text(format!("{:.2}", score.handstream)).color(color_diff(score.handstream)),
                            text(format!("{:.2}", score.stamina)).color(color_diff(score.stamina)),
                            text(format!("{:.2}", score.jackspeed)).color(color_diff(score.jackspeed)),
                            text(format!("{:.2}", score.chordjack)).color(color_diff(score.chordjack)),
                            text(format!("{:.2}", score.technical)).color(color_diff(score.technical))
                        ].spacing(10)
                    ].spacing(50))
                ]))).into()
        }
    }

    fn subscription(&self) -> Subscription<Message> {
        return Subscription::run(Overlay::worker);
    }

    fn worker() -> impl Stream<Item = Message>  {
        return iced::stream::channel(100, async |mut output| {
            // Get quaver path
            let quaver_installation_path = get_quaver_installation_path();
            
            if !quaver_installation_path.exists() {
                println!("Quaver installation could not be found, please give a correct path as argument of this program.");
                std::process::exit(1);
            }
            
            let now_playing_path = quaver_installation_path.join("Data").join("Temp").join("Now Playing");
            let mapid_path = now_playing_path.join("mapid.txt");
            let mods_path = now_playing_path.join("mods.txt");
            let songs_path = quaver_installation_path.join("Songs");

            // Setup file watcher
            let (tx, rx) = std::sync::mpsc::channel();

            let backend_config = notify::Config::default()
                .with_poll_interval(Duration::from_millis(250));
            
            let debouncer_config = Config::default()
                .with_timeout(Duration::from_millis(1000))
                .with_notify_config(backend_config);

            let mut debouncer = new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, tx).unwrap();
            debouncer.watcher().watch(&mapid_path, RecursiveMode::NonRecursive).unwrap();
            debouncer.watcher().watch(&mods_path, RecursiveMode::NonRecursive).unwrap();

            // Track last state to know what changed
            let mut current_mapid: Option<String> = None;
            let mut current_rate: Option<f32> = None;
            let mut current_parsed_map: Option<Map> = None;

            // On file update
            loop {
                let recv_result = rx.try_recv();
                match recv_result {
                    Ok(debouncer_result) => {
                        match debouncer_result {
                            Ok(events) => {
                                let mut should_redraw = false;
                                let mut new_mapid = current_mapid.clone();
                                let mut new_rate = current_rate.clone();

                                // Check if mapid changed
                                if events.iter().any(|event| event.path.eq(&mapid_path)) {
                                    new_mapid = fs::read_to_string(&mapid_path)
                                        .map_err(|e| {println!("mapid read error: {e}");})
                                        .and_then(|id| Ok(id.trim().to_owned()))
                                        .ok();

                                    if new_mapid.is_some() && new_mapid != current_mapid {
                                        should_redraw = true;
                                    }
                                }

                                // Check if mods changed
                                if events.iter().any(|event| event.path.eq(&mods_path)) {
                                    new_rate = fs::read_to_string(&mods_path)
                                        .map_err(|e| {println!("mods read error: {e}");})
                                        .and_then(|mods| Ok(parse_rate_from_mods(&mods)))
                                        .ok();

                                    if new_rate.is_some() && new_rate != current_rate {
                                        should_redraw = true;
                                    }
                                }

                                // Only proceed if something actually changed
                                if !should_redraw {
                                    continue;
                                }
                                        
                                // If mapid changed -> re-read and parse the .qua file
                                if new_mapid != current_mapid {
                                    let qua = find_qua_file(&songs_path, &new_mapid.as_ref().unwrap());

                                    if qua.is_none() {
                                        let _ = output.send(Message::ChangeText(String::from("Could not find currently playing map"))).await;
                                        continue;
                                    }

                                    let parsed_map = fs::read_to_string(&qua.unwrap())
                                        .map_err(|e| {println!("qua read error: {e}");})
                                        .and_then(|yaml_text| serde_yaml::from_str::<Map>(&yaml_text)
                                            .map_err(|e| {println!("Could not parse map: {e}");}))
                                        .ok();

                                    match parsed_map {
                                        Some(m) => {
                                            current_parsed_map = Some(m);
                                            current_mapid = new_mapid;
                                        }
                                        None => continue,
                                    }
                                }
                                // Update state
                                current_rate = new_rate;

                                // Get the current map (either just loaded or previously loaded)
                                let map = match &current_parsed_map {
                                    Some(m) => m,
                                    None => continue,
                                };

                                // Find keys of map
                                let n_key : u32 = map.Mode
                                    .strip_prefix("Keys")
                                    .and_then(|key| key.parse().ok())
                                    .expect("missing 'Keys' prefix or could not parse number");

                                if n_key.lt(&2) {
                                    let _ = output.send(Message::ChangeText(String::from("Cannot do difficulty calculation on less than 2K"))).await;
                                    continue;
                                }

                                let _ = output.send(Message::UpdateCalc(map.clone(), n_key, new_rate.unwrap_or(1.0))).await;
                            },
                            Err(e) => {println!("file watcher error: {:?}", e);},
                        }
                    }
                    Err(_) => {tokio::time::sleep(Duration::from_millis(500)).await;}
                } 
                
            }
        });
    }
}



// FUNCTIONS
fn color_diff(v: f32) -> Color {
    let x = v.clamp(0.0, 40.0) / 40.0; // normalize to [0, 1]
    if x <= 0.5 { // 0.0..0.5 : blue -> green
        let t = x / 0.5; // 0..1
        let r = 0.0;
        let g = t;
        let b = 1.0 - t;
        Color::from_rgb(r, g, b)
    } else { // 0.5..1.0 : green -> red
        let t = (x - 0.5) / 0.5; // 0..1
        let r = t;
        let g = 1.0 - t;
        let b = 0.0;
        Color::from_rgb(r, g, b)
    }
}

fn color_rate(v: f32) -> Color {
    let x = (v.clamp(0.5, 2.0) - 0.5) / 1.5; // normalize to [0, 1]
    if x <= 0.5 { // 0.0..0.5 : blue -> green
        let t = x / 0.5; // 0..1
        let r = 0.0;
        let g = t;
        let b = 1.0 - t;
        Color::from_rgb(r, g, b)
    } else { // 0.5..1.0 : green -> red
        let t = (x - 0.5) / 0.5; // 0..1
        let r = t;
        let g = 1.0 - t;
        let b = 0.0;
        Color::from_rgb(r, g, b)
    }
}

fn compute_etterna_difficulty(calc: &Calc, hit_objects: &Vec<HitObject>, n_key: u32, rate: f32) -> (f32, SkillsetScores) {
    // Transform quaver to etterna format
    let mut sums: HashMap<u32, u32> = HashMap::new();
    
    for it in hit_objects {
        *sums.entry(it.StartTime.unwrap_or(0)).or_insert(0) += 1 << (it.Lane - 1);
    }

    let mut result: Vec<Note> = sums
        .into_iter()
        .map(|(start_time, lane)| Note { row_time: start_time as f32 / 1000.0, notes: lane })
        .collect();
    result.sort_by(|note_a, note_b| note_a.row_time.partial_cmp(&note_b.row_time).unwrap());

    // Do difficulty calculation
    let scores = calc
        .calc_all_rates(&result, n_key, CalcMode::Msd)
        .expect("calculation of difficulty should work");
    
    // Print result
    let rate_idx = ((rate - 0.7) / 0.1).round() as usize;
    let rate_etterna  = rate_idx as f32 * 0.1 + 0.7;
    let score = scores.rates[rate_idx];

    return (rate_etterna, score);
}

fn find_qua_file(songs_path: &PathBuf, map_id: &str) -> Option<PathBuf> {
    // Find Qua file from mapid
    let file_searched = format!("{map_id}.qua");
    return WalkDir::new(songs_path)
        .into_iter()
        .filter_map(Result::ok)
        .find(|entry| entry.file_type().is_file() && entry.file_name().to_str() == Some(&file_searched))
        .map(|entry| entry.into_path());
}

fn parse_rate_from_mods(mods: &str) -> f32 {
    // Take the prefix before the first 'x' should be the rate
    return mods.split_once('x')
        .map(|(rate_part, _)| rate_part.trim())
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.0);
}

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
    return args.next().map(PathBuf::from).unwrap_or(quaver_installation_default);
}

fn bottom_left(window_size: Size<f32>, monitor_size: Size<f32>) -> Point<f32> {
    return (0.0, monitor_size.height - window_size.height).into();
}

// DO
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let window_settings = Settings {
        size: (400, 450).into(),
        position: Position::SpecificWith(bottom_left),
        resizable: true,
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