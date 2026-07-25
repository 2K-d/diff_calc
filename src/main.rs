use std::{collections::HashMap, env, fs, io::stdout, path::PathBuf, time::Duration};
use serde::Deserialize;
use minacalc_rs::{Calc, CalcMode, Note, SkillsetScores};
use notify::{RecursiveMode};
use notify_debouncer_mini::{new_debouncer_opt, Config};
use walkdir::WalkDir;
use crossterm::{
    execute,
    terminal::{Clear, ClearType},
    cursor::MoveTo
};

// QUAVER MODEL
#[allow(non_snake_case)]
#[derive(Deserialize)]
struct HitObject {
    StartTime: Option<u32>,
    Lane: u32
}

#[allow(non_snake_case)]
#[derive(Deserialize)]
struct Map {
    Title: String,
    Artist: String,
    DifficultyName: String,
    HitObjects: Vec<HitObject>,
    Mode: String
}

fn compute_etterna_difficulty(calc: &Calc, hit_objects: &Vec<HitObject>, n_key: u32, rate: f32) -> (f32, SkillsetScores) {
    // Transform quaver to etterna format
    let mut sums: HashMap<u32, u32> = HashMap::new();
    
    for it in hit_objects {
        *sums.entry(it.StartTime.unwrap_or(0)).or_insert(0) += u32::pow (2, it.Lane - 1);
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

// Find Qua file from mapid
fn find_qua_file(songs_path: &PathBuf, map_id: &str) -> Option<PathBuf> {
    let file_searched = format!("{map_id}.qua");
    return WalkDir::new(songs_path)
        .into_iter()
        .filter_map(Result::ok)
        .find_map(|entry| {
            if entry.file_type().is_file()
                && entry.file_name().to_str() == Some(&file_searched)
            {
                Some(entry.into_path())
            } else {
                None
            }
        });
}

fn parse_rate_from_mods(mods: &str) -> f32 {
    // Take the prefix before the first 'x' should be the rate
    let (rate_part, _) = mods
        .split_once('x')
        .unwrap_or(("1.0", ""));

    return rate_part.trim().parse::<f32>().unwrap_or(1.0);
}

fn print_score(map: &Map, rate: f32, score: &SkillsetScores) {
    println!("{} - {} - {} - {:.1}x →", map.Artist, map.Title, map.DifficultyName, rate);
    println!("overall: {:.2}", score.overall);
    println!("stream: {:.2}", score.stream);
    println!("jumpstream: {:.2}", score.jumpstream);
    println!("handstream: {:.2}", score.handstream);
    println!("stamina: {:.2}", score.stamina);
    println!("jackspeed: {:.2}", score.jackspeed);
    println!("chordjack: {:.2}", score.chordjack);
    println!("technical: {:.2}", score.technical);
}

// DO
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Start Ettena calc
    let calc = Calc::new().expect("Etterna Calc should launch");
    
    // Get input
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
    let quaver_installation_path = args.next().map(PathBuf::from).unwrap_or(quaver_installation_default);
    
    if !quaver_installation_path.exists() {
        eprintln!("Quaver installation could not be found, please give a correct path as argument of this program.");
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

    debouncer.watcher().watch(&now_playing_path, RecursiveMode::NonRecursive)?;

    // On file update
    for res in rx {
        match res {
            Ok(events) => {
                // Treat update on currently playing map
                if events.iter().any(|event| { return event.path.eq(&mapid_path) || event.path.eq(&mods_path) }) {
                    // Clear term
                    let _ = execute!(stdout(), Clear(ClearType::All));
                    let _ = execute!(stdout(), MoveTo(0, 0));
                        
                    let mut rate = 1.0;
                    let mut qua = None;
                    let mut map: Option<Map> = None;
                    match fs::read_to_string(&mods_path) {
                        Ok(mods) => {
                            rate = parse_rate_from_mods(&mods);
                        }
                        Err(e) => {
                            eprintln!("mods read error: {e}");
                            continue;
                        }
                    }
                    match fs::read_to_string(&mapid_path) {
                        Ok(mapid) => {
                            qua = find_qua_file(&songs_path, mapid.trim());
                            if qua.is_none() {
                                eprintln!("Could not find currently playing map");
                                continue;
                            }
                        }
                        Err(e) => {
                            eprintln!("mapid read error: {e}");
                            continue;
                        }
                    }

                    // Open Quaver file and parse it
                    match fs::read_to_string(qua.unwrap()) {
                        Ok(yaml_text) => {
                            match serde_yaml::from_str(&yaml_text) {
                                Ok(parsed_map) => {
                                    map = Some(parsed_map);
                                }
                                Err(e) => {
                                    eprintln!("Could not parse currently playing map {e}");
                                    continue;
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("qua read error: {e}");
                            continue;
                        }
                    }

                    let m = map.unwrap();

                    // Find keys of map
                    let n_key : u32 = m.Mode
                        .strip_prefix("Keys")
                        .and_then(|key| key.parse().ok())
                        .expect("missing 'Keys' prefix or could not parse number");

                    if n_key.lt(&2) {
                        eprintln!("Cannot do difficulty calculation on less than 2K");
                        continue;
                    }

                    let (rate, score) = compute_etterna_difficulty(&calc, &m.HitObjects, n_key, rate);
                    print_score(&m, rate, &score);
                }
            },
            Err(e) => println!("file watcher error: {:?}", e),
        }
    }

    Ok(())
}