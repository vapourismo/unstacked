use crate::{
    commit::Commit,
    repo::{self, Repo},
    state,
};
use git2::{Blob, FileMode, Oid};
use serde::{Deserialize, Serialize};

fn tree_modify<'key, I, F>(
    repo: &Repo,
    parent: Option<Oid>,
    path: &mut I,
    f: F,
) -> Result<(Oid, FileMode), git2::Error>
where
    I: Iterator<Item = &'key str>,
    F: FnOnce(Option<Oid>) -> Result<(Oid, FileMode), git2::Error>,
{
    match path.next() {
        Some(key) => match parent {
            Some(tree_oid) => {
                let tree = repo.find_tree(tree_oid).ok();
                let entry = tree
                    .as_ref()
                    .and_then(|tree| tree.get_name(key).map(|entry| entry.id()));

                let (oid, mode) = tree_modify(repo, entry, path, f)?;

                let mut builder = repo.treebuilder(tree.as_ref())?;
                builder.insert(key, oid, mode.into())?;
                let tree_id = builder.write()?;

                Ok((tree_id, FileMode::Tree))
            }

            None => {
                let (oid, mode) = tree_modify(repo, None, path, f)?;

                let mut builder = repo.treebuilder(None)?;
                builder.insert(key, oid, mode.into())?;
                let tree_id = builder.write()?;

                Ok((tree_id, FileMode::Tree))
            }
        },

        None => f(parent),
    }
}

fn tree_find<'key, I>(repo: &Repo, parent: Oid, path: &mut I) -> Result<Oid, git2::Error>
where
    I: Iterator<Item = &'key str>,
{
    path.next()
        .map(|key| {
            repo.find_tree(parent)?
                .get_name(key)
                .map(|entry| tree_find(repo, entry.id(), path))
                .unwrap_or(Err(git2::Error::new(
                    git2::ErrorCode::NotFound,
                    git2::ErrorClass::Tree,
                    format!("Did not find path element {key}"),
                )))
        })
        .unwrap_or(Ok(parent))
}

pub struct Store<'a> {
    repo: &'a Repo,
    parent: Option<Commit<'a>>,
    tree: git2::Tree<'a>,
}

const UNSTACKED_STORE_PATH: &str = "unstacked/store";

impl<'a> Store<'a> {
    fn new(repo: &'a Repo) -> Result<Self, git2::Error> {
        let tree = repo.find_tree(repo.treebuilder(None)?.write()?)?;
        Ok(Self {
            repo,
            parent: None,
            tree,
        })
    }

    fn from_commit(repo: &'a Repo, commit: Commit<'a>) -> Result<Self, git2::Error> {
        let kv = commit.tree()?;
        Ok(Self {
            repo,
            parent: Some(commit),
            tree: kv,
        })
    }

    fn find_commit(repo: &Repo) -> Result<Option<git2::Commit>, git2::Error> {
        match repo.find_reference(UNSTACKED_STORE_PATH) {
            Ok(refer) => refer.peel_to_commit().map(Some),

            Err(err) => {
                if err.code() == git2::ErrorCode::NotFound {
                    Ok(None)
                } else {
                    Err(err)?
                }
            }
        }
    }

    pub fn open(repo: &'a Repo) -> Result<Self, git2::Error> {
        match Self::find_commit(repo)? {
            Some(commit) => Self::from_commit(repo, commit.into()),
            None => Self::new(repo),
        }
    }

    pub fn write(&mut self) -> Result<(), repo::Error> {
        let sig = self.repo.signature()?;
        let new_parent = self.repo.commit(
            &sig,
            &sig,
            "Update Unstacked Store",
            &self.tree,
            self.parent
                .as_ref()
                .map(|parent| vec![parent])
                .unwrap_or(Vec::new()),
        )?;

        self.repo.reference(
            UNSTACKED_STORE_PATH,
            new_parent.id(),
            true,
            "Update Unstacked Store ref",
        )?;

        self.parent = Some(new_parent);

        Ok(())
    }

    fn get_blob<'key>(
        &self,
        path: impl IntoIterator<Item = &'key str>,
    ) -> Result<Blob<'a>, state::Error> {
        let mut iter = path.into_iter();
        let oid = tree_find(self.repo, self.tree.id(), &mut iter)?;
        let blob = self.repo.find_blob(oid)?;
        Ok(blob)
    }

    pub fn get<'key, T: for<'de> Deserialize<'de>>(
        &self,
        path: impl IntoIterator<Item = &'key str>,
    ) -> Result<T, state::Error> {
        let blob = self.get_blob(path)?;
        let value = serde_json::de::from_slice(blob.content())?;
        Ok(value)
    }

    fn put_oid<'key>(
        &mut self,
        path: impl IntoIterator<Item = &'key str>,
        oid: Oid,
    ) -> Result<(), git2::Error> {
        let (new_tree_id, mode) = tree_modify(
            self.repo,
            Some(self.tree.id()),
            &mut path.into_iter(),
            |_parent| Ok((oid, FileMode::Blob)),
        )?;

        match mode {
            FileMode::Tree => {}
            mode => Err(git2::Error::new(git2::ErrorCode::Invalid, git2::ErrorClass::Tree, format!("Attempt to write {mode:?} to store is not allowed (potentially empty path into KV store?)")))?,
        }

        self.tree = self.repo.find_tree(new_tree_id)?;

        Ok(())
    }

    pub fn put<'key, T: Serialize>(
        &mut self,
        path: impl IntoIterator<Item = &'key str>,
        value: &T,
    ) -> Result<(), state::Error> {
        let data = serde_json::ser::to_vec_pretty(value)?;
        let blob = self.repo.blob(data.as_slice())?;
        Ok(self.put_oid(path, blob)?)
    }
}

#[cfg(test)]
mod tests {
    use crate::repo::Repo;

    use super::Store;

    #[test]
    fn read_write_simple() {
        let (repo, _temp_dir) = Repo::temporary();
        let mut kv = Store::new(&repo).expect("Failed to create KV store");

        kv.put(["foo"], &1337u64).unwrap();
        assert_eq!(kv.get::<u64>(["foo"]).unwrap(), 1337u64);
    }

    #[test]
    fn read_write_nested() {
        let (repo, _temp_dir) = Repo::temporary();
        let mut kv = Store::new(&repo).expect("Failed to create KV store");

        kv.put(["foo"], &1337u64).unwrap();
        kv.put(["foo", "bar"], &"Hello World").unwrap();

        assert!(kv.get::<u64>(["foo"]).is_err());
        assert_eq!(kv.get::<String>(["foo", "bar"]).unwrap(), "Hello World");
    }

    #[test]
    fn read_write_override_nested() {
        let (repo, _temp_dir) = Repo::temporary();
        let mut kv = Store::new(&repo).expect("Failed to create KV store");

        kv.put(["foo", "bar"], &"Hello World").unwrap();
        kv.put(["foo"], &1337u64).unwrap();

        assert!(kv.get::<String>(["foo", "bar"]).is_err());
        assert_eq!(kv.get::<u64>(["foo"]).unwrap(), 1337u64);
    }
}
