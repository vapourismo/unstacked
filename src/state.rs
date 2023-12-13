use std::{env, fs, io, path, process, string::FromUtf8Error};

use crate::{
    commit::{self, Commit},
    repo::{self, Repo},
};
use git2::{Oid, ResetType};
use serde::{Deserialize, Serialize};

pub struct Manager {
    repo: Repo,
}

impl Manager {
    pub fn new(repo: Repo) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    fn dot_git_path(&self) -> path::PathBuf {
        self.repo.path().into()
    }

    fn dot_git_child(&self, name: impl AsRef<path::Path>) -> path::PathBuf {
        let mut path = self.dot_git_path();
        path.push(name);
        path
    }

    fn commit_message_file(&self) -> path::PathBuf {
        self.dot_git_child("COMMIT_EDITMSG")
    }

    pub fn compose_message(
        &self,
        msg_file: path::PathBuf,
        headline: Option<String>,
    ) -> Result<String, Error> {
        let editor = env::var("EDITOR").expect("Need $EDITOR set when omitting commit message");

        let headline = headline.unwrap_or("".to_string());
        let separator = "# ------------------------ >8 ------------------------";
        let init_contents = [
            headline.as_str(),
            "",
            "# Everthing below the following line will be ignored.",
            separator,
        ]
        .join("\n");
        fs::write(&msg_file, &init_contents)?;

        let exit = process::Command::new(editor)
            .arg(&msg_file)
            .spawn()?
            .wait()?;

        assert!(exit.success());

        let msg = fs::read(&msg_file)?;
        let msg = String::from_utf8(msg)?;
        let msg = msg.split(separator).next().unwrap_or("").trim();
        fs::remove_file(&msg_file)?;

        let all_whitespace = msg.chars().all(|c| c.is_whitespace());
        if all_whitespace {
            return Err(Error::EmptyMessage);
        }

        let msg = git2::message_prettify(msg, Some('#'.try_into().unwrap()))?;

        Ok(msg)
    }

    pub fn compose_commit_message(&self, headline: Option<String>) -> Result<String, Error> {
        self.compose_message(self.commit_message_file(), headline)
    }
}

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
pub enum Error {
    Git(git2::Error),
    Serde(serde_json::Error),
    Repo(repo::Error),
    Commit(commit::Error),
    IO(io::Error),
    Utf8(FromUtf8Error),
    EmptyMessage,
}

const STATE_REF: &str = "refs/unstacked/state";

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

#[derive(Debug, Clone, derive_more::Display)]
pub enum MoveResult {
    #[display(fmt = "HEAD has not moved")]
    Stationary,

    #[display(fmt = "{from}..{to}: {message}")]
    Moved { from: Oid, to: Oid, message: String },
}

#[derive(Deserialize, Serialize, Debug)]
pub struct State {
    next: Box<Unrealised>,
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
                Ok(State { next })
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

    pub fn validate(self, _mgr: &Manager) -> Result<Self, Error> {
        Ok(self)
    }

    pub fn prev(&mut self, mgr: &Manager) -> Result<MoveResult, Error> {
        let head = mgr.repo.head()?.peel_to_commit()?;
        let parent = mgr.repo.0.find_commit(head.parent_id(0)?)?;
        let parent_id = parent.id();

        self.next = Box::new(Unrealised::Commit {
            next: self.next.clone(),
            commit: PlainOid(head.id()),
        });

        mgr.repo
            .0
            .reset(parent.as_object(), ResetType::Soft, None)?;
        self.write(mgr)?;

        Ok(MoveResult::Moved {
            from: head.id(),
            to: parent_id,
            message: parent.message().unwrap_or("<no message>").to_string(),
        })
    }

    pub fn next(&mut self, mgr: &Manager) -> Result<MoveResult, Error> {
        match self.next.as_ref() {
            Unrealised::Commit { next, commit } => {
                let cherry: Commit = mgr.repo.0.find_commit(commit.0)?.into();
                let head: Commit = mgr.repo.head()?.peel_to_commit()?.into();

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

                mgr.repo
                    .0
                    .reset(new_head.as_object(), ResetType::Soft, None)?;
                self.write(mgr)?;

                Ok(MoveResult::Moved {
                    from: head.id(),
                    to: new_head.id(),
                    message: new_head.message().unwrap_or("<no message>").to_string(),
                })
            }

            Unrealised::Stop => Ok(MoveResult::Stationary),
        }
    }

    pub fn commit(&mut self, mgr: &Manager, msg: impl AsRef<str>) -> Result<MoveResult, Error> {
        let head: Commit = mgr.repo.0.head()?.peel_to_commit()?.into();

        let mut index = mgr.repo.0.index()?;
        let new_tree = index.write_tree_to(&mgr.repo.0)?;
        let new_tree = mgr.repo.0.find_tree(new_tree)?;

        let sig = mgr.repo.0.signature()?;
        let new_head_commit = mgr.repo.commit(&sig, &sig, msg, &new_tree, [&head])?;

        mgr.repo
            .0
            .reset(new_head_commit.as_object(), ResetType::Soft, None)?;
        self.write(mgr)?;

        Ok(MoveResult::Moved {
            from: head.id(),
            to: new_head_commit.id(),
            message: new_head_commit
                .message()
                .unwrap_or("<no message>")
                .to_string(),
        })
    }

    pub fn amend(&mut self, mgr: &Manager) -> Result<MoveResult, Error> {
        let head = mgr.repo.0.head()?.peel_to_commit()?;

        let mut index = mgr.repo.0.index()?;
        let new_tree = index.write_tree_to(&mgr.repo.0)?;
        let new_tree = mgr.repo.0.find_tree(new_tree)?;

        let new_head_id = head.amend(None, None, None, None, None, Some(&new_tree))?;
        let new_head_commit = mgr.repo.0.find_commit(new_head_id)?;

        mgr.repo
            .0
            .reset(new_head_commit.as_object(), ResetType::Soft, None)?;
        self.write(mgr)?;

        Ok(MoveResult::Moved {
            from: head.id(),
            to: new_head_id,
            message: new_head_commit
                .message()
                .unwrap_or("<no message>")
                .to_string(),
        })
    }
}

#[derive(Deserialize, Serialize, Debug, Clone)]
pub enum Unrealised {
    Stop,
    Commit {
        next: Box<Unrealised>,
        commit: PlainOid,
    },
}
