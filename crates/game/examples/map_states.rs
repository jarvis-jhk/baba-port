//! Prints the icon and path states of a map as the game would show them
//! for one slot of a data directory (import cross-checks):
//!
//! ```sh
//! cargo run -p baba-port --example map_states -- DATA_DIR SLOT WORLD MAP
//! ```

use std::path::PathBuf;

use baba_game::world::WorldInfo;
use baba_progress::Profile;

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 4 {
        eprintln!("usage: map_states DATA_DIR SLOT WORLD MAP");
        std::process::exit(2);
    }
    let game = PathBuf::from(std::env::var("BABA_GAME_DIR").expect("BABA_GAME_DIR"));
    let data = game.join("Data");
    let profile = Profile::load(&PathBuf::from(&a[0]));
    let slot = &profile.slots[a[1].parse::<usize>().expect("slot")];
    let w = WorldInfo::open(&data, &a[2]).expect("world");
    let info = w.load(&a[3]).expect("map").info;
    let l = baba_game::play::map_layout(&a[3], &info, slot, &w.id);
    let mut open = 0;
    let mut done = 0;
    for i in &l.icons {
        println!(
            "icon {:>3},{:<3} {:<12} state {}",
            i.x, i.y, i.target, i.state
        );
        open += (i.state == 2) as usize;
        done += (i.state >= 3) as usize;
    }
    let paths = l.paths.iter().filter(|p| p.state > 0).count();
    println!(
        "{} icons: {done} completed, {open} open; {paths}/{} paths shown",
        l.icons.len(),
        l.paths.len()
    );
}
