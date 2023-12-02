mod commit;
mod repo;
mod state;

use clap::{Parser, Subcommand};
use repo::Repo;
use state::{Plan, State};
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

    /// Create a plan to create a ref.
    Plan {
        /// Plan name
        #[arg(short, long)]
        name: String,

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
    },

    /// Realise a plan
    Realise {
        /// Plans to realise
        #[arg()]
        names: Vec<String>,
    },
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

fn plan(
    state: State,
    name: String,
    base_ref: String,
    use_merge_base: bool,
    added_refs: Vec<String>,
    sign: bool,
) -> Result<(), Box<dyn Error>> {
    let plan = Plan {
        base_ref,
        use_merge_base,
        added_refs,
        sign,
    };

    state.save_plan(name, &plan)?;

    Ok(())
}

fn realise(state: State, plans: Vec<String>) -> Result<(), Box<dyn Error>> {
    if plans.is_empty() {
        for plan in state.all_plans()? {
            chain(
                state.repo(),
                plan.base_ref,
                plan.use_merge_base,
                plan.added_refs,
                plan.sign,
                None,
                None,
            )?;
        }
    } else {
        for plan in plans {
            let plan = state.find_plan(plan)?;
            chain(
                state.repo(),
                plan.base_ref,
                plan.use_merge_base,
                plan.added_refs,
                plan.sign,
                None,
                None,
            )?;
        }
    }

    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    let repo = Repo::discover(args.repo.as_str())?;
    let state = State::new(repo);

    match args.command {
        Cmd::Chain {
            base_ref,
            use_merge_base,
            added_refs,
            sign,
            update_ref,
            push,
        } => chain(
            state.repo(),
            base_ref,
            use_merge_base,
            added_refs,
            sign,
            update_ref,
            push,
        )?,

        Cmd::Plan {
            name,
            base_ref,
            use_merge_base,
            added_refs,
            sign,
        } => plan(state, name, base_ref, use_merge_base, added_refs, sign)?,

        Cmd::Realise { names: plans } => realise(state, plans)?,
    }

    Ok(())
}
