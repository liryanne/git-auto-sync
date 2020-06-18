#![feature(exact_size_is_empty)]

use pathsearch::find_executable_in_path;
use std::fs;
use git2::{Repository, IndexAddOption, IntoCString, FetchOptions, RemoteCallbacks, Remote};
use git2::FileMode::Tree;
use git2_credentials::CredentialHandler;


// #[derive(Debug, Deserialize, Clone, Copy)]
#[derive(serde::Deserialize)]
pub struct Config {
    // interval_minutes: u8,
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

fn get_remote(repo: &Repository) -> (Remote, FetchOptions) {
    let remote = repo.find_remote("origin").unwrap();

    let mut remote_callbacks = RemoteCallbacks::new();
    let git_config = git2::Config::open_default().unwrap();
    let mut credential_handler = CredentialHandler::new(git_config);
    remote_callbacks.credentials(move |url, username, allowed|
        credential_handler.try_next_credential(url, username, allowed)
    );
    remote_callbacks.push_update_reference(move |name, status| {
        println!("name: {}; status: {:?}", name, status);
        Ok(())
    });

    let mut fetch_options = FetchOptions::new();
    fetch_options.remote_callbacks(remote_callbacks);
    
    (remote, fetch_options)
}

fn pull(repo: &Repository, branch_name: &str) -> Result<(), &'static str> {
    println!("starting pull...");
    
    let (mut remote, mut fetch_options) = get_remote(repo);

    remote.fetch(&[branch_name], Some(&mut fetch_options), None).unwrap();
    
    let remote_ref_name = "refs/remotes/origin/".to_owned() + branch_name;
    let remote_ref = repo.find_reference(remote_ref_name.as_str()).unwrap();
    let remote_commit_ann = repo.reference_to_annotated_commit(&remote_ref).unwrap();
    
    let (analysis, _) = repo.merge_analysis(&[&remote_commit_ann]).unwrap();
    
    if analysis.is_up_to_date() {
        println!("up to date. skipping...");
        return Ok(())
    }
    
    if analysis.is_fast_forward() {
        println!("fast forward");
        
        let remote_ref_oid = remote_ref.target().unwrap();
        let remote_tree = repo.find_tree(remote_ref_oid).unwrap();
        repo.checkout_tree(&remote_tree.into_object(), None);
        
        let head_ref_name = "refs/heads/".to_owned() + branch_name;
        let mut head_ref = repo.find_reference(head_ref_name.as_str()).unwrap();
        
        match head_ref.set_target(remote_ref_oid, &"") {
            Err(e) => {
                println!("error setting target: {}", e.message());
                repo.branch_from_annotated_commit(branch_name, &remote_commit_ann, false);
            },
            Ok(_reference) => ()
        }
        
        repo.head().unwrap().set_target(remote_ref_oid, &"");

        return Ok(())
    }
    
    if analysis.is_normal() {
        repo.merge(&[&remote_commit_ann], None, None);
        
        let mut index = repo.index().unwrap();
        if index.has_conflicts() {
            for conflict in index.conflicts().unwrap() {
                println!("conflict: {}", String::from_utf8(conflict.unwrap().their.unwrap().path).unwrap());
            }
            return Err("aborting: conflicts found")
        }
        
        let tree_oid = index.write_tree().unwrap();
        let tree = repo.find_tree(tree_oid).unwrap();
        
        let signature = repo.signature().unwrap();
        
        let head = repo.head().unwrap();
        let commit_oid = 
            repo.commit(
                head.name(), 
                &signature, 
                &signature, 
                "merge", 
                &tree, 
                &[&head.peel_to_commit().unwrap(), &remote_ref.peel_to_commit().unwrap()])
            .unwrap();
        
        println!("commit oid: {}", commit_oid.to_string());
        
        repo.cleanup_state().unwrap();
            
        return Ok(());
    }

    return Err("Unknown merge analysis result");
}

fn push(repo: &Repository) -> Result<(), &'static str> {
    println!("starting push...");

    let (mut remote, _) = get_remote(repo);
    
    remote.push(&[String::from("")], None).unwrap();

    return Ok(())
}

fn run() -> Result<(), &'static str> {
    let config_path = find_executable_in_path("git-auto-sync.toml").expect("Config file not found");
    let config_bytes = fs::read(config_path).expect("Error reading config file.");
    let config: Config = toml::from_slice(&config_bytes).expect("Error parsing config file");

    let repo = Repository::open(config.repo_path).expect("Error opening repository");

    commit(&repo).unwrap();
    pull(&repo, config.branch_name.as_str())?;
    push(&repo)?;

    println!("end");
    Ok(())
}

fn main() {
    run().unwrap();
}
