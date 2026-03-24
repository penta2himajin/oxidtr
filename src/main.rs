use clap::{Parser, Subcommand, ValueEnum};
use oxidtr::generate::{self, GenerateConfig, WarningLevel};
use oxidtr::check::{self, CheckConfig};
use oxidtr::extract;

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
        /// Force JSON Schema generation (default: auto per language — on for TS/Java, off for Rust/Kotlin)
        #[arg(long)]
        schema: Option<bool>,
        /// Test runner for TypeScript target (bun or vitest, default: bun)
        #[arg(long, default_value = "bun")]
        test_runner: TestRunnerArg,
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
    Extract {
        /// Path to source file or directory
        source: String,
        /// Source language (auto-detected; omit for multi-lang merge)
        #[arg(long)]
        lang: Option<String>,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
        /// Conflict handling when merging multi-lang sources (warn or error)
        #[arg(long, default_value = "warn")]
        conflict: ConflictArg,
    },
}

#[derive(Clone, ValueEnum)]
enum WarningArg {
    Error,
    Warn,
    Off,
}

#[derive(Clone, ValueEnum)]
enum TestRunnerArg {
    Bun,
    Vitest,
}

#[derive(Clone, ValueEnum)]
enum ConflictArg {
    Warn,
    Error,
}

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Commands::Generate { model, target, output, warnings, features, schema, test_runner } => {
            use oxidtr::backend::typescript::TsTestRunner;
            let config = GenerateConfig {
                target,
                output_dir: output,
                warnings: match warnings {
                    WarningArg::Error => WarningLevel::Error,
                    WarningArg::Warn  => WarningLevel::Warn,
                    WarningArg::Off   => WarningLevel::Off,
                },
                features,
                schema,
                ts_test_runner: match test_runner {
                    TestRunnerArg::Bun => TsTestRunner::Bun,
                    TestRunnerArg::Vitest => TsTestRunner::Vitest,
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

        Commands::Extract { source, lang, output, conflict } => {
            let result = match extract::run_merge(&source, lang.as_deref()) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };

            // Report conflicts
            if !result.conflicts.is_empty() {
                for c in &result.conflicts {
                    eprintln!("[CONFLICT] {}.{}: {} ({})",
                        c.sig_name, c.field_name, c.description,
                        c.sources.join(" vs "));
                }
                if matches!(conflict, ConflictArg::Error) {
                    eprintln!("error: {} conflict(s) found", result.conflicts.len());
                    std::process::exit(1);
                }
            }

            let rendered = extract::renderer::render(&result.model);

            if let Some(path) = output {
                if let Err(e) = std::fs::write(&path, &rendered) {
                    eprintln!("error: cannot write {path}: {e}");
                    std::process::exit(1);
                }
                println!("Mined {} sig(s), {} fact candidate(s) → {path}",
                    result.model.sigs.len(), result.model.fact_candidates.len());
            } else {
                print!("{rendered}");
                eprintln!("\nMined {} sig(s), {} fact candidate(s)",
                    result.model.sigs.len(), result.model.fact_candidates.len());
            }

            // Report sources
            if result.sources_used.len() > 1 {
                eprintln!("Sources: {}", result.sources_used.join(", "));
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
