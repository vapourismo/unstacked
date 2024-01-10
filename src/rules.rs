use super::{
    anchor::Anchor,
    path::{self, Path},
    series::{self, Series},
};
use crate::git_cache::CachedRepo;
use git2::{ErrorClass, Oid, Reference, Repository};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    fmt::Display,
};

fn update_rule_ref(
    repo: &Repository,
    name: impl Display,
    id: Oid,
) -> Result<Reference<'_>, git2::Error> {
    repo.reference(format!("refs/unstacked/rule/{name}").as_str(), id, true, "")
}

#[derive(Debug, derive_more::Error, derive_more::Display, derive_more::From)]
pub enum Error {
    Git(git2::Error),

    #[display(fmt = "PatchConflict {path:?}: {base} <- {patch}")]
    PatchConflict {
        path: path::Path,
        base: Oid,
        patch: Oid,
    },
}

impl Error {
    fn from_series_error(name: &str, error: series::Error) -> Self {
        match error {
            series::Error::Git(git_error) => Self::Git(git_error),
            series::Error::Rule(done) => done,
            series::Error::PatchConflict { index, base, patch } => Self::PatchConflict {
                path: path::Path::SeriesItem {
                    name: name.to_owned(),
                    index: Some(index),
                },
                base,
                patch,
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Rule {
    Series(Series),
    Anchor(Anchor),
}

impl Rule {
    fn parent(&self) -> Option<&String> {
        match self {
            Rule::Series(series) => Some(series.parent()),
            Rule::Anchor(_) => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleBook {
    rules: HashMap<String, Rule>,
}

impl RuleBook {
    pub fn new() -> Self {
        Self {
            rules: HashMap::new(),
        }
    }

    pub fn set_rule(&mut self, name: String, rule: Rule) {
        self.rules.insert(name, rule);
    }

    pub fn rule(&self, name: impl AsRef<str>) -> Result<&Rule, git2::Error> {
        let name = name.as_ref();
        self.rules.get(name).ok_or_else(|| {
            git2::Error::new(
                git2::ErrorCode::NotFound,
                ErrorClass::Reference,
                format!("Could not find rule {name}"),
            )
        })
    }

    pub fn rule_mut(&mut self, name: impl AsRef<str>) -> Result<&mut Rule, git2::Error> {
        let name = name.as_ref();
        self.rules.get_mut(name).ok_or_else(|| {
            git2::Error::new(
                git2::ErrorCode::NotFound,
                ErrorClass::Reference,
                format!("Could not find rule {name}"),
            )
        })
    }

    pub fn series(&self, name: impl AsRef<str>) -> Result<&Series, git2::Error> {
        let name = name.as_ref();
        match self.rule(name)? {
            Rule::Series(series) => Ok(series),
            _ => Err(git2::Error::new(
                git2::ErrorCode::NotFound,
                ErrorClass::None,
                format!("Expected {name} to be a series"),
            )),
        }
    }

    pub fn series_mut(&mut self, name: impl AsRef<str>) -> Result<&mut Series, git2::Error> {
        let name = name.as_ref();
        match self.rule_mut(name)? {
            Rule::Series(series) => Ok(series),
            _ => Err(git2::Error::new(
                git2::ErrorCode::NotFound,
                ErrorClass::None,
                format!("Expected {name} to be a series"),
            )),
        }
    }

    pub fn build(&mut self, cache: &mut CachedRepo, name: impl AsRef<str>) -> Result<Oid, Error> {
        let name = name.as_ref();
        let rule = self.rule(name)?.clone();

        let id = match rule {
            Rule::Series(mut series) => {
                let id = series
                    .build(self, cache)
                    .map_err(|err| Error::from_series_error(name, err))?;
                self.rules.insert(name.to_owned(), Rule::Series(series));
                id
            }

            Rule::Anchor(anchor) => anchor.id,
        };

        update_rule_ref(cache.repo(), name, id)?;

        Ok(id)
    }

    pub fn build_path(&mut self, cache: &mut CachedRepo, path: &Path) -> Result<Oid, Error> {
        match path {
            Path::SeriesItem { name, index } => {
                let mut series = self.series(name)?.clone();
                let is_top = series.is_top_patch(*index);
                let id = series
                    .build_at(self, cache, *index)
                    .map_err(|err| Error::from_series_error(name, err))?;

                self.rules.insert(name.clone(), Rule::Series(series));

                if is_top {
                    update_rule_ref(cache.repo(), name, id)?;
                }

                Ok(id)
            }
        }
    }

    pub fn build_all(&mut self, cache: &mut CachedRepo) -> Result<HashMap<String, Oid>, Error> {
        self.rules
            .keys()
            .cloned()
            .collect::<Vec<_>>() // Need to collect in between to deccouple lifetimes
            .into_iter()
            .map(|name| {
                let id = self.build(cache, &name)?;
                Ok((name, id))
            })
            .collect()
    }

    pub fn find_rule_use<T>(&self, name: &T) -> VecDeque<&str>
    where
        String: PartialEq<T>,
    {
        self.rules
            .iter()
            .filter(|(_, rule)| rule.parent().is_some_and(|parent| parent.eq(name)))
            .map(|(name, _)| name.as_str())
            .collect()
    }
}
