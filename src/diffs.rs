use git2::Diff;

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
