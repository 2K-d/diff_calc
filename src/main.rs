use std::fs;
use std::collections::HashMap;
use clap::Parser;
use serde::Deserialize;
use minacalc_rs::{Calc, CalcMode, Note};

// CLI MODEL
#[derive(Parser)]
#[command(name = "Diff-Calc")]
#[command(about = "Compute Etterna difficulty on Quaver map", long_about = None)]
struct Args {
    #[arg(short, long)]
    input: String,
    #[arg(short, long, default_value_t = 1.0)]
    rate: f32,
}

// QUAVER MODEL
#[derive(Deserialize)]
struct HitObject {
    StartTime: u32,
    Lane: u32
}

#[derive(Deserialize)]
struct Map {
    HitObjects: Vec<HitObject>
}

// DO
fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Get input
    let args = Args::parse();

    // Transform quaver to etterna format
    let mut sums: HashMap<u32, u32> = HashMap::new();
    let yaml_text = fs::read_to_string(args.input)?;
    let map: Map = serde_yaml::from_str(&yaml_text)?;
    for it in &map.HitObjects {
        *sums.entry(it.StartTime).or_insert(0) += u32::pow (2, it.Lane - 1);
    }
    let mut result: Vec<Note> = sums
        .into_iter()
        .map(|(start_time, lane)| Note { row_time: start_time as f32 / 1000.0, notes: lane })
        .collect();
    result.sort_by(|note_a, note_b| note_a.row_time.partial_cmp(&note_b.row_time).unwrap());
    
    // Do difficulty calculation
    let calc = Calc::new()?;
    let scores = calc
        .calc_all_rates(&result, 4, CalcMode::Msd)
        .unwrap();

    let score = scores.rates[((args.rate - 0.7) / 0.1) as usize];
    println!("{:.1}x →", args.rate);
    println!("overall: {:.2}", score.overall);
    println!("stream: {:.2}", score.stream);
    println!("jumpstream: {:.2}", score.jumpstream);
    println!("handstream: {:.2}", score.handstream);
    println!("stamina: {:.2}", score.stamina);
    println!("jackspeed: {:.2}", score.jackspeed);
    println!("chordjack: {:.2}", score.chordjack);
    println!("technical: {:.2}", score.technical);

    Ok(())
}
