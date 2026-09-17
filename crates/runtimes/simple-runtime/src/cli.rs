use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};

/// A CLI tool to translate images.
#[derive(Debug, Parser)]
#[command(author, version, about, long_about = None)]
pub struct Cli {
    /// Verbose mode (-v, -vv, -vvv)
    #[arg(short, long, global = true, action = ArgAction::Count)]
    pub verbose: u8,

    /// maximum batch size for ocr
    #[arg(long, global = true, default_value_t = 16)]
    pub max_batch_size_ocr: usize,

    /// maximum batch size for upscaler
    #[arg(long, global = true, default_value_t = 2)]
    pub max_batch_size_upscaler: usize,

    /// Choose a subcommand (cli, daemon, or ui)
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Run the image translation CLI
    Cli {
        /// Input file or directory
        #[arg(short, long)]
        input: PathBuf,

        /// Output directory
        #[arg(short, long)]
        output: PathBuf,

        /// Optional config file
        #[arg(short, long)]
        config: Option<PathBuf>,
        /// Overwrite already translated images
        #[arg(long)]
        overwrite: bool,
        /// Write refined mask next to the output as *.mask.png
        #[arg(long)]
        save_mask: bool,
    },

    /// Run the Daemon
    Daemon {
        /// Host to bind the Daemon
        #[arg(long, default_value = "0.0.0.0")]
        host: String,

        /// Port to bind the Daemon
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },

    /// Run the UI
    Ui,
}
