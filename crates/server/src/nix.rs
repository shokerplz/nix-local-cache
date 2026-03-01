use anyhow::{anyhow, Context, Result};
use chrono::Local;
use serde_json::Value;
use std::process::Stdio;
use tokio::fs::File;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tracing::debug;

#[derive(Clone)]
pub struct NixOps;

impl NixOps {
    pub async fn get_hosts(&self, flake_path: &str) -> Result<Vec<String>> {
        let expr = format!(
            "let f = builtins.getFlake (toString {}); in builtins.concatStringsSep \" \" (builtins.attrNames f.nixosConfigurations)",
            flake_path
        );

        let output = run_nix(&["eval", "--impure", "--raw", "--expr", &expr]).await?;
        Ok(output.split_whitespace().map(String::from).collect())
    }

    pub async fn get_system_arch(&self, flake_path: &str, host: &str) -> Result<String> {
        let attr = format!(
            "{}#nixosConfigurations.{}.config.nixpkgs.system",
            flake_path, host
        );
        run_nix(&["eval", "--raw", &attr]).await
    }

    pub async fn get_drv_path(&self, flake_path: &str, host: &str) -> Result<String> {
        let attr = format!(
            "{}#nixosConfigurations.{}.config.system.build.toplevel.drvPath",
            flake_path, host
        );
        run_nix(&["eval", "--raw", &attr]).await
    }

    pub async fn build_system(
        &self,
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

    pub async fn copy_to_cache(
        &self,
        paths: &[String],
        cache_dir: &str,
        secret_key_file: Option<&str>,
        log_file: &mut File,
    ) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut dest = format!("file://{}", cache_dir);
        if let Some(key_file) = secret_key_file {
            dest = format!("{}?secret-key={}", dest, key_file);
        }

        // Chunk paths to avoid ARG_MAX issues
        const CHUNK_SIZE: usize = 10;
        for chunk in paths.chunks(CHUNK_SIZE) {
            let mut args = vec!["copy", "--to", &dest];
            args.extend(chunk.iter().map(|s| s.as_str()));
            run_nix_with_logging(&args, log_file).await?;
        }
        Ok(())
    }

    pub async fn query_requisites(&self, path: &str) -> Result<Vec<String>> {
        let output = run_nix_store(&["--query", "--requisites", "--include-outputs", path])
            .await
            .context(format!("Failed to query requisites for path: {}", path))?;
        Ok(output.lines().map(String::from).collect())
    }

    pub async fn realise(&self, paths: &[String], log_file: &mut File) -> Result<()> {
        if paths.is_empty() {
            return Ok(());
        }

        // Chunk paths to avoid ARG_MAX issues
        const CHUNK_SIZE: usize = 10;
        for chunk in paths.chunks(CHUNK_SIZE) {
            let mut args = vec!["--realise"];
            args.extend(chunk.iter().map(|s| s.as_str()));
            run_nix_store_with_logging(&args, log_file).await?;
        }
        Ok(())
    }

    pub async fn get_derivation_outputs(&self, drv_path: &str) -> Result<Vec<String>> {
        let output = run_nix(&["derivation", "show", drv_path])
            .await
            .context(format!(
                "Failed to get derivation outputs for: {}",
                drv_path
            ))?;
        parse_derivation_outputs(&output).context(format!(
            "Failed to parse derivation outputs for: {}",
            drv_path
        ))
    }
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

async fn run_nix(args: &[&str]) -> Result<String> {
    debug!("Running nix {:?}", args);
    let output = Command::new("nix")
        .args(args)
        .output()
        .await
        .context(format!(
            "Failed to execute nix command: nix {}",
            args.join(" ")
        ))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "Command 'nix {}' failed: {}",
            args.join(" "),
            stderr
        ));
    }

    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}

async fn run_nix_store(args: &[&str]) -> Result<String> {
    debug!("Running nix-store {:?}", args);
    let output = Command::new("nix-store")
        .args(args)
        .output()
        .await
        .context(format!(
            "Failed to execute nix-store command: nix-store {}",
            args.join(" ")
        ))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!(
            "Command 'nix-store {}' failed: {}",
            args.join(" "),
            stderr
        ));
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
        .kill_on_drop(true)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context(format!("Failed to spawn {}", cmd_name))?;

    let stdout = child.stdout.take().context("Failed to capture stdout")?;
    let stderr = child.stderr.take().context("Failed to capture stderr")?;

    let mut reader = BufReader::new(stderr);
    let mut line = String::new();

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
        let timestamp = Local::now().format("%F %H:%M:%S.%3f");
        let stamped_line = format!("[{}] {}", timestamp, line);
        log_file.write_all(stamped_line.as_bytes()).await?;
    }

    let status = child.wait().await?;
    let stdout_result = stdout_handle.await??;

    if !status.success() {
        return Err(anyhow!(
            "Command '{} {}' failed with exit code: {:?}",
            cmd_name,
            args.join(" "),
            status.code()
        ));
    }

    Ok(stdout_result.trim().to_string())
}

pub fn resolve_flake_ref(
    url: Option<String>,
    branch: Option<String>,
    default_path: &str,
) -> String {
    // Guessing if git ref or http ref is passed, probably can be done better
    if let Some(mut url) = url {
        if !url.contains("://") && url.contains('@') {
            url = format!("git+ssh://{}", url.replace(":", "/"));
        } else if url.starts_with("https://") && url.ends_with(".git") {
            url = format!("git+{}", url);
        }

        if let Some(branch) = branch {
            format!("{}?ref={}", url, branch)
        } else {
            url
        }
    } else {
        default_path.to_string()
    }
}
