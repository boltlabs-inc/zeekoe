use std::path::PathBuf;
use structopt::StructOpt;

use zkabacus_crypto::ChannelId;

pub use crate::merchant;

/// The merchant zkChannels command-line interface.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Cli {
    /// Path to a configuration file.
    #[structopt(long)]
    pub config: Option<PathBuf>,

    /// Run merchant commands.
    #[structopt(subcommand)]
    pub merchant: Merchant,
}

#[derive(Debug, StructOpt)]
pub enum Merchant {
    List(List),
    Show(Show),
    Configure(Configure),
    Run(Run),
    Close(Close),
}

/// List all the zkChannels you've established with customers.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct List {
    /// Get machine-readable json output. In particular, currencies are expressed in minor units,
    /// not the standard human representation.
    #[structopt(long)]
    pub json: bool,
}

/// Show details for a single zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Show {
    #[structopt(empty_values(false))]
    pub prefix: String,

    /// Get machine-readable json output. In particular, currencies are expressed in minor units,
    /// not the standard human representation.
    #[structopt(long)]
    pub json: bool,
}

/// Edit the configuration in a text editor.
///
/// This will use the `VISUAL` or `EDITOR` environment variables if they are set.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Configure {}

/// Run the merchant server.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Run {}

/// Close an existing zkChannel.
#[derive(Debug, StructOpt)]
#[non_exhaustive]
pub struct Close {
    /// Close all zkChannels that haven't already been closed. Incompatible with `--channel`
    #[structopt(long, conflicts_with = "channel")]
    pub all: bool,

    /// Close a single zkChannel by ID. Incompatible with `--all`.
    #[structopt(long, required_unless = "all")]
    pub channel: Option<ChannelId>,
}
