use crate::repo::Repo;
use std::error::Error;

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
    ) -> Result<Commit<'a>, Box<dyn Error>> {
        assert_eq!(cherry.0.parent_count(), 1);

        let mut new_index = repo.0.cherrypick_commit(&cherry.0, &self.0, 0, None)?;

        if new_index.has_conflicts() {
            return Err(format!(
                "Could not cherry-pick {} onto {} in a conflict-free way",
                cherry.id(),
                self.id()
            )
            .into());
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
