mod anchor;
mod commit;
mod diffs;
mod git_cache;
mod git_helper;
mod model;
mod path;
mod repo;
mod rules;
mod series;
mod state;

use crate::state::{MoveResult, State};
use clap::{Parser, Subcommand};
use diffs::PrettyDiff;
use git_cache::CachedRepo;
use model::Model;
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

    ///
    #[command(visible_alias = "go")]
    Goto {
        #[arg()]
        target: String,
    },

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
    #[command(visible_alias = "diff")]
    Staged {
        /// Onlys show changes in the index
        #[arg(short = 'i', long = "index")]
        use_index: bool,
    },

    /// Display the staged changes
    #[command(visible_alias = "i")]
    Info {},

    ///
    #[command()]
    NewSeries {
        #[arg()]
        name: String,

        #[arg(short, long)]
        parent: Option<String>,
    },

    ///
    #[command()]
    NewAnchor {
        #[arg()]
        name: String,

        #[arg(short = 'v', long)]
        initial_value: Option<String>,
    },

    ///
    Build {
        #[arg()]
        rules: Vec<String>,
    },

    ///
    BuildAll {},

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
        commit = commit.cherry_pick(repo, &new_commit, sign)?;
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
    env_logger::init();

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
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            model.goto_next(&mut repo)?;

            println!("{:?}", model.focus());
            model.save(repo.repo())?;
        }

        Cmd::Prev {} => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            model.goto_parent(&mut repo)?;

            println!("{:?}", model.focus());
            model.save(repo.repo())?;
        }

        Cmd::Goto { target } => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            model.goto_rule(&mut repo, &target)?;

            println!("{:?}", model.focus());
            model.save(repo.repo())?;
        }

        Cmd::Commit { msg, use_index } => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            let msg = {
                let diff = model.staged_diff(repo.repo(), use_index)?;
                git_helper::compose_commit_message(repo.repo(), msg, diff.as_ref())?
            };

            model.commit_onto_focus(&mut repo, msg, use_index, false)?;

            println!("{:?}", model.focus());
            model.save(repo.repo())?;
        }

        Cmd::Amend { use_index } => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            model.amend_focus(&mut repo, use_index)?;

            println!("{:?}", model.focus());
            model.save(repo.repo())?;
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
            let repo = CachedRepo::discover(args.repo)?;
            let model = Model::load(repo.repo())?;

            if let Some(diff) = model.staged_diff(repo.repo(), use_index)? {
                let pretty = PrettyDiff::new(&diff)?;
                println!("{pretty}");
            }

            // XXX: To make the borrow-checker happy
            let _ = repo;
        }

        Cmd::Info {} => {
            println!(
                "{}",
                MoveResult::stationary(mgr.repo().head_commit()?.as_ref())
            )
        }

        Cmd::NewSeries { name, parent } => {
            let repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            let rule = match parent {
                Some(name) => name,
                None => match model.focus_rule() {
                    Some(rule) => rule,
                    None => Err(git2::Error::new(git2::ErrorCode::Invalid, git2::ErrorClass::Invalid, "Model has no focus to act as parent, you need to specify a parent explicitly"))?,
                },
            };

            model.new_series(name.as_str(), rule);
            model.save(repo.repo())?;
        }

        Cmd::NewAnchor {
            name,
            initial_value: parent,
        } => {
            let repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            let id = match parent {
                Some(rev) => repo
                    .repo()
                    .revparse(rev.as_str())?
                    .from()
                    .ok_or_else(|| {
                        git2::Error::new(
                            git2::ErrorCode::Invalid,
                            git2::ErrorClass::Invalid,
                            format!("Revision {rev} does not resolve to a usable object"),
                        )
                    })?
                    .id(),
                None => repo.repo().head()?.peel_to_commit()?.id(),
            };

            model.new_anchor(name.as_str(), id);
            model.save(repo.repo())?;
        }

        Cmd::Build { rules } => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            for rule in rules {
                let id = model.build(&mut repo, rule)?;
                println!("{}", id);
            }
        }

        Cmd::BuildAll {} => {
            let mut repo = CachedRepo::discover(args.repo)?;
            let mut model = Model::load(repo.repo())?;

            let builds = model.build_all(&mut repo)?;
            for (name, id) in builds {
                println!("{name} => {id}");
            }
        }

        Cmd::Test {} => {
            let state = State::read(&mgr)?.validate(&mgr)?;
            state.write(&mgr)?;
            eprintln!("{state:#?}");
        }
    }

    Ok(())
}
