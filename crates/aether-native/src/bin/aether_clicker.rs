use enigo::{Enigo, MouseButton, MouseControllable};
use std::env;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: aether_clicker <x> <y>");
        std::process::exit(1);
    }

    let x: i32 = match args[1].parse() {
        Ok(val) => val,
        Err(_) => {
            eprintln!("Invalid X coordinate");
            std::process::exit(1);
        }
    };

    let y: i32 = match args[2].parse() {
        Ok(val) => val,
        Err(_) => {
            eprintln!("Invalid Y coordinate");
            std::process::exit(1);
        }
    };

    let mut enigo = Enigo::new();
    enigo.mouse_move_to(x, y);
    enigo.mouse_click(MouseButton::Left);

    println!(r#"{{"action": "click", "x": {}, "y": {}, "status": "success"}}"#, x, y);
}
