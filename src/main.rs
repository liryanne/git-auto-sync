use pathsearch::find_executable_in_path;
use std::fs;
use git2::{Repository, IndexAddOption};


// #[derive(Debug, Deserialize, Clone, Copy)]
#[derive(serde::Deserialize)]
pub struct Config {
    // interval_minutes: u8,
    repo_path: String,
}

fn sync_repo(path: String) -> Result<(), ()> {
    let repo = Repository::open(path).expect("Error opening repository");
    
    let mut index = repo.index().unwrap();
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).unwrap();
    // index.write().unwrap();
    let tree_oid = index.write_tree().unwrap();
    
    let signature = repo.signature().unwrap();
    
    let tree = repo.find_tree(tree_oid).unwrap();
    
    repo.commit(None, &signature, &signature, "sync", &tree, &[]).unwrap();
    // let oid = repo.commit_signed("content", "signature", None).unwrap();
    
    Ok(())
}

fn run() -> Result<(), ()> {
    let config_path = find_executable_in_path("git-auto-sync.toml").expect("Config file not found");
    let config_bytes = fs::read(config_path).expect("Error reading config file.");
    let config : Config = toml::from_slice(&config_bytes).expect("Error parsing config file");
    
    sync_repo(config.repo_path)?;
    
    Ok(())
}

fn main() {
    run().unwrap();
}
