use clap::Parser;
use git2::{Commit, Repository, Signature, Tree};
use gpgme::Context;
use std::error::Error;

fn commit_signed<'a>(
    repo: &'a Repository,
    author: &Signature,
    committer: &Signature,
    message: impl AsRef<str>,
    tree: &Tree,
    parent: &Commit,
) -> Result<Commit<'a>, Box<dyn Error>> {
    let commit_buffer =
        repo.commit_create_buffer(author, committer, message.as_ref(), tree, &[parent])?;
    let commit_buffer_str = commit_buffer
        .as_str()
        .ok_or_else(|| -> Box<dyn Error> { "Empty commit buffer string".into() })?;

    let signature = {
        let mut ctx = Context::from_protocol(gpgme::Protocol::OpenPgp)?;
        ctx.set_armor(true);
        let mut sig_out = Vec::new();
        ctx.sign(gpgme::SignMode::Detached, commit_buffer_str, &mut sig_out)?;
        std::str::from_utf8(&sig_out)?.to_string()
    };

    let new_commit_oid = repo.commit_signed(commit_buffer_str, &signature, None)?;
    let new_commit = repo.find_commit(new_commit_oid)?;

    Ok(new_commit)
}

fn chain_commit<'a>(
    repo: &'a Repository,
    parent: &Commit<'a>,
    commit: &Commit<'a>,
) -> Result<Commit<'a>, Box<dyn Error>> {
    assert_eq!(commit.parent_count(), 1);

    let changes = repo.diff_tree_to_tree(
        Some(&commit.parent(0)?.tree()?),
        Some(&commit.tree()?),
        None,
    )?;

    let new_tree = repo.find_tree(
        repo.apply_to_tree(&parent.tree()?, &changes, None)?
            .write_tree_to(repo)?,
    )?;

    let new_commit = commit_signed(
        repo,
        &commit.author(),
        &commit.committer(),
        commit.message().unwrap_or(""),
        &new_tree,
        parent,
    )?;

    Ok(new_commit)
}

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

    let repo = Repository::discover(args.repo.as_str())?;

    let mut commit = repo
        .revparse(args.base_ref.as_str())?
        .from()
        .unwrap()
        .peel_to_commit()?;

    for new_ref in &args.added_refs {
        let new_commit = repo
            .revparse(new_ref.as_str())?
            .from()
            .unwrap()
            .peel_to_commit()?;

        commit = chain_commit(&repo, &commit, &new_commit)?;
    }

    println!("{}", commit.id());

    Ok(())
}
