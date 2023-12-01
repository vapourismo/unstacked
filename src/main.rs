mod commit;
mod repo;

use crate::repo::Repo;
use clap::Parser;
use std::error::Error;

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg(long, default_value = ".")]
    repo: String,

    #[arg(short = 'b', long = "base")]
    base_ref: String,

    #[arg(short = 'r', long = "ref")]
    added_refs: Vec<String>,
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();

    let repo = Repo::discover(args.repo.as_str())?;
    let mut commit = repo.find_commit(args.base_ref)?;

    for new_ref in &args.added_refs {
        let new_commit = repo.find_commit(new_ref)?;
        commit = commit.cherry_pick(&repo, &new_commit, args.sign)?;
    }

    println!("{}", commit.id());

    Ok(())
}
