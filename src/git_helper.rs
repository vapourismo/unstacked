use crate::diffs;
use git2::{Commit, IndexConflict, Oid, Repository, ResetType, Tree};
use std::{env, fmt, fs, process};

#[derive(derive_more::Display, derive_more::Error)]
#[display(fmt = "Error while applying {cherry} onto {target}")]
pub struct CherryPickConflict {
    pub target: Oid,
    pub cherry: Oid,
    pub conflicts: Vec<IndexConflict>,
}

impl fmt::Debug for CherryPickConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct CherryPickConflict {
            target: Oid,
            cherry: Oid,
        }

        CherryPickConflict::fmt(
            &CherryPickConflict {
                target: self.target,
                cherry: self.cherry,
            },
            f,
        )
    }
}

#[derive(Debug, derive_more::Error, derive_more::Display, derive_more::From)]
pub enum Error {
    GitError(git2::Error),
    CherryPickConflict(CherryPickConflict),
}

fn dot_git_child(repo: &Repository, name: impl AsRef<std::path::Path>) -> std::path::PathBuf {
    let mut path: std::path::PathBuf = repo.path().into();
    path.push(name);
    path
}

fn commit_message_file(repo: &Repository) -> std::path::PathBuf {
    dot_git_child(repo, "COMMIT_EDITMSG")
}

pub fn compose_message_plain(
    msg_file: &std::path::PathBuf,
    body: String,
) -> Result<String, Box<dyn std::error::Error>> {
    let editor = env::var("EDITOR").expect("Need $EDITOR set when omitting commit message");

    fs::write(msg_file, body)?;

    let exit = process::Command::new(editor)
        .arg(msg_file)
        .spawn()?
        .wait()?;

    assert!(exit.success());

    let msg = fs::read(msg_file)?;
    let msg = String::from_utf8(msg)?;
    fs::remove_file(msg_file)?;

    Ok(msg)
}

pub fn compose_commit_message(
    repo: &Repository,
    headline: Option<String>,
    diff: Option<&git2::Diff>,
) -> Result<String, Box<dyn std::error::Error>> {
    compose_message(&commit_message_file(repo), headline, diff)
}

pub fn compose_message(
    msg_file: &std::path::PathBuf,
    headline: Option<String>,
    diff: Option<&git2::Diff>,
) -> Result<String, Box<dyn std::error::Error>> {
    let headline = headline.unwrap_or("".to_string());
    let diff = match diff {
        Some(diff) => diffs::render(diff)?,
        None => "".to_string(),
    };

    let separator = "# ------------------------ >8 ------------------------";
    let init_contents = [
        headline.as_str(),
        "",
        separator,
        "# Do not modify or remove the line above.",
        "# Everything below it will be ignored.",
        diff.as_str(),
    ]
    .join("\n");

    let msg = compose_message_plain(msg_file, init_contents)?;
    let msg = msg.split(separator).next().unwrap_or("").trim();

    let all_whitespace = msg.chars().all(|c| c.is_whitespace());
    if all_whitespace {
        Err(git2::Error::new(
            git2::ErrorCode::User,
            git2::ErrorClass::None,
            "Empty commit message",
        ))?
    }

    let msg = git2::message_prettify(msg, Some('#'.try_into().unwrap()))?;
    Ok(msg)
}

