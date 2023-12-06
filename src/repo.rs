use crate::commit::Commit;
use auth_git2::GitAuthenticator;
use git2::Oid;
use std::{error::Error, path::Path};

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

    pub fn commit_signed<'a, 'b>(
        &'a self,
        author: &git2::Signature,
        committer: &git2::Signature,
        message: impl AsRef<str>,
        tree: &git2::Tree,
        parents: impl IntoIterator<Item = &'b Commit<'a>>,
    ) -> Result<Commit<'a>, Box<dyn Error>>
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
            .ok_or_else(|| -> Box<dyn Error> { "Empty commit buffer string".into() })?;

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
    ) -> Result<Commit<'a>, Box<dyn Error>>
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
    ) -> Result<Commit<'a>, Box<dyn Error>> {
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
}
