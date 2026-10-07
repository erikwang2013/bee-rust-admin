// Copyright (c) 2026 erik <erik@erik.xyz> — https://erik.xyz
use std::path::Path;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "bee-rust", about = "bee-rust framework CLI tool")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Create a new bee-rust project
    New { name: String },
    /// Generate a controller or model
    Generate {
        #[command(subcommand)]
        kind: GenerateKind,
    },
    /// Run the development server
    Run {
        #[arg(long)]
        watch: bool,
    },
    /// Scaffold and run the database migration binary
    Migrate {
        #[command(subcommand)]
        action: MigrateAction,
    },
    /// Package for deployment
    Pack {
        #[arg(long, default_value = "linux/x86_64")]
        target: String,
    },
    /// Show the project pet bee (mood follows project state)
    Pet,
}

#[derive(Subcommand)]
enum GenerateKind {
    Controller {
        name: String,
    },
    Model {
        name: String,
        #[arg(long)]
        fields: Option<String>,
    },
}

#[derive(Subcommand)]
enum MigrateAction {
    /// Scaffold src/bin/bee_migrate.rs in the current crate
    Init,
    /// Run the scaffolded src/bin/bee_migrate.rs
    Run,
}

fn main() {
    let cli = Cli::parse();
    let result = match cli.command {
        Commands::New { name } => bee_cli::new_project(&name),
        Commands::Generate { kind } => match kind {
            GenerateKind::Controller { name } => bee_cli::generate_controller(&name),
            GenerateKind::Model { name, fields } => {
                bee_cli::generate_model(&name, fields.as_deref())
            }
        },
        Commands::Run { watch } => bee_cli::run_server(watch),
        Commands::Migrate { action } => match action {
            MigrateAction::Init => bee_cli::migrate_init(Path::new(".")),
            // cargo's own exit code, passed through to the caller's shell.
            MigrateAction::Run => bee_cli::migrate_run(Path::new(".")).map(|code| {
                if code != 0 {
                    std::process::exit(code);
                }
            }),
        },
        Commands::Pack { target } => bee_cli::pack(&target),
        Commands::Pet => bee_cli::pet(),
    };
    if let Err(message) = result {
        eprintln!("error: {message}");
        std::process::exit(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn test_new_command() {
        let cli = Cli::try_parse_from(["bee-rust", "new", "myapp"]).unwrap();
        match cli.command {
            Commands::New { name } => assert_eq!(name, "myapp"),
            _ => panic!("expected New command"),
        }
    }

    #[test]
    fn test_generate_controller() {
        let cli = Cli::try_parse_from(["bee-rust", "generate", "controller", "users"]).unwrap();
        match cli.command {
            Commands::Generate { kind } => match kind {
                GenerateKind::Controller { name } => assert_eq!(name, "users"),
                _ => panic!("expected Controller"),
            },
            _ => panic!("expected Generate command"),
        }
    }

    #[test]
    fn test_generate_model() {
        let cli = Cli::try_parse_from([
            "bee-rust",
            "generate",
            "model",
            "post",
            "--fields",
            "title:string,body:text",
        ])
        .unwrap();
        match cli.command {
            Commands::Generate { kind } => match kind {
                GenerateKind::Model { name, fields } => {
                    assert_eq!(name, "post");
                    assert_eq!(fields, Some("title:string,body:text".into()));
                }
                _ => panic!("expected Model"),
            },
            _ => panic!("expected Generate command"),
        }
    }

    #[test]
    fn test_run_command() {
        let cli = Cli::try_parse_from(["bee-rust", "run"]).unwrap();
        match cli.command {
            Commands::Run { watch } => assert!(!watch),
            _ => panic!("expected Run command"),
        }
    }

    #[test]
    fn test_run_watch() {
        let cli = Cli::try_parse_from(["bee-rust", "run", "--watch"]).unwrap();
        match cli.command {
            Commands::Run { watch } => assert!(watch),
            _ => panic!("expected Run command"),
        }
    }

    #[test]
    fn test_migrate_init() {
        let cli = Cli::try_parse_from(["bee-rust", "migrate", "init"]).unwrap();
        match cli.command {
            Commands::Migrate { action } => match action {
                MigrateAction::Init => {}
                _ => panic!("expected Init"),
            },
            _ => panic!("expected Migrate command"),
        }
    }

    #[test]
    fn test_migrate_run() {
        let cli = Cli::try_parse_from(["bee-rust", "migrate", "run"]).unwrap();
        match cli.command {
            Commands::Migrate { action } => match action {
                MigrateAction::Run => {}
                _ => panic!("expected Run"),
            },
            _ => panic!("expected Migrate command"),
        }
    }

    #[test]
    fn test_migrate_rejects_the_old_directions() {
        for bad in ["up", "down"] {
            assert!(Cli::try_parse_from(["bee-rust", "migrate", bad]).is_err(), "{bad}");
        }
    }

    #[test]
    fn test_pack_default_target() {
        let cli = Cli::try_parse_from(["bee-rust", "pack"]).unwrap();
        match cli.command {
            Commands::Pack { target } => assert_eq!(target, "linux/x86_64"),
            _ => panic!("expected Pack command"),
        }
    }

    #[test]
    fn test_pack_custom_target() {
        let cli = Cli::try_parse_from(["bee-rust", "pack", "--target", "linux/aarch64"]).unwrap();
        match cli.command {
            Commands::Pack { target } => assert_eq!(target, "linux/aarch64"),
            _ => panic!("expected Pack command"),
        }
    }

    #[test]
    fn test_pet_command() {
        let cli = Cli::try_parse_from(["bee-rust", "pet"]).unwrap();
        match cli.command {
            Commands::Pet => {}
            _ => panic!("expected Pet command"),
        }
    }
}
