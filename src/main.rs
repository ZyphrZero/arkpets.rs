use arkpets::{Bounds, CharacterState, ModelCatalog, PetConfig, Vec2};
use std::{env, process::ExitCode};

fn print_help() {
    println!("Ark-Pets Rust runtime\n\nUsage:\n  arkpets inspect-model <manifest.json>\n  arkpets simulate <manifest.json> [frames]\n\nThe runtime is renderer-agnostic; these commands exercise model loading and simulation.");
}

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    let Some(command) = args.next() else {
        print_help();
        return ExitCode::SUCCESS;
    };
    if matches!(command.as_str(), "--help" | "-h" | "help") {
        print_help();
        return ExitCode::SUCCESS;
    }
    let Some(path) = args.next() else {
        print_help();
        return ExitCode::from(2);
    };
    let catalog = match ModelCatalog::from_json_file(&path) {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("error: {error}");
            return ExitCode::from(1);
        }
    };
    match command.as_str() {
        "inspect-model" => {
            println!(
                "{}",
                serde_json::to_string_pretty(&catalog).expect("catalog is serializable")
            );
            ExitCode::SUCCESS
        }
        "simulate" => {
            let frames = match args.next() {
                None => 60,
                Some(value) => match value.parse::<usize>() {
                    Ok(frames) if (1..=1_000_000).contains(&frames) => frames,
                    _ => {
                        eprintln!("error: frames must be an integer between 1 and 1000000");
                        return ExitCode::from(2);
                    }
                },
            };
            let Some(model) = catalog.models.first().cloned() else {
                eprintln!("error: manifest contains no models");
                return ExitCode::from(1);
            };
            let mut character =
                CharacterState::new(model, PetConfig::default(), Vec2::new(150.0, 150.0), 42);
            for _ in 0..frames {
                character.tick(1.0 / 60.0, Bounds::new(1280.0, 720.0));
            }
            println!("{}", character.to_json().expect("snapshot is serializable"));
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("unknown command: {command}");
            print_help();
            ExitCode::from(2)
        }
    }
}
