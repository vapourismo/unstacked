use crate::repo::{self, Repo};
use git2::Oid;

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

impl<'a> Commit<'a> {
    pub fn cherry_pick(
        &self,
        repo: &'a Repo,
        cherry: &Self,
        sign: bool,
    ) -> Result<Commit<'a>, Error> {
        assert_eq!(cherry.0.parent_count(), 1);

        let mut new_index = repo.0.cherrypick_commit(&cherry.0, &self.0, 0, None)?;

        if new_index.has_conflicts() {
            return Err(Error::CherryPickError {
                cherry: cherry.id(),
                commit: self.id(),
            });
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
