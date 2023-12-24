use core::fmt;
use git2::{Diff, Patch};
use termion::color::{Cyan, Fg, Green, Red, Reset};

enum DiffLine {
    HunkStart { offset: String, line: String },
    Deletion(String),
    Addition(String),
    Other(String),
}

impl DiffLine {
    fn new(body: &str) -> Self {
        if body.starts_with('+') {
            Self::Addition(body[1..].to_string())
        } else if body.starts_with('-') {
            Self::Deletion(body[1..].to_string())
        } else if body.starts_with("@@") {
            match body[2..].split_once("@@") {
                Some((offset, line)) => Self::HunkStart {
                    offset: offset.to_string(),
                    line: line.to_string(),
                },
                None => Self::Other(body.to_string()),
            }
        } else {
            Self::Other(body.to_string())
        }
    }
}

impl fmt::Display for DiffLine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DiffLine::HunkStart { offset, line } => {
                write!(f, "{}@@{offset}@@{}{line}", Fg(Cyan), Fg(Reset))
            }
            DiffLine::Deletion(line) => write!(f, "{}-{line}{}", Fg(Red), Fg(Reset)),
            DiffLine::Addition(line) => write!(f, "{}+{line}{}", Fg(Green), Fg(Reset)),
            DiffLine::Other(line) => line.fmt(f),
        }
    }
}

pub struct PrettyPatch {
    lines: Vec<DiffLine>,
}

impl PrettyPatch {
    pub fn new(patch: &mut Patch) -> Result<Self, git2::Error> {
        let buffer = patch.to_buf()?;
        let lines = buffer
            .as_str()
            .unwrap_or("")
            .lines()
            .map(DiffLine::new)
            .collect::<Vec<_>>();
        Ok(Self { lines })
    }
}

impl fmt::Display for PrettyPatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in self.lines.iter() {
            writeln!(f, "{}", line)?;
        }

        Ok(())
    }
}

pub struct PrettyDiff {
    patches: Vec<PrettyPatch>,
}

impl PrettyDiff {
    pub fn new(diff: &Diff) -> Result<Self, git2::Error> {
        let stats = diff.stats()?;
        let patches = (0..stats.files_changed())
            .filter_map(|index| {
                let mut patch = git2::Patch::from_diff(&diff, index).ok()??;
                PrettyPatch::new(&mut patch).ok()
            })
            .collect::<Vec<_>>();

        Ok(Self { patches })
    }
}

impl fmt::Display for PrettyDiff {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for patch in self.patches.iter() {
            writeln!(f, "{}", patch)?;
        }

        Ok(())
    }
}

pub fn render(diff: &Diff) -> Result<String, git2::Error> {
    let stats = diff.stats()?;

    let buffers = (0..stats.files_changed())
        .map(|index| -> Result<String, git2::Error> {
            let mut patch = git2::Patch::from_diff(&diff, index)?.expect("Patch");
            let buf = patch.to_buf()?;
            Ok(buf.as_str().expect("Patch is not valid UTF-8").to_string())
        })
        .collect::<Result<Vec<_>, _>>()?
        .join("\n");

    Ok(buffers)
}
