mod commit;
mod repo;

use crate::repo::Repo;
use clap::Parser;
use std::error::Error;

#[derive(Parser, Debug)]
#[command()]
struct Args {
    /// Repository location
    #[arg(long, default_value = ".")]
    repo: String,

    /// Base commit
    #[arg(short, long = "base")]
    base_ref: String,

    /// Commits to be added on top of the base
    #[arg(short = 'r', long = "ref")]
    added_refs: Vec<String>,

    /// Sign the resulting commit?
    #[arg(short, long)]
    sign: bool,

    /// Update this reference
    #[arg(short, long)]
    update_ref: Option<String>,

    /// Push updated reference to this remote
    #[arg(short, long)]
    push: Option<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let repo = Repo::discover(args.repo.as_str())?;
    let mut commit = repo.find_commit(args.base_ref)?;

    for new_ref in &args.added_refs {
        let new_commit = repo.find_commit(new_ref)?;
        commit = commit.cherry_pick(&repo, &new_commit, args.sign)?;
    }

    if let Some(ref_) = args.update_ref {
        repo.update_reference(&ref_, commit.id())?;

        if let Some(remote_name) = args.push {
            repo.push(remote_name, &[format!("+{ref_}:refs/{ref_}").as_str()])?;
        }
    }

    println!("{}", commit.id());

    Ok(())
}
