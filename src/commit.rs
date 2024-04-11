use crate::repo::{self, Repo};
use git2::{Index, IndexEntry, MergeOptions, Oid};

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
pub enum Error {
    Git(git2::Error),
    Repo(repo::Error),

    #[display(fmt = "Could not cherry-pick {cherry} onto {commit} in a conflict-free way")]
    CherryPickError {
        commit: Oid,
        cherry: Oid,
    },
}

#[derive(
    Clone,
    derive_more::From,
    derive_more::Into,
    derive_more::AsRef,
    derive_more::AsMut,
    derive_more::Deref,
)]
pub struct Commit<'a>(pub git2::Commit<'a>);

fn remove_conflict(index: &Index, entry: &IndexEntry) {
    struct MyIndex {
        raw: *mut libgit2_sys::git_index,
    }

    unsafe {
        let funky_index: &MyIndex = std::mem::transmute(index);
        let path = entry.path.as_ptr();
        let result = libgit2_sys::git_index_conflict_remove(funky_index.raw, path.cast());
        assert_eq!(result, 0);
    }
}

impl<'a> Commit<'a> {
    pub fn cherry_pick(
        &self,
        repo: &'a Repo,
        cherry: &Self,
        sign: bool,
        forceful: bool,
    ) -> Result<Commit<'a>, Error> {
        assert_eq!(cherry.0.parent_count(), 1);

        let mut merge_options = MergeOptions::new();

        if forceful {
            merge_options.file_favor(git2::FileFavor::Theirs);
        }

        let mut new_index =
            repo.0
                .cherrypick_commit(&cherry.0, &self.0, 0, Some(&merge_options))?;

        if new_index.has_conflicts() {
            if forceful {
                return Err(Error::CherryPickError {
                    cherry: cherry.id(),
                    commit: self.id(),
                });
            }

            let new_entries = new_index
                .conflicts()?
                .map(|conflict| {
                    let conflict = conflict?;
                    match (conflict.our, conflict.their) {
                        // A new file has been created
                        (None, Some(index)) => Ok(index),

                        _ => Err(Error::CherryPickError {
                            cherry: cherry.id(),
                            commit: self.id(),
                        }),
                    }
                })
                // Need to collect to relinquish the reference to [index].
                .collect::<Result<Vec<_>, _>>()?;

            for mut entry in new_entries {
                // Set stage to 0
                entry.flags &= !0b11_0000_0000_0000;

                remove_conflict(&new_index, &entry);
                new_index.add(&entry)?;
            }
        }

        let new_tree = repo.0.find_tree(new_index.write_tree_to(&repo.0)?)?;

        let new_commit = if sign {
            repo.commit_signed(
                &cherry.0.author(),
                &cherry.0.committer(),
                cherry.0.message().unwrap_or(""),
                &new_tree,
                [self],
            )?
        } else {
            repo.commit(
                &cherry.0.author(),
                &cherry.0.committer(),
                cherry.0.message().unwrap_or(""),
                &new_tree,
                [self],
            )?
        };

        Ok(new_commit)
    }

    pub fn id(&self) -> git2::Oid {
        self.0.id()
    }
}