pub fn commit_signed<'a, 'b>(
    repo: &'a Repository,
    author: &git2::Signature,
    committer: &git2::Signature,
    message: impl AsRef<str>,
    tree: &git2::Tree,
    parents: impl IntoIterator<Item = &'b Commit<'a>>,
) -> Result<Oid, git2::Error>
where
    'a: 'b,
{
    let parents: Vec<_> = parents.into_iter().collect();

    let commit_buffer = repo.commit_create_buffer(
        author,
        committer,
        message.as_ref(),
        tree,
        parents.as_slice(),
    )?;
    let commit_buffer_str = commit_buffer.as_str().expect("Invalid commit buffer");

    let signature = {
        let mut ctx = gpgme::Context::from_protocol(gpgme::Protocol::OpenPgp).map_err(|err| {
            git2::Error::new(
                git2::ErrorCode::User,
                git2::ErrorClass::None,
                format!("Failed to instantiate GPG context: {err}"),
            )
        })?;
        ctx.set_armor(true);

        let mut sig_out = Vec::new();
        ctx.sign(gpgme::SignMode::Detached, commit_buffer_str, &mut sig_out)
            .map_err(|err| {
                git2::Error::new(
                    git2::ErrorCode::User,
                    git2::ErrorClass::None,
                    format!("Failed to sign commit: {err}"),
                )
            })?;

        std::str::from_utf8(&sig_out)
            .expect("Signature is not valid UTF-8")
            .to_string()
    };

    repo.commit_signed(commit_buffer_str, &signature, None)
}

pub fn commit<'a, 'b>(
    repo: &'a Repository,
    author: &git2::Signature,
    committer: &git2::Signature,
    message: impl AsRef<str>,
    tree: &git2::Tree,
    parents: impl IntoIterator<Item = &'b Commit<'a>>,
) -> Result<Oid, git2::Error>
where
    'a: 'b,
{
    let parents: Vec<_> = parents.into_iter().collect();

    log::debug!(
        "Committing {} with parents {:?}",
        tree.id(),
        parents.as_slice()
    );

    repo.commit(
        None,
        author,
        committer,
        message.as_ref(),
        tree,
        parents.as_slice(),
    )
}

pub fn cherry_pick(
    repo: &Repository,
    target: &Commit,
    cherry: &Commit,
    sign: bool,
) -> Result<Oid, Error> {
    log::debug!("Cherry-picking {} onto {}", cherry.id(), target.id());

    let mut new_index = repo.cherrypick_commit(cherry, target, 0, None)?;
    if new_index.has_conflicts() {
        Err(Error::CherryPickConflict(CherryPickConflict {
            target: target.id(),
            cherry: cherry.id(),
            conflicts: new_index.conflicts()?.collect::<Result<Vec<_>, _>>()?,
        }))?
    }

    let new_tree = repo.find_tree(new_index.write_tree_to(repo)?)?;

    Ok((if sign { commit_signed } else { commit })(
        repo,
        &cherry.author(),
        &cherry.committer(),
        cherry.message().unwrap_or(""),
        &new_tree,
        [target],
    )?)
}

fn reapply_tree_changes<'a>(
    repo: &'a Repository,
    base: &Tree,
    changes: &Tree,
    target: &Tree,
) -> Result<Tree<'a>, git2::Error> {
    let mut merge_result = repo.merge_trees(base, changes, target, None)?;

    if merge_result.has_conflicts() {
        return Err(git2::Error::new(
            git2::ErrorCode::Conflict,
            git2::ErrorClass::Tree,
            format!(
                "Could not re-apply changes from {} on top of {}",
                changes.id(),
                target.id()
            ),
        ));
    }

    let tree = repo.find_tree(merge_result.write_tree_to(repo)?)?;
    Ok(tree)
}

fn unstaged_tree<'a>(repo: &'a Repository, index_tree: &Tree) -> Result<Tree<'a>, git2::Error> {
    let unstaged_changes = repo.diff_tree_to_workdir(Some(index_tree), None)?;
    let mut workdir = repo.apply_to_tree(index_tree, &unstaged_changes, None)?;

    if workdir.has_conflicts() {
        return Err(git2::Error::new(
            git2::ErrorCode::Conflict,
            git2::ErrorClass::Tree,
            "Could not capture unstaged changes due to a conflict",
        ));
    }

    let wt_tree_id = workdir.write_tree_to(repo)?;
    let wt_tree = repo.find_tree(wt_tree_id)?;

    Ok(wt_tree)
}

