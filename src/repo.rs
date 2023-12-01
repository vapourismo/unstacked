use crate::commit::Commit;
use std::{error::Error, path::Path};

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

    pub fn commit_signed<'a>(
        &'a self,
        author: &git2::Signature,
        committer: &git2::Signature,
        message: impl AsRef<str>,
        tree: &git2::Tree,
        parent: &Commit,
    ) -> Result<Commit<'a>, Box<dyn Error>> {
        let commit_buffer =
            self.0
                .commit_create_buffer(author, committer, message.as_ref(), tree, &[&parent.0])?;
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

    pub fn commit<'a>(
        &'a self,
        author: &git2::Signature,
        committer: &git2::Signature,
        message: impl AsRef<str>,
        tree: &git2::Tree,
        parent: &Commit,
    ) -> Result<Commit<'a>, Box<dyn Error>> {
        let new_commit_oid = self.0.commit(
            None,
            author,
            committer,
            message.as_ref(),
            tree,
            &[&parent.0],
        )?;
        let new_commit = self.0.find_commit(new_commit_oid)?;
        Ok(Commit(new_commit))
    }
}
