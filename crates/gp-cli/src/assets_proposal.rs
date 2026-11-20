//! `gp assets proposal`: the person's answer, from a terminal, to a sprite
//! edit the agent proposed with `assets_edit` (issue #4), as `gp theme
//! sound` and `gp theme picture` are for sounds and pictures. The page's
//! card (mechanic/sprite-card.js, gp serve's `/api/sprites` routes) does
//! the same. The proposal stays in the copy's art history and the answer
//! is written there with the person as who decided
//! (gp_mcp::sprite_proposal). `gp_run` refuses accept, reject and undo,
//! and `assets_edit` refuses to accept or reject an open proposal: only
//! the person decides (gp_mcp::policy).

use anyhow::Result;
use clap::Subcommand;
use gp_evidence::Paths;
use gp_mcp::sprite_proposal::{self as store, Answer};

use crate::Outcome;

#[derive(Subcommand)]
pub enum ProposalCmd {
    /// The sprite edits the agent proposed for your copies (those under
    /// $GP_HOME/work/mcp/): open ones still waiting, then the ones you
    /// answered (accepted, rejected with your words, or put back), newest
    /// first. Example: gp assets proposal list
    List {
        /// Only this copy's.
        #[arg(long)]
        copy: Option<String>,
    },
    /// The person's ACCEPT, from a terminal: place the edit in the copy
    /// with every check. The page's ACCEPT does the same. Refused to an
    /// agent. Example: gp assets proposal accept my_hop-3
    Accept {
        /// The proposal's id, COPY-SEQ.
        id: String,
        /// Words for the agent, if any.
        #[arg(long)]
        words: Option<String>,
    },
    /// The person's REJECT, from a terminal: drop the edit; the agent
    /// hears your words. Refused to an agent. Example: gp assets proposal
    /// reject my_hop-3 --words "a green cap"
    Reject {
        /// The proposal's id, COPY-SEQ.
        id: String,
        /// What the agent should change.
        #[arg(long)]
        words: Option<String>,
    },
    /// The person's PUT THE OLD ONE BACK, from a terminal: undo an edit you
    /// accepted. Refused to an agent. Example: gp assets proposal undo
    /// my_hop-3
    Undo {
        /// The proposal's id, COPY-SEQ.
        id: String,
    },
}

/// The MCP tools' settings for the person's home, as gp serve's buttons
/// use them: copies under $GP_HOME/work/mcp/.
fn config(paths: &Paths) -> gp_mcp::Config {
    gp_mcp::Config {
        gp: std::env::current_exe().unwrap_or_else(|_| "gp".into()),
        repo: crate::bundle::root()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default(),
        work: store::copies_root(&paths.home),
        home: paths.home.clone(),
        rom: None,
        assets: None,
        games: None,
        worker_games: None,
    }
}

pub(crate) fn run(cmd: &ProposalCmd, paths: &Paths) -> Result<Outcome> {
    let decided = |id: &str, a: Answer, words: Option<&str>, done: &str| -> Result<Outcome> {
        let v = store::decide(&config(paths), id, a, words).map_err(anyhow::Error::msg)?;
        let text = format!(
            "{done}: the {} in {} (edit {}).",
            v["subject"].as_str().unwrap_or("sprite"),
            v["copy"].as_str().unwrap_or("your copy"),
            v["seq"]
        );
        Ok(Outcome::ok(v, text))
    };
    match cmd {
        ProposalCmd::List { copy } => {
            let (v, text) = store::list(&paths.home, copy.as_deref());
            Ok(Outcome::ok(v, text))
        }
        ProposalCmd::Accept { id, words } => decided(
            id,
            Answer::Accept,
            words.as_deref(),
            "Accepted, in your game now",
        ),
        ProposalCmd::Reject { id, words } => decided(
            id,
            Answer::Reject,
            words.as_deref(),
            "Rejected; your agent hears your words",
        ),
        ProposalCmd::Undo { id } => decided(id, Answer::Undo, None, "Put back as it was"),
    }
}
