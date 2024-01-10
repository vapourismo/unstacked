use super::rules::{self, RuleBook};
use crate::{git_cache::CachedRepo, git_helper};
use git2::Oid;
use serde::{Deserialize, Serialize};

pub type Index = Option<usize>;

#[derive(Debug, derive_more::Error, derive_more::Display, derive_more::From)]
pub enum Error {
    Git(git2::Error),
    Rule(rules::Error),

    #[display(fmt = "ConflictForPatch: {base} <- {patch}")]
    PatchConflict {
        index: usize,
        base: Oid,
        patch: Oid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Series {
    #[serde(with = "crate::git_helper::serde::vec_oid")]
    patches: Vec<Oid>,
    parent: String,
}

impl Series {
    pub fn new(parent: String) -> Self {
        Self {
            patches: Vec::new(),
            parent,
        }
    }

    pub fn build(&mut self, rules: &mut RuleBook, cache: &mut CachedRepo) -> Result<Oid, Error> {
        self.build_partial(rules, cache, self.patches.len())
    }

    pub fn build_partial(
        &mut self,
        rules: &mut RuleBook,
        cache: &mut CachedRepo,
        patches: usize,
    ) -> Result<Oid, Error> {
        let mut accum = rules.build(cache, self.parent.clone())?;
        for (index, patch) in self.patches.iter_mut().enumerate().take(patches) {
            accum = cache
                .cherry_pick(accum, *patch, false)
                .map_err(|err| match err {
                    git_helper::Error::GitError(git_error) => Error::Git(git_error),
                    git_helper::Error::CherryPickConflict(conflict) => Error::PatchConflict {
                        index,
                        base: conflict.target,
                        patch: conflict.cherry,
                    },
                })?;
            *patch = accum;
        }
        Ok(accum)
    }

    pub fn build_at(
        &mut self,
        rules: &mut RuleBook,
        cache: &mut CachedRepo,
        index: Index,
    ) -> Result<Oid, Error> {
        let patches = index.map(|i| i + 1).unwrap_or(self.num_patches());
        self.build_partial(rules, cache, patches)
    }

    pub fn parent(&self) -> &String {
        &self.parent
    }

    pub fn has_patches(&self) -> bool {
        !self.patches.is_empty()
    }

    pub fn num_patches(&self) -> usize {
        self.patches.len()
    }

    pub fn is_top_patch(&self, index: Index) -> bool {
        index.is_none() || index == self.num_patches().checked_sub(1)
    }

    pub fn set_patch(&mut self, index: usize, id: Oid) {
        self.patches[index] = id;
    }

    pub fn insert_patch(&mut self, index: Index, id: Oid) -> Index {
        let index = index.unwrap_or(self.num_patches());
        self.patches.insert(index, id);
        Some(index)
    }

    #[cfg(test)]
    pub fn push_patch(&mut self, id: Oid) {
        self.patches.push(id);
    }
}
