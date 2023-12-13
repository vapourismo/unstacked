mod commit;
mod repo;
mod state;

use clap::{Parser, Subcommand};
use repo::Repo;
use state::Manager;
use std::{env, error::Error, fs, process};

use crate::state::State;

#[derive(Parser, Debug)]
#[command()]
struct Args {
    /// Repository location
    #[arg(long, default_value = ".")]
    repo: String,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Construct a chain of commits from the given base
    Chain {
        /// Base commit
        #[arg(short, long = "base")]
        base_ref: String,

        /// Use merge-base instead of base
        #[arg(short = 'm', long)]
        use_merge_base: bool,

        /// Commits to be added on top of the base
        #[arg()]
        added_refs: Vec<String>,

        /// Sign the resulting commit
        #[arg(short, long)]
        sign: bool,

        /// Update this reference
        #[arg(short, long)]
        update_ref: Option<String>,

        /// Push updated reference to this remote
        #[arg(short, long)]
        push: Option<String>,
    },

    ///
    Next {},

    ///
    Prev {},

    /// Produce a new commit with the staged changes
    Commit {
        /// Commit message
        #[arg(short, long)]
        msg: Option<String>,
    },

    /// Incorporate the staged changes into the active commit
    Amend {},

    ///
    Test {},
}

fn chain(
    repo: &Repo,
    base_ref: String,
    use_merge_base: bool,
    added_refs: Vec<String>,
    sign: bool,
    update_ref: Option<String>,
    push: Option<String>,
) -> Result<(), Box<dyn Error>> {
    let mut commit = repo.find_commit(base_ref)?;
    let num_refs = added_refs.len();
    let add_commits = added_refs
        .into_iter()
        .map(|ref_| repo.find_commit(ref_))
        .collect::<Result<Vec<_>, _>>()?;

    if use_merge_base && num_refs > 0 {
        let mut all_commits = Vec::with_capacity(add_commits.len() + 1);
        all_commits.push(commit.clone());
        all_commits.splice(1.., add_commits.iter().cloned());
        commit = repo.merge_base(&all_commits)?;
    }

    for new_commit in add_commits {
        commit = commit.cherry_pick(&repo, &new_commit, sign)?;
    }

    if let Some(ref_) = update_ref {
        repo.update_reference(&ref_, commit.id())?;

        if let Some(remote_name) = push {
            repo.push(remote_name, &[format!("+{ref_}").as_str()])?;
        }
    }

    println!("{}", commit.id());

    Ok(())
}

fn compose_message(msg: Option<String>) -> Result<String, Box<dyn Error>> {
    let editor = env::var("EDITOR").expect("Need $EDITOR set when omitting commit message");

    let msg_file = {
        let mut path = env::temp_dir();
        path.push("UNSTACKED_MSG");
        path
    };

    fs::write(&msg_file, msg.unwrap_or("".to_string()))?;

    let exit = process::Command::new(editor)
        .arg(&msg_file)
        .spawn()?
        .wait()?;

    assert!(exit.success());

    let msg = fs::read(&msg_file)?;
    fs::remove_file(&msg_file)?;

    let msg = git2::message_prettify(String::from_utf8(msg)?, Some('#'.try_into().unwrap()))?;

    Ok(msg)
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let repo = Repo::discover(args.repo.as_str())?;
    let mgr = Manager::new(repo);

    match args.command {
        Cmd::Chain {
            base_ref,
            use_merge_base,
            added_refs,
            sign,
            update_ref,
            push,
        } => chain(
            mgr.repo(),
            base_ref,
            use_merge_base,
            added_refs,
            sign,
            update_ref,
            push,
        )?,

        Cmd::Next {} => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let moved = state.next(&mgr)?;
            eprintln!("{moved}");
        }

        Cmd::Prev {} => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let moved = state.prev(&mgr)?;
            eprintln!("{moved}");
        }

        Cmd::Commit { msg } => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;

            let msg = match msg {
                Some(msg) => msg,
                None => compose_message(None)?,
            };
            let msg = git2::message_prettify(msg, Some('#'.try_into().unwrap()))?;

            let moved = state.commit(&mgr, msg)?;
            eprintln!("{moved}");
        }

        Cmd::Amend {} => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let moved = state.amend(&mgr)?;
            eprintln!("{moved}");
        }

        Cmd::Test {} => {
            let state = State::read(&mgr)?.validate(&mgr)?;
            state.write(&mgr)?;
            eprintln!("{state:#?}");
        }
    }

    Ok(())
}
