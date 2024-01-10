use crate::git_helper;
use git2::{Error, ErrorClass, ErrorCode, Oid, Repository};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize)]
pub enum Action {
    CherryPick {
        sign: bool,

        #[serde(with = "crate::git_helper::serde::oid")]
        target: Oid,

        #[serde(with = "crate::git_helper::serde::oid")]
        cherry: Oid,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[repr(transparent)]
pub struct GitOpCache {
    #[serde(with = "crate::git_helper::serde::hashmap_oid")]
    items: HashMap<Action, Oid>,
}

impl GitOpCache {
    pub fn new() -> Self {
        Self {
            items: HashMap::new(),
        }
    }

    pub fn cherry_pick(
        &mut self,
        repo: &Repository,
        target: Oid,
        cherry: Oid,
        sign: bool,
    ) -> Result<Oid, git_helper::Error> {
        let action = Action::CherryPick {
            cherry,
            target,
            sign,
        };

        match self.items.get(&action) {
            Some(id) => {
                log::debug!("Found {action:?} in cache: {id}");
                Ok(*id)
            }

            None => {
                let target = repo.find_commit(target)?;
                let cherry = repo.find_commit(cherry)?;
                let id = git_helper::cherry_pick(repo, &target, &cherry, sign)?;

                self.items.insert(action, id);

                Ok(id)
            }
        }
    }
}

const CACHE_REF: &str = "refs/unstacked/cache";

pub struct CachedRepo {
    repo: Repository,
    cache: GitOpCache,
}

impl CachedRepo {
    pub fn discover(path: impl AsRef<std::path::Path>) -> Result<CachedRepo, Error> {
        let repo = Repository::discover(path)?;
        Self::from_repo(repo)
    }

    pub fn from_repo(repo: Repository) -> Result<Self, Error> {
        let cache = repo
            .find_reference(CACHE_REF)
            .and_then(|reff| {
                let json = reff.peel_to_blob()?;
                serde_json::de::from_slice(json.content()).or_else(|_| Ok(GitOpCache::new()))
            })
            .or_else(|err| {
                if err.code() == git2::ErrorCode::NotFound {
                    Ok(GitOpCache::new())
                } else {
                    Err(err)
                }
            })?;

        Ok(Self { repo, cache })
    }

    pub fn cherry_pick(
        &mut self,
        target: Oid,
        cherry: Oid,
        sign: bool,
    ) -> Result<Oid, git_helper::Error> {
        self.cache.cherry_pick(&self.repo, target, cherry, sign)
    }

    pub fn repo(&self) -> &Repository {
        &self.repo
    }

    #[allow(dead_code)]
    pub fn save(&self) -> Result<(), Error> {
        let data = serde_json::ser::to_vec_pretty(&self.cache).map_err(|err| {
            Error::new(
                ErrorCode::GenericError,
                ErrorClass::None,
                format!("Could not serialise GitOpCache: {err}"),
            )
        })?;

        let blob = self.repo.blob(data.as_slice())?;
        self.repo.reference(CACHE_REF, blob, true, "")?;

        Ok(())
    }
}

impl Drop for CachedRepo {
    fn drop(&mut self) {
        // TODO: Save Git cache
        // if let Err(err) = self.save() {
        //     log::warn!("Failed to save Git cache: {err}");
        // }
    }
}
