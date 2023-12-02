use std::error::Error;

use crate::repo::Repo;
use serde::{Deserialize, Serialize};

pub struct State {
    repo: Repo,
}

impl State {
    pub fn new(repo: Repo) -> Self {
        Self { repo }
    }

    pub fn repo(&self) -> &Repo {
        &self.repo
    }

    fn plan_ref_name(name: impl AsRef<str>) -> String {
        format!("refs/unstacked/plans/{}", name.as_ref())
    }

    pub fn find_plan(&self, name: impl AsRef<str>) -> Result<Plan, Box<dyn Error>> {
        let ref_name = Self::plan_ref_name(name);
        let ref_ = self.repo.0.find_reference(ref_name.as_str())?;
        let blob = ref_.peel_to_blob()?;

        let plan = serde_json::de::from_slice(blob.content())?;
        Ok(plan)
    }

    pub fn save_plan(&self, name: impl AsRef<str>, plan: &Plan) -> Result<(), Box<dyn Error>> {
        let content = serde_json::ser::to_vec(plan)?;
        let oid = self.repo.0.blob(content.as_slice())?;
        let ref_name = Self::plan_ref_name(name);
        self.repo
            .0
            .reference(ref_name.as_str(), oid, true, "Save unstacked plan")?;
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Plan {
    pub base_ref: String,
    pub use_merge_base: bool,
    pub added_refs: Vec<String>,
    pub sign: bool,
}
