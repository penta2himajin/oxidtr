use clap::{Parser, Subcommand, ValueEnum};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::check::{self, CheckConfig};
use oxidtr::mine;

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
        /// Comma-separated feature flags (e.g., serde)
        #[arg(long, value_delimiter = ',')]
        features: Vec<String>,
    },
    /// Check structural consistency between Alloy model and implementation
    Check {
        /// Path to the Alloy model file (.als)
        #[arg(long)]
        model: String,
        /// Path to the implementation directory
        #[arg(long)]
        r#impl: String,
    },
    /// Extract Alloy model draft from existing source code
    Mine {
        /// Path to source file or directory
        source: String,
        /// Source language (auto-detected from extension if omitted)
        #[arg(long)]
        lang: Option<String>,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
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
        Commands::Generate { model, target, output, warnings, features } => {
            let config = GenerateConfig {
                target,
                output_dir: output,
                warnings: match warnings {
                    WarningArg::Error => WarningLevel::Error,
                    WarningArg::Warn  => WarningLevel::Warn,
                    WarningArg::Off   => WarningLevel::Off,
                },
                features,
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

        Commands::Mine { source, lang, output } => {
            let mined = match mine::run(&source, lang.as_deref()) {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };

            let rendered = mine::renderer::render(&mined);

            if let Some(path) = output {
                if let Err(e) = std::fs::write(&path, &rendered) {
                    eprintln!("error: cannot write {path}: {e}");
                    std::process::exit(1);
                }
                println!("Mined {} sig(s), {} fact candidate(s) → {path}",
                    mined.sigs.len(), mined.fact_candidates.len());
            } else {
                print!("{rendered}");
                eprintln!("\nMined {} sig(s), {} fact candidate(s)",
                    mined.sigs.len(), mined.fact_candidates.len());
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
