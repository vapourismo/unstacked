use crate::rules::{Rule, RuleBook};
use git2::{Error, ErrorClass};
use serde::{Deserialize, Serialize};

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Clone, Serialize, Deserialize)]
pub enum Path {
    SeriesItem { name: String, index: Option<usize> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Side {
    First,
    Last,
}

impl Path {
    pub fn from_rule(
        rules: &RuleBook,
        name: impl AsRef<str>,
        side: Side,
    ) -> Result<Self, git2::Error> {
        let name = name.as_ref();
        match rules.rule(name)? {
            Rule::Series(series) => Ok(Self::SeriesItem {
                name: name.to_owned(),
                index: match side {
                    Side::Last => series.num_patches().checked_sub(1),
                    Side::First => series.has_patches().then_some(0),
                },
            }),

            Rule::Anchor { .. } => Err(Error::new(
                git2::ErrorCode::GenericError,
                ErrorClass::None,
                format!("Can't target anchor rule {name}"),
            )),
        }
    }

    pub fn to_rule_ref(&self) -> String {
        match self {
            Path::SeriesItem { name, .. } => name.clone(),
        }
    }

    pub fn next(&self, rules: &RuleBook) -> Result<Self, git2::Error> {
        Ok(match self {
            Path::SeriesItem { name, index } => match index {
                None => {
                    let dependent_rules = rules.find_rule_use(name);

                    if dependent_rules.is_empty() {
                        self.clone()
                    } else if dependent_rules.len() == 1 {
                        Self::from_rule(rules, dependent_rules[0], Side::First)?
                    } else {
                        return Err(Error::new(
                            git2::ErrorCode::Ambiguous,
                            ErrorClass::None,
                            format!("Series {name} has multiple potential successors"),
                        ));
                    }
                }

                // The first patch to be applied to the parent is at index 0.
                Some(index) => {
                    let series = rules.series(name)?;
                    let next_index = index + 1;

                    if next_index >= series.num_patches() {
                        let dependent_rules = rules.find_rule_use(name);

                        if dependent_rules.is_empty() {
                            self.clone()
                        } else if dependent_rules.len() == 1 {
                            Self::from_rule(rules, &dependent_rules[0], Side::First)?
                        } else {
                            return Err(Error::new(
                                git2::ErrorCode::Ambiguous,
                                ErrorClass::None,
                                format!("Series {name} has multiple potential successors"),
                            ));
                        }
                    } else {
                        Path::SeriesItem {
                            name: name.clone(),
                            index: Some(next_index),
                        }
                    }
                }
            },
        })
    }

    pub fn parent(&self, rules: &RuleBook) -> Result<Self, git2::Error> {
        Ok(match self {
            Path::SeriesItem { name, index } => match index {
                None => {
                    let series = rules.series(name)?;
                    if !series.has_patches() {
                        Self::from_rule(rules, series.parent(), Side::Last)?
                    } else {
                        Path::SeriesItem {
                            name: name.clone(),
                            index: series.num_patches().checked_sub(1),
                        }
                    }
                }

                // The first patch to be applied to the parent is at index 0.
                Some(0) => Self::from_rule(rules, rules.series(name)?.parent(), Side::Last)?,

                Some(index) => Path::SeriesItem {
                    name: name.clone(),
                    index: Some(index - 1),
                },
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::Path;
    use crate::{
        rules::{Rule, RuleBook},
        series,
    };
    use git2::Oid;

    fn blob(content: &str) -> Oid {
        Oid::hash_object(git2::ObjectType::Blob, content.as_bytes()).unwrap()
    }

    #[test]
    fn test_from_rule_nonempty() {
        let mut series = series::Series::new("bogus".to_owned());

        series.push_patch(blob("Patch 1"));
        series.push_patch(blob("Patch 2"));
        series.push_patch(blob("Patch 3"));

        let mut rules = RuleBook::new();
        rules.set_rule("series1".to_owned(), Rule::Series(series));

        let path = Path::from_rule(&rules, &"series1".to_owned(), super::Side::First).unwrap();

        assert_eq!(
            path,
            Path::SeriesItem {
                name: "series1".to_string(),
                index: Some(0)
            }
        );

        let path = Path::from_rule(&rules, &"series1".to_owned(), super::Side::Last).unwrap();

        assert_eq!(
            path,
            Path::SeriesItem {
                name: "series1".to_string(),
                index: Some(2)
            }
        );
    }

    #[test]
    fn test_from_rule_empty() {
        let series = series::Series::new("bogus".to_owned());

        let mut rules = RuleBook::new();
        rules.set_rule("series1".to_owned(), Rule::Series(series));

        let path = Path::from_rule(&rules, &"series1".to_owned(), super::Side::First).unwrap();

        assert_eq!(
            path,
            Path::SeriesItem {
                name: "series1".to_string(),
                index: None
            }
        );

        let path = Path::from_rule(&rules, &"series1".to_owned(), super::Side::Last).unwrap();

        assert_eq!(
            path,
            Path::SeriesItem {
                name: "series1".to_string(),
                index: None
            }
        );
    }

    #[test]
    fn empty_empty() {
        let series1 = series::Series::new("series2".to_owned());
        let series2 = series::Series::new("bogus1".to_owned());

        let mut rules = RuleBook::new();
        rules.set_rule("series1".to_owned(), Rule::Series(series1));
        rules.set_rule("series2".to_owned(), Rule::Series(series2));

        let path = Path::SeriesItem {
            name: "series1".to_owned(),
            index: None,
        };

        assert_eq!(path.next(&rules), Ok(path.clone()));

        assert_eq!(
            path.parent(&rules),
            Ok(Path::SeriesItem {
                name: "series2".to_string(),
                index: None
            })
        );
    }

    #[test]
    fn empty_nonempty() {
        let series1 = series::Series::new("series2".to_owned());

        let mut series2 = series::Series::new("bogus1".to_owned());
        series2.push_patch(blob("Patch 1"));
        series2.push_patch(blob("Patch 2"));
        series2.push_patch(blob("Patch 3"));

        let mut rules = RuleBook::new();
        rules.set_rule("series1".to_owned(), Rule::Series(series1));
        rules.set_rule("series2".to_owned(), Rule::Series(series2));

        let path = Path::SeriesItem {
            name: "series1".to_owned(),
            index: None,
        };

        assert_eq!(path.next(&rules), Ok(path.clone()));

        assert_eq!(
            path.parent(&rules),
            Ok(Path::SeriesItem {
                name: "series2".to_string(),
                index: Some(2)
            })
        );

        let path = Path::SeriesItem {
            name: "series2".to_owned(),
            index: Some(2),
        };

        assert_eq!(
            path.next(&rules),
            Ok(Path::SeriesItem {
                name: "series1".to_owned(),
                index: None
            })
        );

        assert_eq!(
            path.parent(&rules),
            Ok(Path::SeriesItem {
                name: "series2".to_owned(),
                index: Some(1)
            })
        );

        let path = Path::SeriesItem {
            name: "series2".to_owned(),
            index: Some(0),
        };

        assert!(path.parent(&rules).is_err());
    }

    #[test]
    fn nonempty_nonempty() {
        let mut series1 = series::Series::new("series2".to_owned());
        series1.push_patch(blob("Patch 1"));
        series1.push_patch(blob("Patch 2"));
        series1.push_patch(blob("Patch 3"));

        let mut series2 = series::Series::new("bogus1".to_owned());
        series2.push_patch(blob("Patch 1"));
        series2.push_patch(blob("Patch 2"));
        series2.push_patch(blob("Patch 3"));

        let mut rules = RuleBook::new();
        rules.set_rule("series1".to_owned(), Rule::Series(series1));
        rules.set_rule("series2".to_owned(), Rule::Series(series2));

        let path = Path::SeriesItem {
            name: "series1".to_owned(),
            index: Some(2),
        };

        assert_eq!(path.next(&rules), Ok(path.clone()));

        assert_eq!(
            path.parent(&rules),
            Ok(Path::SeriesItem {
                name: "series1".to_string(),
                index: Some(1)
            })
        );

        let path = Path::SeriesItem {
            name: "series1".to_owned(),
            index: Some(0),
        };

        assert_eq!(
            path.next(&rules),
            Ok(Path::SeriesItem {
                name: "series1".to_owned(),
                index: Some(1)
            })
        );

        assert_eq!(
            path.parent(&rules),
            Ok(Path::SeriesItem {
                name: "series2".to_owned(),
                index: Some(2)
            })
        );

        let path = Path::SeriesItem {
            name: "series2".to_owned(),
            index: Some(2),
        };

        assert_eq!(
            path.next(&rules),
            Ok(Path::SeriesItem {
                name: "series1".to_owned(),
                index: Some(0)
            })
        );
    }
}
