//! Publish the locally built binary as the `current` jcode build.
//!
//! Runs the same validated path `selfdev reload` uses: fingerprint check against
//! the working tree, install under a version label, smoke-test the installed
//! binary, then atomically swap the `current` symlink. Existing versions are
//! retained, so rolling back is another symlink swap.
//!
//! Usage: cargo run -p jcode-build-support --example publish_current

fn main() -> anyhow::Result<()> {
    let repo_dir = std::env::current_dir()?;
    let source = jcode_build_support::current_source_state(&repo_dir)?;
    let previous = jcode_build_support::read_current_version()?;

    println!("repo:     {}", repo_dir.display());
    println!("version:  {}", source.version_label);
    println!("dirty:    {}", source.dirty);
    println!("previous: {previous:?}");

    let published = jcode_build_support::publish_local_current_build_for_source(&repo_dir, &source)?;

    println!("published {} -> {}", published.version, published.versioned_path.display());
    println!("rollback: previous current was {previous:?}");
    Ok(())
}
