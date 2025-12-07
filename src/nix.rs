use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde_json::Value;
use std::process::Stdio;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::debug;

pub async fn get_hosts(flake_path: &str) -> Result<Vec<String>> {
    let expr = format!(
        "let f = builtins.getFlake (toString {}); in builtins.concatStringsSep \" \" (builtins.attrNames f.nixosConfigurations)",
        flake_path
    );

    let output = run_nix(&["eval", "--impure", "--raw", "--expr", &expr]).await?;
    Ok(output.split_whitespace().map(String::from).collect())
}

pub async fn get_system_arch(flake_path: &str, host: &str) -> Result<String> {
    let attr = format!(
        "{}#nixosConfigurations.{}.config.nixpkgs.system",
        flake_path, host
    );
    run_nix(&["eval", "--raw", &attr]).await
}

pub async fn get_drv_path(flake_path: &str, host: &str) -> Result<String> {
    let attr = format!(
        "{}#nixosConfigurations.{}.config.system.build.toplevel.drvPath",
        flake_path, host
    );
    run_nix(&["eval", "--raw", &attr]).await
}

pub async fn build_system(
    flake_path: &str,
    host: &str,
    cores: Option<u32>,
    builders: Option<&str>,
    log_file: &mut File,
) -> Result<String> {
    let attr = format!(
        "{}#nixosConfigurations.{}.config.system.build.toplevel",
        flake_path, host
    );
    let mut args = vec!["build", &attr, "--print-out-paths", "--print-build-logs"];

    let cores_str = cores.map(|c| c.to_string());
    if let Some(ref c) = cores_str {
        args.push("--cores");
        args.push(c);
    }

    if let Some(b) = builders {
        args.push("--builders");
        args.push(b);
    }

    run_nix_with_logging(&args, log_file).await
}

pub async fn copy_to_cache(paths: &[String], cache_dir: &str, secret_key_file: Option<&str>, log_file: &mut File) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }
    let mut dest = format!("file://{}", cache_dir);
    if let Some(key_file) = secret_key_file {
        dest = format!("{}?secret-key={}", dest, key_file);
    }
    
    // Chunk paths to avoid ARG_MAX issues
    const CHUNK_SIZE: usize = 1000;
    for chunk in paths.chunks(CHUNK_SIZE) {
        let mut args = vec!["copy", "--to", &dest];
        args.extend(chunk.iter().map(|s| s.as_str()));
        run_nix_with_logging(&args, log_file).await?;
    }
    Ok(())
}
pub async fn query_requisites(path: &str) -> Result<Vec<String>> {
    let output = run_nix_store(&["--query", "--requisites", "--include-outputs", path]).await?;
    Ok(output.lines().map(String::from).collect())
}

pub async fn realise(paths: &[String], log_file: &mut File) -> Result<()> {
    if paths.is_empty() {
        return Ok(());
    }

    const CHUNK_SIZE: usize = 1000;
    for chunk in paths.chunks(CHUNK_SIZE) {
        let mut args = vec!["--realise"];
        args.extend(chunk.iter().map(|s| s.as_str()));
        run_nix_store_with_logging(&args, log_file).await?;
    }
    Ok(())
}

pub async fn get_derivation_outputs(drv_path: &str) -> Result<Vec<String>> {
    // nix derivation show "$path" | jq -r '.[].outputs[].path'
    let output = run_nix(&["derivation", "show", drv_path]).await?;
    parse_derivation_outputs(&output)
}

fn parse_derivation_outputs(json_output: &str) -> Result<Vec<String>> {
    let json: Value = serde_json::from_str(json_output)?;

    let mut paths = Vec::new();
    if let Some(obj) = json.as_object() {
        for (_drv, details) in obj {
            if let Some(outputs) = details.get("outputs").and_then(|o| o.as_object()) {
                for (_out_name, out_details) in outputs {
                    if let Some(path) = out_details.get("path").and_then(|p| p.as_str()) {
                        paths.push(path.to_string());
                    }
                }
            }
        }
    }
    Ok(paths)
}

pub async fn run_collect_garbage() -> Result<()> {
    run_nix(&["collect-garbage", "-d"]).await?;
    Ok(())
}

async fn run_nix(args: &[&str]) -> Result<String> {
    debug!("Running nix {:?}", args);
    let output = Command::new("nix")
        .args(args)
        .output()
        .await
        .context("Failed to execute nix command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("Nix command failed: {}", stderr));
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn run_nix_store(args: &[&str]) -> Result<String> {
    debug!("Running nix-store {:?}", args);
    let output = Command::new("nix-store")
        .args(args)
        .output()
        .await
        .context("Failed to execute nix-store command")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("nix-store command failed: {}", stderr));
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn run_nix_with_logging(args: &[&str], log_file: &mut File) -> Result<String> {
    debug!("Running nix (logged) {:?}", args);
    run_cmd_logged("nix", args, log_file).await
}

async fn run_nix_store_with_logging(args: &[&str], log_file: &mut File) -> Result<String> {
    debug!("Running nix-store (logged) {:?}", args);
    run_cmd_logged("nix-store", args, log_file).await
}

async fn run_cmd_logged(cmd_name: &str, args: &[&str], log_file: &mut File) -> Result<String> {
    let mut child = Command::new(cmd_name)
        .args(args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(format!("Failed to spawn {}", cmd_name))?;

    let stdout = child.stdout.take().unwrap();
    let stderr = child.stderr.take().unwrap();

    // Stream stderr to log file
    let mut reader = BufReader::new(stderr);
    let mut line = String::new();

    // We need to read stdout separately. Since we want to return it, we can read it to string at the end or async.
    // However, `stdout` pipe might fill up if we don't read it while reading stderr.
    // So we should use `tokio::join!` or spawn a task for stdout.

    // Spawning a task for stdout reading is safer to prevent deadlocks if output is huge.
    // But `nix build --print-out-paths` stdout is small.
    // `nix-store --realise` might output paths too.

    // Let's assume stdout is relatively small but stderr is large (logs).
    // To be safe, let's read stdout in a separate task or join.

    let stdout_handle = tokio::spawn(async move {
        let mut reader = BufReader::new(stdout);
        let mut out = String::new();
        reader.read_to_string(&mut out).await?;
        Ok::<String, std::io::Error>(out)
    });

    loop {
        line.clear();
        let n = reader.read_line(&mut line).await?;
        if n == 0 {
            break;
        }
        let timestamp = Local::now().format("%F_%H-%M-%S.%3f");
        let stamped_line = format!("[{}] {}", timestamp, line);
        log_file.write_all(stamped_line.as_bytes()).await?;
    }

    let status = child.wait().await?;
    let stdout_result = stdout_handle.await??;

    if !status.success() {
        return Err(anyhow!("Command {} failed", cmd_name));
    }

    Ok(stdout_result.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_derivation_outputs() {
        let json = r#"{ 
            \"/nix/store/z...-foo.drv\": { 
                \"outputs\": { 
                    \"out\": { \"path\": \"/nix/store/a...-foo\" }, 
                    \"dev\": { \"path\": \"/nix/store/b...-foo-dev\" } 
                } 
            } 
        }"#;
        let paths = parse_derivation_outputs(json).unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.contains(&"/nix/store/a...-foo".to_string()));
        assert!(paths.contains(&"/nix/store/b...-foo-dev".to_string()));
    }
}
