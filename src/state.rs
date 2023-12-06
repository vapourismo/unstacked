use crate::{
    commit::{self, Commit},
    repo::Repo,
};
use git2::{Oid, ResetType};
use serde::{Deserialize, Serialize};

pub struct Manager {
    repo: Repo,
}

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
pub enum Error {
    Git(git2::Error),
    Serde(serde_json::Error),
    Commit(commit::Error),
    UnexpectedHEAD,
}

const STATE_REF: &str = "refs/unstacked/state";

impl Manager {
    pub fn new(repo: Repo) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }
}

#[repr(transparent)]
#[derive(Debug, derive_more::Display, Clone, Copy)]
pub struct PlainOid(Oid);

impl<'de> Deserialize<'de> for PlainOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let str = String::deserialize(deserializer)?;
        Oid::from_str(str.as_str())
            .map(PlainOid)
            .map_err(serde::de::Error::custom)
    }
}

impl Serialize for PlainOid {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.0.to_string().serialize(serializer)
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub struct State {
    next: Box<Unrealised>,
    head: Box<Realised>,
}

impl State {
    pub fn read(mgr: &Manager) -> Result<Self, Error> {
        match mgr.repo.find_reference(STATE_REF) {
            Ok(ref_) => {
                let oid = ref_.peel_to_blob()?;
                Ok(serde_json::de::from_slice(oid.content())?)
            }

            Err(git_error) if git_error.code() == git2::ErrorCode::NotFound => {
                let next = Box::new(Unrealised::Stop);
                let head = Box::new(Realised::Stop);
                Ok(State { next, head })
            }

            Err(err) => Err(err.into()),
        }
    }

    pub fn write(&self, mgr: &Manager) -> Result<(), Error> {
        let contents = serde_json::ser::to_vec_pretty(self)?;
        let oid = mgr.repo.blob(contents.as_slice())?;
        mgr.repo.update_reference(STATE_REF, oid)?;
        Ok(())
    }

    pub fn validate(self, mgr: &Manager) -> Result<Self, Error> {
        match self.head.as_ref() {
            Realised::Commit { commit, .. } => {
                let head = mgr.repo.head()?.peel_to_commit()?;

                if head.id() != commit.0 {
                    return Err(Error::UnexpectedHEAD);
                }

                Ok(self)
            }
            Realised::Stop => Ok(self),
        }
    }

    pub fn prev(&mut self, mgr: &Manager) -> Result<(), Error> {
        match self.head.as_ref() {
            Realised::Commit { commit, prev } => todo!(),
            Realised::Stop => {
                let head = mgr.repo.head()?.peel_to_commit()?;
                let parent = mgr.repo.0.find_commit(head.parent_id(0)?)?;
                let parent_id = parent.id();

                self.next = Box::new(Unrealised::Commit {
                    next: self.next.clone(),
                    commit: PlainOid(head.id()),
                });

                self.head = Box::new(Realised::Commit {
                    commit: PlainOid(parent_id),
                    prev: Box::new(Realised::Stop),
                });

                mgr.repo
                    .0
                    .reset(parent.as_object(), ResetType::Soft, None)?;
                self.write(mgr)?;
            }
        }

        Ok(())
    }

    pub fn next(&mut self, mgr: &Manager) -> Result<(), Error> {
        match self.next.as_ref() {
            Unrealised::Commit { next, commit } => {
                let cherry: Commit = mgr.repo.0.find_commit(commit.0)?.into();

                let head: Commit = match self.head.as_ref() {
                    Realised::Commit { commit, .. } => mgr.repo.0.find_commit(commit.0)?.into(),
                    Realised::Stop => mgr.repo.head()?.peel_to_commit()?.into(),
                };

                let new_head = if cherry.parent_count() == 1
                    && cherry
                        .parent(0)
                        .map(|cherry_parent| cherry_parent.id() == head.id())
                        .unwrap_or(false)
                {
                    cherry
                } else {
                    head.cherry_pick(mgr.repo(), &cherry, false)?
                };

                self.next = next.clone();

                self.head = Box::new(Realised::Commit {
                    commit: PlainOid(new_head.id()),
                    prev: self.head.clone(),
                });

                mgr.repo
                    .0
                    .reset(new_head.as_object(), ResetType::Soft, None)?;
                self.write(mgr)?;
            }

            Unrealised::Stop => {}
        }

        Ok(())
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Unrealised {
    Commit {
        next: Box<Unrealised>,
        commit: PlainOid,
    },
    Stop,
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Realised {
    Commit {
        commit: PlainOid,
        prev: Box<Realised>,
    },
    Stop,
}
