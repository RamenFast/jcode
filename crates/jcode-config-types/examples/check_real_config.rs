//! Parse the user's real ~/.jcode/config.toml `[compaction]` section.
//!
//! Guards the upgrade path: a config written before `keep_fraction` and
//! `min_keep_turns` existed must still load and pick up the defaults.
//!
//! Usage: cargo run -p jcode-config-types --example check_real_config

#[derive(serde::Deserialize)]
struct Probe {
    #[serde(default)]
    compaction: jcode_config_types::CompactionConfig,
}

fn main() {
    let path = std::env::var("HOME").unwrap() + "/.jcode/config.toml";
    let text = std::fs::read_to_string(&path).expect("read config");
    let probe: Probe = toml::from_str(&text).expect("real config must parse");
    println!("keep_fraction  = {}", probe.compaction.keep_fraction);
    println!("min_keep_turns = {}", probe.compaction.min_keep_turns);
    println!("mode           = {:?}", probe.compaction.mode);
}