pub fn checkout(repo: &Repository, commit: &Commit) -> Result<(), git2::Error> {
    // Index
    let mut index = repo.index()?;
    index.read(false)?;

    let head_tree = repo.head()?.peel_to_commit()?.tree()?;
    let target_tree = commit.tree()?;

    // Obtain tree for the currently staged changes
    let current_index_tree = repo.find_tree(index.write_tree_to(repo)?)?;

    // Rebase the changes on top of the destination tree
    let new_index_tree = reapply_tree_changes(repo, &head_tree, &current_index_tree, &target_tree)?;

    // Obtain working directory changes relative to the new index tree
    let workdir_tree = unstaged_tree(repo, &new_index_tree)?;

    // Rebase the working directory changes on top of the destination tree
    let new_workdir_tree = reapply_tree_changes(repo, &head_tree, &workdir_tree, &target_tree)?;

    // Move HEAD
    repo.set_head_detached(commit.id())?;
    repo.reset(commit.as_object(), ResetType::Hard, None)?;

    // Ensure working tree has the right changes.
    repo.checkout_tree(new_workdir_tree.as_object(), None)?;

    // [checkout_tree] above also updates the index, so we need to reset that one.
    index.read_tree(&new_index_tree)?;
    index.write()?;

    Ok(())
}

pub fn capture_tree(repo: &Repository, use_index: bool) -> Result<git2::Tree, git2::Error> {
    let head = repo.head()?.peel_to_commit()?;

    let mut index = repo.index()?;
    let index_tree_id = index.write_tree_to(repo)?;

    let tree = if index_tree_id == head.tree_id() && !use_index {
        // No changes were staged in the index, therefore we use the working directory
        let index_tree = repo.find_tree(index_tree_id)?;
        unstaged_tree(repo, &index_tree)?
    } else {
        repo.find_tree(index_tree_id)?
    };

    Ok(tree)
}

pub mod serde {
    pub mod oid {

        use git2::Oid;
        use serde::{de::Error, Deserialize, Deserializer, Serialize, Serializer};

        #[repr(transparent)]
        pub struct SerDeOid(Oid);

        impl Serialize for SerDeOid {
            fn serialize<S>(&self, ser: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serialize(&self.0, ser)
            }
        }

        impl<'de> Deserialize<'de> for SerDeOid {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserialize(deserializer).map(SerDeOid)
            }
        }

        pub fn serialize<S: Serializer>(value: &Oid, ser: S) -> Result<S::Ok, S::Error> {
            ser.serialize_str(value.to_string().as_str())
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Oid, D::Error> {
            let str = String::deserialize(de)?;
            Oid::from_str(str.as_str()).map_err(D::Error::custom)
        }
    }

    pub mod vec_oid {
        use super::oid::SerDeOid;
        use git2::Oid;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};

        pub fn serialize<S: Serializer>(value: &Vec<Oid>, ser: S) -> Result<S::Ok, S::Error> {
            Vec::serialize(
                unsafe { std::mem::transmute::<&Vec<Oid>, &Vec<SerDeOid>>(value) },
                ser,
            )
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(de: D) -> Result<Vec<Oid>, D::Error> {
            let result = Vec::deserialize(de)?;
            Ok(unsafe { std::mem::transmute::<Vec<SerDeOid>, Vec<Oid>>(result) })
        }
    }

    pub mod hashmap_oid {
        use super::oid::SerDeOid;
        use git2::Oid;
        use serde::{Deserialize, Deserializer, Serialize, Serializer};
        use std::{collections::HashMap, hash::Hash};

        pub fn serialize<K: Serialize, S: Serializer>(
            value: &HashMap<K, Oid>,
            ser: S,
        ) -> Result<S::Ok, S::Error> {
            HashMap::serialize(
                unsafe { std::mem::transmute::<&HashMap<K, Oid>, &HashMap<K, SerDeOid>>(value) },
                ser,
            )
        }

        pub fn deserialize<'de, K: Deserialize<'de> + Hash + Eq, D: Deserializer<'de>>(
            de: D,
        ) -> Result<HashMap<K, Oid>, D::Error> {
            let result = HashMap::deserialize(de)?;
            Ok(unsafe { std::mem::transmute::<HashMap<K, SerDeOid>, HashMap<K, Oid>>(result) })
        }
    }
}
