use std::{io::{stdout, Write}, collections::HashMap, fs, path::{Path, PathBuf}, time::Duration};
use clap::Parser;
use serde::Deserialize;
use minacalc_rs::{Calc, CalcMode, Note};
use notify::{RecursiveMode};
use notify_debouncer_mini::{new_debouncer_opt, Config};
use walkdir::WalkDir;
use crossterm::{
    execute,
    terminal::{Clear, ClearType},
    cursor::MoveTo
};

// CLI MODEL
#[derive(Parser)]
#[command(name = "Diff-Calc", version = clap::crate_version!(), about = "Compute Etterna difficulty on Quaver map", long_about = None)]
struct Args {
    #[arg(help = "path to a quaver installation")]
    quaver_installation: String
}

// QUAVER MODEL
#[derive(Deserialize)]
struct HitObject {
    StartTime: u32,
    Lane: u32
}

#[derive(Deserialize)]
struct Map {
    Title: String,
    Artist: String,
    DifficultyName: String,
    HitObjects: Vec<HitObject>,
}

// compute etterna diff
fn compute_difficulty(qua_path: PathBuf, rate: f32) {
    let yaml_text = fs::read_to_string(qua_path).unwrap();
    let map: Map = serde_yaml::from_str(&yaml_text).unwrap();

    // Transform quaver to etterna format
    let mut sums: HashMap<u32, u32> = HashMap::new();
    
    for it in &map.HitObjects {
        *sums.entry(it.StartTime).or_insert(0) += u32::pow (2, it.Lane - 1);
    }
    let mut result: Vec<Note> = sums
        .into_iter()
        .map(|(start_time, lane)| Note { row_time: start_time as f32 / 1000.0, notes: lane })
        .collect();
    result.sort_by(|note_a, note_b| note_a.row_time.partial_cmp(&note_b.row_time).unwrap());
    
    // Do difficulty calculation
    let calc = Calc::new().expect("expected Calc to launch");
    let scores = calc
        .calc_all_rates(&result, 4, CalcMode::Msd)
        .unwrap();

    // Clear term
    let _ = execute!(stdout(), Clear(ClearType::All));
    let _ = execute!(stdout(), MoveTo(0, 0));
    
    // Print result
    let score = scores.rates[((rate - 0.7) / 0.1) as usize];
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

// DO
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get input
    let args = Args::parse();

    let quaver_installation = Path::new(&args.quaver_installation);
    let now_playing_path = quaver_installation.join("Data").join("Temp").join("Now Playing");
    let mapid_path = now_playing_path.join("mapid.txt");
    let mods_path = now_playing_path.join("mods.txt");
    let songs_path = quaver_installation.join("Songs");

    let (tx, rx) = std::sync::mpsc::channel();

    let backend_config = notify::Config::default()
        .with_poll_interval(Duration::from_millis(1000));
    // debouncer configuration
    let debouncer_config = Config::default()
        .with_timeout(Duration::from_millis(1000))
        .with_notify_config(backend_config);

    let mut debouncer = new_debouncer_opt::<_, notify::PollWatcher>(debouncer_config, tx).unwrap();

    debouncer.watcher().watch(&now_playing_path, RecursiveMode::NonRecursive)?;

    for res in rx {
        match res {
            Ok(events) => {
                if events.iter().any(|event| { return event.path.eq(&mapid_path) || event.path.eq(&mods_path) }) {
                    let mapid = fs::read_to_string(&mapid_path)?;
                    let mods = fs::read_to_string(&mods_path)?;
                    let rate = parse_rate_from_mods(&mods);
                    let qua = find_qua_file(&songs_path, mapid.trim());
                    if qua.is_some() {
                        compute_difficulty(qua.unwrap(), rate);
                    }
                }
            },
            Err(e) => println!("watcher error: {:?}", e),
        }
    }

    Ok(())
}
