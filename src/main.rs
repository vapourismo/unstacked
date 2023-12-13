mod commit;
mod diffs;
mod repo;
mod state;

use clap::{Parser, Subcommand};
use repo::Repo;
use state::Manager;
use std::error::Error;

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
    #[command(alias = "n")]
    Next {},

    ///
    #[command(alias = "p")]
    Prev {},

    /// Produce a new commit with the staged changes
    #[command(alias = "co")]
    Commit {
        /// Commit message
        #[arg(short, long)]
        msg: Option<String>,
    },

    /// Incorporate the staged changes into the active commit
    #[command(alias = "am")]
    Amend {},

    ///
    #[command(alias = "ed")]
    Edit {
        #[arg(long = "author-name")]
        author_name: Option<String>,

        #[arg(long = "author-email")]
        author_email: Option<String>,

        #[arg(long = "committer-name")]
        committer_name: Option<String>,

        #[arg(long = "committer-email")]
        committer_email: Option<String>,

        #[arg(short, long)]
        message: Option<String>,
    },

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
                None => {
                    let diff = mgr.repo().0.diff_tree_to_index(
                        Some(&mgr.repo().head_commit()?.tree()?),
                        Some(&mgr.repo().0.index()?),
                        None,
                    )?;
                    mgr.compose_commit_message(None, Some(&diff))?
                }
            };

            let moved = state.commit(&mgr, msg)?;
            eprintln!("{moved}");
        }

        Cmd::Amend {} => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let moved = state.amend(&mgr)?;
            eprintln!("{moved}");
        }

        Cmd::Edit {
            author_name,
            author_email,
            committer_name,
            committer_email,
            message,
        } => {
            let mut info = mgr.commit_info()?;

            let need_edit = author_name.is_none()
                && author_email.is_none()
                && committer_name.is_none()
                && committer_email.is_none()
                && message.is_none();

            if let Some(author_name) = author_name {
                info.author.name = author_name;
            }

            if let Some(author_email) = author_email {
                info.author.email = author_email;
            }

            if let Some(committer_name) = committer_name {
                info.committer.name = committer_name;
            }

            if let Some(committer_email) = committer_email {
                info.committer.email = committer_email;
            }

            if let Some(message) = message {
                info.message = message;
            }

            if need_edit {
                let info_rendered = serde_json::ser::to_string_pretty(&info)?;
                let info_edited =
                    mgr.compose_message_plain(&mgr.commit_info_file(), info_rendered)?;
                info = serde_json::de::from_str(info_edited.as_str())?;
            }

            let moved = mgr.edit(&info)?;
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
