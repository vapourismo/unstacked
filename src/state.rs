use crate::repo::Repo;
use git2::Oid;
use serde::{Deserialize, Serialize};

pub struct Manager {
    repo: Repo,
}

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
pub enum Error {
    Git(git2::Error),
    Serde(serde_json::Error),
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
    head: PlainOid,
    prev: Box<Realised>,
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
                let head = PlainOid(mgr.repo.head()?.peel_to_commit()?.id());
                let prev = Box::new(Realised::Stop);
                Ok(State { next, head, prev })
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
        let head = mgr.repo().head()?.peel_to_commit()?;

        if self.head.0 != head.id() {
            // Unexpected HEAD
            return Err(Error::UnexpectedHEAD);
        }

        Ok(self)
    }
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Unrealised {
    Commit {
        next: Box<Unrealised>,
        name: Option<String>,
        commit: PlainOid,
    },
    Bookmark {
        next: Box<Unrealised>,
        name: String,
        commit: PlainOid,
    },
    Stop,
}

#[derive(Deserialize, Serialize, Debug)]
pub enum Realised {
    Commit {
        commit: PlainOid,
        prev: Box<Realised>,
    },
    Bookmark {
        name: String,
        commit: PlainOid,
        prev: Box<Realised>,
    },
    Stop,
}
