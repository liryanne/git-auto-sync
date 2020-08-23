#![feature(exact_size_is_empty)]

use pathsearch::find_executable_in_path;
use std::fs;
use git2::{Repository, IndexAddOption, FetchOptions, RemoteCallbacks, Remote, PushOptions};
use git2_credentials::CredentialHandler;
use eventual::{Timer};
use std::time::Duration;
use chrono::{Local, Timelike, Datelike};
use std::fs::File;
use std::io::BufReader;
use rodio::Source;

// #[derive(Debug, Deserialize, Clone, Copy)]
#[derive(serde::Deserialize)]
pub struct Config {
    interval_minutes: u32,
    repo_path: String,
    branch_name: String,
}

fn commit(repo: &Repository) -> Result<(), git2::Error> {
    println!("starting commit...");

    let head = repo.head()?;
    let parent_commit = head.peel_to_commit()?;

    let mut index = repo.index()?;
    index.add_all(["*"].iter(), IndexAddOption::DEFAULT, None)?;

    let diff = repo.diff_tree_to_index(Some(&parent_commit.tree()?), Some(&index), None)?;

    if diff.deltas().is_empty() {
        println!("empty tree. skipping...");
        return Ok(());
    }

    let tree_oid = index.write_tree()?;
    let tree = repo.find_tree(tree_oid)?;

    let signature = repo.signature()?;
    let commit_oid = repo.commit(head.name(), &signature, &signature, "sync", &tree, &[&parent_commit])?;
    println!("commit oid: {}", commit_oid.to_string());

    index.write()?;

    Ok(())
}

fn get_remote(repo: &Repository) -> Result<(Remote, RemoteCallbacks), git2::Error> {
    let remote = repo.find_remote("origin")?;

    let mut remote_callbacks = RemoteCallbacks::new();
    let git_config = git2::Config::open_default()?;
    let mut credential_handler = CredentialHandler::new(git_config);
    remote_callbacks.credentials(move |url, username, allowed|
        credential_handler.try_next_credential(url, username, allowed)
    );
    remote_callbacks.push_update_reference(move |name, status| {
        println!("ref pushed. name: {}; status: {:?}", name, status);
        Ok(())
    });
    Ok((remote, remote_callbacks))
}

fn pull(repo: &Repository, branch_name: &str) -> Result<(), git2::Error> {
    println!("starting pull...");

    let (mut remote, remote_callbacks) = get_remote(repo)?;

    let mut fetch_options = FetchOptions::new();
    fetch_options
        .remote_callbacks(remote_callbacks)
        .update_fetchhead(true);

    remote.fetch(&[branch_name], Some(&mut fetch_options), None)?;

    let remote_ref_name = "refs/remotes/origin/".to_owned() + branch_name;
    let remote_ref = repo.find_reference(remote_ref_name.as_str())?;
    let remote_commit_ann = repo.reference_to_annotated_commit(&remote_ref)?;

    let (analysis, _) = repo.merge_analysis(&[&remote_commit_ann])?;

    if analysis.is_up_to_date() {
        println!("up to date. skipping...");
        return Ok(());
    }

    if analysis.is_fast_forward() || analysis.is_normal() {
        println!("merging...");

        let head = repo.head()?;
        let parent_commit = head.peel_to_commit()?;

        repo.merge(&[&remote_commit_ann], None, None)?;

        let mut index = repo.index()?;
        if index.has_conflicts() {
            for conflict in index.conflicts()? {
                let conflict_path =
                    conflict?
                        .their
                        .map(|index_entry| index_entry.path)
                        .unwrap_or(Vec::from("<error_no_conflict>"));

                println!("conflict: {}",
                         String::from_utf8(conflict_path)
                             .unwrap_or(String::from("<conflict_invalid_path>")));
            }
            return Err(git2::Error::from_str("aborting: conflicts found"));
        }

        let diff = repo.diff_tree_to_index(Some(&parent_commit.tree()?), Some(&index), None)?;

        if diff.deltas().is_empty() {
            println!("empty tree. skipping...");
            return Ok(());
        }

        let tree_oid = index.write_tree()?;
        let tree = repo.find_tree(tree_oid)?;

        let signature = repo.signature()?;

        let commit_oid =
            repo.commit(
                head.name(),
                &signature,
                &signature,
                "merge",
                &tree,
                &[&parent_commit, &remote_ref.peel_to_commit()?])?;

        println!("commit oid: {}", commit_oid.to_string());

        repo.cleanup_state()?;

        return Ok(());
    }

    return Err(git2::Error::from_str("Unknown merge analysis result"));
}

fn push(repo: &Repository, branch_name: &str) -> Result<(), git2::Error> {
    println!("starting push...");

    let (mut remote, remote_callbacks) = get_remote(repo)?;

    let mut push_options = PushOptions::new();
    push_options.remote_callbacks(remote_callbacks);

    let head_ref_name = "refs/heads/".to_owned() + branch_name;
    remote.push(&[head_ref_name], Some(&mut push_options))?;

    return Ok(());
}

async fn run(repo: &Repository, branch_name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let now = Local::now();
    println!("{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
             now.day(),
             now.month(),
             now.year(),
             now.hour(),
             now.minute(),
             now.second());

    commit(&repo)?;
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

                if let Ok(mut dir) = std::env::current_dir() {
                    if dir.ends_with(r"\debug") {
                        dir.push(r"\..\..");
                    }

                    let wav_path = dir.join("assets").join("error.wav");
                    let file = File::open(wav_path).unwrap();

                    let device = rodio::default_output_device().unwrap();
                    let source = rodio::Decoder::new(BufReader::new(file)).unwrap();
                    rodio::play_raw(&device, source.convert_samples());
                }
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
