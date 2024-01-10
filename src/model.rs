#![allow(dead_code)]

use crate::{anchor, git_cache::CachedRepo, git_helper, path, rules, series};
use git2::{Diff, Oid, Repository};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, derive_more::Error, derive_more::Display, derive_more::From)]
pub enum Error {
    Git(git2::Error),
    Rule(rules::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Focus {
    path: path::Path,
    #[serde(with = "crate::git_helper::serde::oid")]
    id: Oid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Model {
    rules: rules::RuleBook,
    focus: Option<Focus>,
}

const MODEL_REF: &str = "refs/unstacked/model";

impl Model {
    pub fn new() -> Self {
        Self {
            rules: rules::RuleBook::new(),
            focus: None,
        }
    }

    pub fn load(repo: &Repository) -> Result<Self, Error> {
        let mut model = repo
            .find_reference(MODEL_REF)
            .and_then(|reff| {
                let json = reff.peel_to_blob()?;

                serde_json::de::from_slice(json.content()).map_err(|err| {
                    git2::Error::new(
                        git2::ErrorCode::Invalid,
                        git2::ErrorClass::Invalid,
                        format!("Could not parse model from ref {MODEL_REF}: {err}"),
                    )
                })
            })
            .or_else(|err| {
                if err.code() == git2::ErrorCode::NotFound {
                    Ok(Self::new())
                } else {
                    Err(err)
                }
            })?;

        if let Some(focus) = &model.focus {
            let head = repo.head()?.peel_to_commit()?;
            if head.id() != focus.id {
                model.focus = None;
            }
        }

        Ok(model)
    }

    pub fn save(self, repo: &Repository) -> Result<(), Error> {
        let data = serde_json::ser::to_vec_pretty(&self).map_err(|err| {
            git2::Error::new(
                git2::ErrorCode::Invalid,
                git2::ErrorClass::Invalid,
                format!("Could serialise model: {err}"),
            )
        })?;

        let blob = repo.blob(data.as_slice())?;
        repo.reference(MODEL_REF, blob, true, "")?;

        Ok(())
    }

    pub fn new_series(&mut self, name: &str, parent: String) {
        self.rules.set_rule(
            name.to_owned(),
            rules::Rule::Series(series::Series::new(parent)),
        );
    }

    pub fn new_anchor(&mut self, name: &str, id: Oid) {
        self.rules
            .set_rule(name.to_owned(), rules::Rule::Anchor(anchor::Anchor { id }));
    }

    fn checkout_path(&mut self, cache: &mut CachedRepo, path: &path::Path) -> Result<Oid, Error> {
        let id = self.rules.build_path(cache, path)?;
        let commit = cache.repo().find_commit(id)?;

        git_helper::checkout(cache.repo(), &commit)?;

        Ok(id)
    }

    pub fn goto_next(&mut self, cache: &mut CachedRepo) -> Result<(), Error> {
        let Some(focus) = &self.focus else {
            log::warn!("Can't go to next if no focus is set");
            return Ok(());
        };

        let path = focus.path.next(&self.rules)?;
        let id = self.checkout_path(cache, &path)?;

        let new_focus = Focus { path, id };
        log::debug!("Transitioned to {new_focus:?}");
        self.focus = Some(new_focus);

        Ok(())
    }

    pub fn goto_parent(&mut self, cache: &mut CachedRepo) -> Result<(), Error> {
        let Some(focus) = &self.focus else {
            log::warn!("Can't go to parent if no focus is set");
            return Ok(());
        };

        let path = focus.path.parent(&self.rules)?;
        let id = self.checkout_path(cache, &path)?;

        let new_focus = Focus { path, id };
        log::debug!("Transitioned to {new_focus:?}");
        self.focus = Some(new_focus);

        Ok(())
    }

    pub fn goto_rule(&mut self, cache: &mut CachedRepo, rule: &String) -> Result<(), Error> {
        let path = path::Path::from_rule(&self.rules, rule, path::Side::Last)?;
        let id = self.checkout_path(cache, &path)?;
        self.focus = Some(Focus { path, id });

        Ok(())
    }

    pub fn focus(&self) -> Option<&path::Path> {
        self.focus.as_ref().map(|f| &f.path)
    }

    pub fn focus_rule(&self) -> Option<String> {
        self.focus.as_ref().map(|focus| focus.path.to_rule_ref())
    }

    pub fn staged_diff<'a>(
        &self,
        repo: &'a Repository,
        use_index: bool,
    ) -> Result<Option<Diff<'a>>, Error> {
        let id = match &self.focus {
            Some(focus) => focus.id,
            None => repo.head()?.peel_to_commit()?.id(),
        };

        let tree = git_helper::capture_tree(repo, use_index)?;
        let head = repo.find_commit(id)?;
        let diff = repo.diff_tree_to_tree(Some(&head.tree()?), Some(&tree), None)?;

        Ok(Some(diff))
    }

    pub fn amend_focus(&mut self, cache: &mut CachedRepo, use_index: bool) -> Result<(), Error> {
        let Some(mut focus) = self.focus.clone() else {
            return Ok(());
        };

        match &focus.path {
            path::Path::SeriesItem {
                name,
                index: Some(index),
            } => {
                let id = {
                    let tree = git_helper::capture_tree(cache.repo(), use_index)?;
                    let head = cache.repo().find_commit(focus.id)?;
                    head.amend(None, None, None, None, None, Some(&tree))?
                };
                self.rules.series_mut(name)?.set_patch(*index, id);
            }

            _ => Err(git2::Error::new(
                git2::ErrorCode::Invalid,
                git2::ErrorClass::Invalid,
                "Cannot amend unspecified target into series",
            ))?,
        }

        focus.id = self.checkout_path(cache, &focus.path)?;
        self.focus = Some(focus);

        Ok(())
    }

    pub fn commit_onto_focus(
        &mut self,
        cache: &mut CachedRepo,
        message: impl AsRef<str>,
        use_index: bool,
        sign: bool,
    ) -> Result<(), Error> {
        let Some(mut focus) = self.focus.clone() else {
            return Ok(());
        };

        let id = {
            let tree = git_helper::capture_tree(cache.repo(), use_index)?;
            let head = cache.repo().find_commit(focus.id)?;
            let sig = cache.repo().signature()?;

            if sign {
                git_helper::commit_signed(cache.repo(), &sig, &sig, message, &tree, [&head])?
            } else {
                git_helper::commit(cache.repo(), &sig, &sig, message, &tree, [&head])?
            }
        };

        match &mut focus.path {
            path::Path::SeriesItem { name, index } => {
                let series = self.rules.series_mut(name)?;
                let new_index = index.map(|i| i + 1);
                *index = series.insert_patch(new_index, id);
            }
        }

        focus.id = self.checkout_path(cache, &focus.path)?;
        self.focus = Some(focus);

        Ok(())
    }

    pub fn build(&mut self, cache: &mut CachedRepo, rule: impl AsRef<str>) -> Result<Oid, Error> {
        Ok(self.rules.build(cache, rule)?)
    }

    pub fn build_all(&mut self, cache: &mut CachedRepo) -> Result<HashMap<String, Oid>, Error> {
        Ok(self.rules.build_all(cache)?)
    }
}
