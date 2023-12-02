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

    /// Use merge-base instead of base
    #[arg(short = 'm', long)]
    use_merge_base: bool,

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
    let add_commits = args
        .added_refs
        .into_iter()
        .map(|ref_| repo.find_commit(ref_))
        .collect::<Result<Vec<_>, _>>()?;

    if args.use_merge_base {
        let mut all_commits = Vec::with_capacity(add_commits.len() + 1);
        all_commits.push(commit.clone());
        all_commits.splice(1.., add_commits.iter().cloned());
        commit = repo.merge_base(&all_commits)?;
    }

    for new_commit in add_commits {
        commit = commit.cherry_pick(&repo, &new_commit, args.sign)?;
    }

    if let Some(ref_) = args.update_ref {
        repo.update_reference(&ref_, commit.id())?;

        if let Some(remote_name) = args.push {
            repo.push(remote_name, &[format!("+{ref_}").as_str()])?;
        }
    }

    println!("{}", commit.id());

    Ok(())
}
