mod commit;
mod db;
mod diffs;
mod repo;
mod state;

use crate::state::{MoveResult, State};
use clap::{Parser, Subcommand};
use db::Store;
use diffs::PrettyDiff;
use repo::Repo;
use state::Manager;
use std::error::Error;

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

    /// Move to next commit
    #[command(visible_alias = "n")]
    Next {},

    /// Move to previous commit
    #[command(visible_alias = "p")]
    Prev {},

    /// Produce a new commit with the staged changes
    #[command(visible_alias = "co")]
    Commit {
        /// Commit message
        #[arg(short, long)]
        msg: Option<String>,

        /// Only commit changes in the index
        #[arg(short = 'i', long = "index")]
        use_index: bool,
    },

    /// Incorporate the staged changes into the active commit
    #[command(visible_alias = "am")]
    Amend {
        /// Only amend with changes in the index
        #[arg(short = 'i', long = "index")]
        use_index: bool,
    },

    /// Edit commit meta data
    #[command(visible_alias = "ed")]
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

    /// Edit commit message
    #[command(visible_alias = "em")]
    EditMessage {},

    /// Display the staged changes
    #[command()]
    Staged {
        /// Onlys show changes in the index
        #[arg(short = 'i', long = "index")]
        use_index: bool,
    },

    /// Display the staged changes
    #[command(visible_alias = "i")]
    Info {},

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
            let result = state.next(&mgr)?;
            eprintln!("{result}");
        }

        Cmd::Prev {} => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let result = state.prev(&mgr)?;
            eprintln!("{result}");
        }

        Cmd::Commit { msg, use_index } => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let result = state.commit(&mgr, msg, use_index)?;
            eprintln!("{result}");
        }

        Cmd::Amend { use_index } => {
            let mut state = State::read(&mgr)?.validate(&mgr)?;
            let result = state.amend(&mgr, use_index)?;
            eprintln!("{result}");
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

            let result = mgr.edit(&info)?;
            eprintln!("{result}");
        }

        Cmd::EditMessage {} => {
            let mut info = mgr.commit_info()?;
            info.message = mgr.compose_commit_message(Some(info.message), None)?;

            let result = mgr.edit(&info)?;
            eprintln!("{result}");
        }

        Cmd::Staged { use_index } => {
            let tree = mgr.capture_tree(use_index)?;
            let diff = mgr.repo().diff_tree_to_tree(
                Some(&mgr.repo().head_commit()?.tree()?),
                Some(&tree),
                None,
            )?;

            let pretty = PrettyDiff::new(&diff)?;
            println!("{pretty}");
        }

        Cmd::Info {} => {
            println!(
                "{}",
                MoveResult::stationary(mgr.repo().head_commit()?.as_ref())
            )
        }

        Cmd::Test {} => {
            let state = State::read(&mgr)?.validate(&mgr)?;
            state.write(&mgr)?;
            eprintln!("{state:#?}");

            let mut kv = Store::open(mgr.repo())?;

            let value: String = kv.get(["foo", "bar"])?;
            kv.put(
                ["foo", "bar"],
                &value.chars().into_iter().rev().collect::<String>(),
            )?;
            kv.put(["qux"], &"Stored".to_string())?;

            kv.write()?;
        }
    }

    Ok(())
}
