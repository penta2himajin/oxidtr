use clap::{Parser, Subcommand, ValueEnum};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::check::{self, CheckConfig};

#[derive(Parser)]
#[command(name = "oxidtr")]
#[command(about = "Generate type-safe code and tests from Alloy models")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Generate types, operation stubs, and test skeletons from an Alloy model
    Generate {
        /// Path to the Alloy model file (.als)
        model: String,
        #[arg(long, default_value = "rust")]
        target: String,
        #[arg(short, long, default_value = "generated")]
        output: String,
        #[arg(long, default_value = "warn")]
        warnings: WarningArg,
    },
    /// Check structural consistency between Alloy model and Rust implementation
    Check {
        /// Path to the Alloy model file (.als)
        #[arg(long)]
        model: String,
        /// Path to the implementation directory (must contain models.rs)
        #[arg(long)]
        r#impl: String,
    },
}

#[derive(Clone, ValueEnum)]
enum WarningArg {
    Error,
    Warn,
    Off,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { model, target, output, warnings } => {
            let config = GenerateConfig {
                target,
                output_dir: output,
                warnings: match warnings {
                    WarningArg::Error => WarningLevel::Error,
                    WarningArg::Warn  => WarningLevel::Warn,
                    WarningArg::Off   => WarningLevel::Off,
                },
            };
            match generate::run(&model, &config) {
                Ok(result) => {
                    println!("Generated:");
                    for path in &result.files_written {
                        println!("  {path}");
                    }
                    if !result.warnings.is_empty() {
                        println!("\n{} warning(s)", result.warnings.len());
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }

        Commands::Check { model, r#impl } => {
            let config = CheckConfig { impl_dir: r#impl };
            match check::run(&model, &config) {
                Ok(result) => {
                    if result.is_ok() {
                        println!("ok: model and implementation are in sync");
                    } else {
                        println!("{} diff(s) found:", result.diffs.len());
                        for d in &result.diffs {
                            println!("  {d}");
                        }
                        std::process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
    }
}
