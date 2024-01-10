use git2::Oid;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Anchor {
    #[serde(with = "crate::git_helper::serde::oid")]
    pub id: Oid,
}
