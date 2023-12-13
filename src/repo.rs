use crate::commit::Commit;
use auth_git2::GitAuthenticator;
use git2::{Diff, Oid, ResetType};
use std::{path::Path, str::Utf8Error};

#[derive(Debug, derive_more::Display, derive_more::From, derive_more::Error)]
pub enum Error {
    Git(git2::Error),
    Gpg(gpgme::Error),
    Utf8(Utf8Error),
    EmptyCommitMessage,
    IndexConflicts,
    WorkingDirConflicts,
}

#[derive(
    derive_more::From, derive_more::Into, derive_more::AsRef, derive_more::AsMut, derive_more::Deref,
)]
pub struct Repo(pub git2::Repository);

impl Repo {
    pub fn discover(path: impl AsRef<Path>) -> Result<Self, git2::Error> {
        let repo = git2::Repository::discover(path)?;
        Ok(Repo(repo))
    }

    pub fn find_commit<'a>(&'a self, ref_: impl AsRef<str>) -> Result<Commit<'a>, git2::Error> {
        let commit = self
            .0
            .revparse(ref_.as_ref())?
            .from()
            .expect("Bad commit")
            .peel_to_commit()?;

        Ok(Commit(commit))
    }

    pub fn head_commit(&self) -> Result<Commit, git2::Error> {
        let commit = self.0.head()?.peel_to_commit()?;
        Ok(Commit(commit))
    }

    pub fn commit_signed<'a, 'b>(
        &'a self,
        author: &git2::Signature,
        committer: &git2::Signature,
        message: impl AsRef<str>,
        tree: &git2::Tree,
        parents: impl IntoIterator<Item = &'b Commit<'a>>,
    ) -> Result<Commit<'a>, Error>
    where
        'a: 'b,
    {
        let parents: Vec<_> = parents.into_iter().map(|c| c.as_ref()).collect();
        let commit_buffer = self.0.commit_create_buffer(
            author,
            committer,
            message.as_ref(),
            tree,
            parents.as_slice(),
        )?;
        let commit_buffer_str = commit_buffer
            .as_str()
            .ok_or_else(|| Error::EmptyCommitMessage)?;

        let signature = {
            let mut ctx = gpgme::Context::from_protocol(gpgme::Protocol::OpenPgp)?;
            ctx.set_armor(true);
            let mut sig_out = Vec::new();
            ctx.sign(gpgme::SignMode::Detached, commit_buffer_str, &mut sig_out)?;
            std::str::from_utf8(&sig_out)?.to_string()
        };

        let new_commit_oid = self.0.commit_signed(commit_buffer_str, &signature, None)?;
        let new_commit = self.0.find_commit(new_commit_oid)?;

        Ok(Commit(new_commit))
    }

    pub fn commit<'a, 'b>(
        &'a self,
        author: &git2::Signature,
        committer: &git2::Signature,
        message: impl AsRef<str>,
        tree: &git2::Tree,
        parents: impl IntoIterator<Item = &'b Commit<'a>>,
    ) -> Result<Commit<'a>, Error>
    where
        'a: 'b,
    {
        let parents: Vec<_> = parents.into_iter().map(|c| c.as_ref()).collect();
        let new_commit_oid = self.0.commit(
            None,
            author,
            committer,
            message.as_ref(),
            tree,
            parents.as_slice(),
        )?;
        let new_commit = self.0.find_commit(new_commit_oid)?;
        Ok(Commit(new_commit))
    }

    pub fn merge<'a>(
        &'a self,
        first: &Commit<'a>,
        second: &Commit<'a>,
    ) -> Result<Commit<'a>, Error> {
        let mut index = self
            .0
            .merge_commits(first.as_ref(), second.as_ref(), None)?;
        let tree = index.write_tree_to(&self.0)?;
        let tree = self.find_tree(tree)?;

        let sig = self.signature()?;
        let commit = self.commit(&sig, &sig, "Test merge", &tree, [first, second])?;

        Ok(commit)
    }

    pub fn update_reference(
        &self,
        name: impl AsRef<str>,
        oid: Oid,
    ) -> Result<git2::Reference, git2::Error> {
        let ref_ = self.0.reference(name.as_ref(), oid, true, "Unstacked")?;
        Ok(ref_)
    }

    pub fn push(&self, remote: impl AsRef<str>, refspecs: &[&str]) -> Result<(), git2::Error> {
        let mut remote = self.0.find_remote(remote.as_ref())?;

        let auth = GitAuthenticator::default();
        let config = git2::Config::open_default()?;

        let mut remote_cbs = git2::RemoteCallbacks::new();
        remote_cbs.credentials(auth.credentials(&config));

        let mut conn = remote.connect_auth(git2::Direction::Push, Some(remote_cbs), None)?;

        conn.remote().push(
            refspecs.as_ref(),
            None, // Some(PushOptions::new().remote_callbacks(remote_cbs)),
        )?;

        Ok(())
    }

    pub fn merge_base<'a, 'b, CS>(&'a self, commits: CS) -> Result<Commit, git2::Error>
    where
        CS: IntoIterator<Item = &'b Commit<'a>>,
        'a: 'b,
    {
        let oids = commits
            .into_iter()
            .map(|c| c.as_ref().id())
            .collect::<Vec<Oid>>();
        let merge_base = self.0.merge_base_many(&oids)?;
        let commit = self.0.find_commit(merge_base)?;
        Ok(Commit(commit))
    }

    pub fn staged_changes(&self) -> Result<Diff, git2::Error> {
        self.0.diff_tree_to_index(
            Some(&self.head_commit()?.tree()?),
            Some(&self.0.index()?),
            None,
        )
    }

    pub fn unstaged_changes(&self) -> Result<Diff, git2::Error> {
        self.0.diff_index_to_workdir(Some(&self.0.index()?), None)
    }

    pub fn index_is_clean(&self) -> bool {
        self.staged_changes()
            .map(|changes| changes.deltas().len() == 0)
            .unwrap_or(false)
    }

    pub fn goto(&self, commit: &Commit) -> Result<(), Error> {
        // Prepare new index
        let head_tree = self.head_commit()?.tree()?;
        let target_tree = commit.tree()?;
        let mut current_index = self.0.index()?;
        current_index.read(false)?;
        let current_index_tree = self.0.find_tree(current_index.write_tree_to(&self.0)?)?;
        let mut new_index =
            self.0
                .merge_trees(&head_tree, &current_index_tree, &target_tree, None)?;
        assert!(!new_index.has_conflicts());
        let new_index_tree = self.0.find_tree(new_index.write_tree_to(&self.0)?)?;

        // Prepare new working directory
        let mut current_wt_index =
            self.apply_to_tree(&head_tree, &self.unstaged_changes()?, None)?;
        assert!(!current_wt_index.has_conflicts());
        let current_wt_tree = self.0.find_tree(current_wt_index.write_tree_to(&self.0)?)?;
        let mut new_wt_index =
            self.0
                .merge_trees(&head_tree, &current_wt_tree, &target_tree, None)?;

        for c in new_wt_index.conflicts()? {
            let c = c?;
            eprintln!("{:?} {:?} {:?}", c.ancestor, c.our, c.their);
        }

        assert!(!new_wt_index.has_conflicts());

        // HEAD
        self.0.reset(commit.as_object(), ResetType::Hard, None)?;

        // Working directory
        self.0.checkout_index(Some(&mut new_wt_index), None)?;

        // Index
        current_index.read(true)?;
        current_index.read_tree(&new_index_tree)?;
        current_index.write()?;

        Ok(())
    }
}
