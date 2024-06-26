use crate::{
    commit::{self, Commit},
    diffs,
    repo::{self, Repo},
};
use git2::{Oid, ResetType, Signature};
use serde::{Deserialize, Serialize};
use std::{env, fmt, fs, io, path, process, string::FromUtf8Error};
use termion::color::*;

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

    pub fn commit_info_file(&self) -> path::PathBuf {
        self.dot_git_child("COMMITINFO_EDIT")
    }

    pub fn compose_message_plain(
        &self,
        msg_file: &path::PathBuf,
        body: String,
    ) -> Result<String, Error> {
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

    pub fn compose_message(
        &self,
        msg_file: &path::PathBuf,
        headline: Option<String>,
        diff: Option<&git2::Diff>,
    ) -> Result<String, Error> {
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

        let msg = self.compose_message_plain(msg_file, init_contents)?;
        let msg = msg.split(separator).next().unwrap_or("").trim();

        let all_whitespace = msg.chars().all(|c| c.is_whitespace());
        if all_whitespace {
            return Err(Error::EmptyMessage);
        }

        let msg = git2::message_prettify(msg, Some('#'.try_into().unwrap()))?;

        Ok(msg)
    }

    pub fn compose_commit_message(
        &self,
        headline: Option<String>,
        diff: Option<&git2::Diff>,
    ) -> Result<String, Error> {
        self.compose_message(&self.commit_message_file(), headline, diff)
    }

    pub fn commit_info(&self) -> Result<CommitInfo, Error> {
        let head = self.repo.head_commit()?;

        let author = head.author();
        let author = PlainSig {
            name: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
        };

        let committer = head.committer();
        let committer = PlainSig {
            name: committer.name().unwrap_or("").to_string(),
            email: committer.email().unwrap_or("").to_string(),
        };

        Ok(CommitInfo {
            author,
            committer,
            message: head.message().unwrap_or("").to_string(),
        })
    }

    pub fn edit(&self, info: &CommitInfo) -> Result<MoveResult, Error> {
        let head = self.repo.head_commit()?;

        let author = Signature::new(
            info.author.name.as_str(),
            info.author.email.as_str(),
            &head.author().when(),
        )?;

        let committer = Signature::new(
            info.committer.name.as_str(),
            info.committer.email.as_str(),
            &head.committer().when(),
        )?;

        let new_head = head.amend(
            None,
            Some(&author),
            Some(&committer),
            None,
            Some(info.message.as_str()),
            Some(&head.tree()?),
        )?;
        let new_head = self.repo.0.find_commit(new_head)?;

        self.repo
            .0
            .reset(new_head.as_object(), ResetType::Soft, None)?;

        Ok(MoveResult::moved(&head, &new_head))
    }

    pub fn capture_tree(&self, use_index: bool) -> Result<git2::Tree, Error> {
        let head: Commit = self.repo.head_commit()?;

        let mut index = self.repo.index()?;
        let index_tree_id = index.write_tree_to(&self.repo.0)?;

        let tree = if index_tree_id == head.tree_id() && !use_index {
            // No changes were staged in the index, therefore we use the working directory
            self.repo.unstaged_tree(&head.tree()?)?
        } else {
            self.repo.find_tree(index_tree_id)?
        };

        Ok(tree)
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

#[derive(Debug, Clone)]
pub enum MoveResult {
    Stationary {
        head: Oid,
        message: String,
        author: String,
        email: String,
    },

    Moved {
        from: Oid,
        to: Oid,
        message: String,
        author: String,
        email: String,
    },
}

impl MoveResult {
    pub fn stationary(head: &git2::Commit) -> Self {
        let sig = head.author();
        Self::Stationary {
            head: head.id(),
            message: head.message().unwrap_or("<no message>").to_string(),
            author: sig.name().unwrap_or("Unknown").to_string(),
            email: sig.email().unwrap_or("unknown").to_string(),
        }
    }

    pub fn moved(from: &git2::Commit, to: &git2::Commit) -> Self {
        let sig = to.author();
        Self::Moved {
            from: from.id(),
            to: to.id(),
            message: to.message().unwrap_or("<no message>").to_string(),
            author: sig.name().unwrap_or("Unknown").to_string(),
            email: sig.email().unwrap_or("unknown").to_string(),
        }
    }
}
impl fmt::Display for MoveResult {
    fn fmt(&self, fmt: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MoveResult::Stationary {
                head,
                message,
                author,
                email,
            } => {
                writeln!(fmt, "commit  {}{head}{}", Fg(Green), Fg(Reset))?;
                writeln!(fmt, "author  {}{author} <{email}>{}", Fg(Yellow), Fg(Reset))?;
                write!(fmt, "message {}{message}{}", Fg(Cyan), Fg(Reset))?;
            }

            MoveResult::Moved {
                from,
                to,
                message,
                author,
                email,
            } => {
                writeln!(fmt, "previous {}{from}{}", Fg(Red), Fg(Reset))?;
                writeln!(fmt, "commit   {}{to}{}", Fg(Green), Fg(Reset))?;
                writeln!(
                    fmt,
                    "author   {}{author} <{email}>{}",
                    Fg(Yellow),
                    Fg(Reset)
                )?;
                write!(fmt, "message  {}{message}{}", Fg(Cyan), Fg(Reset))?;
            }
        }

        Ok(())
    }
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
        let head = mgr.repo.head_commit()?;
        let parent: Commit = mgr.repo.0.find_commit(head.parent_id(0)?)?.into();

        self.next = Box::new(Unrealised::Commit {
            next: self.next.clone(),
            commit: PlainOid(head.id()),
        });

        mgr.repo.goto(&parent)?;
        self.write(mgr)?;

        Ok(MoveResult::moved(&head, &parent))
    }

    pub fn next(&mut self, mgr: &Manager) -> Result<MoveResult, Error> {
        let head: Commit = mgr.repo.head_commit()?;

        match self.next.as_ref() {
            Unrealised::Commit { next, commit } => {
                let cherry: Commit = mgr.repo.0.find_commit(commit.0)?.into();

                let new_head = if cherry.parent_count() == 1
                    && cherry
                        .parent(0)
                        .map(|cherry_parent| cherry_parent.id() == head.id())
                        .unwrap_or(false)
                {
                    cherry
                } else {
                    head.cherry_pick(mgr.repo(), &cherry, false, false)?
                };

                self.next = next.clone();

                mgr.repo.goto(&new_head)?;
                self.write(mgr)?;

                Ok(MoveResult::moved(&head, &new_head))
            }

            Unrealised::Stop => Ok(MoveResult::stationary(&head)),
        }
    }

    pub fn commit(
        &mut self,
        mgr: &Manager,
        msg: Option<String>,
        use_index: bool,
    ) -> Result<MoveResult, Error> {
        let tree = mgr.capture_tree(use_index)?;
        let head: Commit = mgr.repo.head_commit()?;

        let msg = match msg {
            Some(msg) => msg,
            None => {
                let diff = mgr
                    .repo()
                    .diff_tree_to_tree(Some(&head.tree()?), Some(&tree), None)?;
                mgr.compose_commit_message(None, Some(&diff))?
            }
        };

        let sig = mgr.repo.signature()?;
        let new_head_commit = mgr.repo.commit(&sig, &sig, msg, &tree, [&head])?;

        mgr.repo
            .reset(new_head_commit.as_object(), ResetType::Mixed, None)?;
        self.write(mgr)?;

        Ok(MoveResult::moved(&head, &new_head_commit))
    }

    pub fn amend(&mut self, mgr: &Manager, use_index: bool) -> Result<MoveResult, Error> {
        let new_tree = mgr.capture_tree(use_index)?;

        let head = mgr.repo.head_commit()?;
        let new_head_id = head.amend(None, None, None, None, None, Some(&new_tree))?;
        let new_head = mgr.repo.0.find_commit(new_head_id)?;

        mgr.repo
            .0
            .reset(new_head.as_object(), ResetType::Mixed, None)?;
        self.write(mgr)?;

        Ok(MoveResult::moved(&head, &new_head))
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

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct PlainSig {
    pub name: String,
    pub email: String,
}

#[derive(Deserialize, Serialize, Debug, Clone, PartialEq, Eq)]
pub struct CommitInfo {
    pub author: PlainSig,
    pub committer: PlainSig,
    pub message: String,
}
