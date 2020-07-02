#![feature(exact_size_is_empty)]

use pathsearch::find_executable_in_path;
use std::fs;
use git2::{Repository, IndexAddOption, FetchOptions, RemoteCallbacks, Remote, PushOptions};
use git2_credentials::CredentialHandler;
use eventual::{Timer};
use std::time::Duration;

// #[derive(Debug, Deserialize, Clone, Copy)]
#[derive(serde::Deserialize)]
pub struct Config {
    interval_minutes: u32,
    repo_path: String,
    branch_name: String,
}

fn commit(repo: &Repository) -> Result<(), ()> {
    println!("starting commit...");
    
    let head = repo.head().unwrap();
    let parent_commit = head.peel_to_commit().unwrap();

    let mut index = repo.index().unwrap();
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None).unwrap();

    let diff = repo.diff_tree_to_index(Some(&parent_commit.tree().unwrap()), Some(&index), None).unwrap();

    if diff.deltas().is_empty() {
        println!("empty tree. skipping...");
        return Ok(());
    }

    let tree_oid = index.write_tree().unwrap();
    let tree = repo.find_tree(tree_oid).unwrap();

    let signature = repo.signature().unwrap();
    let commit_oid = repo.commit(head.name(), &signature, &signature, "sync", &tree, &[&parent_commit]).unwrap();
    println!("commit oid: {}", commit_oid.to_string());

    index.write().unwrap();

    Ok(())
}

fn get_remote(repo: &Repository) -> (Remote, RemoteCallbacks) {
    let remote = repo.find_remote("origin").unwrap();

    let mut remote_callbacks = RemoteCallbacks::new();
    let git_config = git2::Config::open_default().unwrap();
    let mut credential_handler = CredentialHandler::new(git_config);
    remote_callbacks.credentials(move |url, username, allowed|
        credential_handler.try_next_credential(url, username, allowed)
    );
    remote_callbacks.push_update_reference(move |name, status| {
        println!("ref pushed. name: {}; status: {:?}", name, status);
        Ok(())
    });
    (remote, remote_callbacks)
}

fn pull(repo: &Repository, branch_name: &str) -> Result<(), &'static str> {
    println!("starting pull...");
    
    let (mut remote, remote_callbacks) = get_remote(repo);

    let mut fetch_options = FetchOptions::new();
    fetch_options
        .remote_callbacks(remote_callbacks)
        .update_fetchhead(true);

    remote.fetch(&[branch_name], Some(&mut fetch_options), None).unwrap();
    
    let remote_ref_name = "refs/remotes/origin/".to_owned() + branch_name;
    let remote_ref = repo.find_reference(remote_ref_name.as_str()).unwrap();
    let remote_commit_ann = repo.reference_to_annotated_commit(&remote_ref).unwrap();
    
    let (analysis, _) = repo.merge_analysis(&[&remote_commit_ann]).unwrap();
    
    if analysis.is_up_to_date() {
        println!("up to date. skipping...");
        return Ok(())
    }
    
    if analysis.is_fast_forward() || analysis.is_normal() {
        println!("merging...");

        let head = repo.head().unwrap();
        let parent_commit = head.peel_to_commit().unwrap();
        
        repo.merge(&[&remote_commit_ann], None, None).unwrap();
        
        let mut index = repo.index().unwrap();
        if index.has_conflicts() {
            for conflict in index.conflicts().unwrap() {
                println!("conflict: {}", String::from_utf8(conflict.unwrap().their.unwrap().path).unwrap());
            }
            return Err("aborting: conflicts found")
        }
        
        let diff = repo.diff_tree_to_index(Some(&parent_commit.tree().unwrap()), Some(&index), None).unwrap();

        if diff.deltas().is_empty() {
            println!("empty tree. skipping...");
            return Ok(());
        }
        
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        
        let signature = repo.signature().unwrap();
        
        let commit_oid = 
            repo.commit(
                head.name(), 
                &signature, 
                &signature, 
                "merge", 
                &tree, 
                &[&parent_commit, &remote_ref.peel_to_commit().unwrap()])
            .unwrap();
        
        println!("commit oid: {}", commit_oid.to_string());
        
        repo.cleanup_state().unwrap();
            
        return Ok(());
    }

    return Err("Unknown merge analysis result");
}

fn push(repo: &Repository, branch_name: &str) -> Result<(), &'static str> {
    println!("starting push...");

    let (mut remote, remote_callbacks) = get_remote(repo);
    
    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(remote_callbacks);

    let head_ref_name = "refs/heads/".to_owned() + branch_name;
    remote.push(&[head_ref_name], Some(&mut push_options)).unwrap();

    return Ok(())
}

async fn run(repo: &Repository, branch_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    commit(&repo).unwrap();
    pull(&repo, branch_name)?;
    push(&repo, branch_name)?;
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let config_path = find_executable_in_path("git-auto-sync.toml").expect("Config file not found");
    let config_bytes = fs::read(config_path).expect("Error reading config file");
    let config: Config = toml::from_slice(&config_bytes).expect("Error parsing config file");

    let repo = Repository::open(config.repo_path).expect("Error opening repository");
    let branch_name = config.branch_name.as_str();

    let interval_ms = config.interval_minutes * 1000 * 60;

    let handled_run = || async {
        let run = async {
            run(&repo, branch_name).await.unwrap_or_else(|e| {
                println!("run error: {}", e);
                // play sound
            })
        };
        
        let timeout_result = tokio::time::timeout(Duration::from_secs((interval_ms / 2) as u64), run).await;
        
        if let Err(_) = timeout_result {
            println!("timed out")
        }
        
        println!("end\n");
    };

    handled_run().await;
    for _ in Timer::new().interval_ms(interval_ms).iter() {
        handled_run().await;
    }

    Ok(())    
}
