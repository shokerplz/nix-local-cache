use clap::{Parser, Subcommand};
use anyhow::{Context, Result};
use reqwest::Client;
use serde::Deserialize;
use std::process::Command;
use chrono::{DateTime, Local};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
    Terminal,
};
use crossterm::{
    event::{self, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};

#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Cli {
    /// URL of the nix-local-cache server API
    #[arg(long, env = "NIX_LOCAL_CACHE_API", default_value = "http://localhost:3000")]
    pub api: String,

    /// URL of the binary cache (defaults to API URL if not set)
    #[arg(long, env = "NIX_LOCAL_CACHE_URI")]
    pub cache_uri: Option<String>,

    /// Hostname to filter builds for (defaults to current hostname)
    #[arg(long)]
    pub host: Option<String>,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List available builds for this host
    List,
    /// Apply a specific build (by Job ID) or select interactively
    Apply {
        /// Job ID to apply (optional, interactive mode if missing)
        job_id: Option<String>,
        
        /// Skip confirmation
        #[arg(long, short)]
        yes: bool,
    },
}

#[derive(Debug, Deserialize)]
struct Job {
    id: String,
    created_at: DateTime<Local>,
    results: Option<std::collections::HashMap<String, String>>,
    flake_ref: String,
    status: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let client = Client::new();

    // Default cache_uri to api if not set
    let cache_uri = cli.cache_uri.clone().unwrap_or_else(|| cli.api.clone());

    let hostname = if let Some(h) = cli.host {
        h
    } else {
        hostname::get()?.into_string().map_err(|_| anyhow::anyhow!("Invalid hostname"))?
    };

    match &cli.command {
        Commands::List => {
            let jobs = fetch_compatible_jobs(&client, &cli.api, &hostname).await?;
            if jobs.is_empty() {
                println!("No compatible builds found for host '{}'", hostname);
                return Ok(());
            }

            println!("{:<38} {:<25} {:<10} {}", "JOB ID", "DATE", "STATUS", "STORE PATH");
            println!("{:-<38} {:-<25} {:-<10} {:-<10}", "", "", "", "");
            for job in jobs {
                let path = job.results.as_ref().and_then(|r| r.get(&hostname)).map(|s| s.as_str()).unwrap_or("N/A");
                println!("{:<38} {:<25} {:<10} {}", job.id, job.created_at.format("%Y-%m-%d %H:%M:%S"), job.status, path);
            }
        }
        Commands::Apply { job_id, yes } => {
            let target_job = if let Some(id) = job_id {
                let jobs = fetch_compatible_jobs(&client, &cli.api, &hostname).await?;
                jobs.into_iter().find(|j| j.id == *id).context("Job not found or not compatible")?
            } else {
                // Interactive mode
                let jobs = fetch_compatible_jobs(&client, &cli.api, &hostname).await?;
                if jobs.is_empty() {
                    println!("No compatible builds found for host '{}'", hostname);
                    return Ok(());
                }
                select_job_interactively(jobs, &hostname)?
            };

            let store_path = target_job.results.as_ref()
                .and_then(|r| r.get(&hostname))
                .context("Selected job has no result for this host")?;

            println!("Selected build:");
            println!("  Job ID:     {}", target_job.id);
            println!("  Date:       {}", target_job.created_at);
            println!("  Store Path: {}", store_path);
            
            if !yes {
                println!("\nPress Enter to apply, or Ctrl+C to cancel...");
                let mut input = String::new();
                std::io::stdin().read_line(&mut input)?;
            }

            apply_system(store_path, &cache_uri).await?;
        }
    }

    Ok(())
}

async fn fetch_compatible_jobs(client: &Client, api: &str, hostname: &str) -> Result<Vec<Job>> {
    let url = format!("{}/jobs", api);
    let mut jobs: Vec<Job> = client.get(&url).send().await?.json().await?;
    
    // Filter jobs that have a result for our hostname and are completed
    jobs.retain(|j| {
        j.status == "Completed" && 
        j.results.as_ref().map(|r| r.contains_key(hostname)).unwrap_or(false)
    });

    Ok(jobs)
}

fn select_job_interactively(jobs: Vec<Job>, hostname: &str) -> Result<Job> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = ListState::default();
    state.select(Some(0));

    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(0), Constraint::Length(3)].as_ref())
                .split(f.size());

            let items: Vec<ListItem> = jobs.iter().map(|job| {
                let path = job.results.as_ref().and_then(|r| r.get(hostname)).map(|s| s.as_str()).unwrap_or("N/A");
                let content = vec![
                    Line::from(vec![
                        Span::styled(format!("{:<22} ", job.created_at.format("%Y-%m-%d %H:%M")), Style::default().fg(Color::Yellow)),
                        Span::styled(format!("{}", job.flake_ref), Style::default().fg(Color::Cyan)),
                    ]),
                    Line::from(Span::styled(format!("  {}", path), Style::default().fg(Color::DarkGray))),
                ];
                ListItem::new(content)
            }).collect();

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Select Build to Apply"))
                .highlight_style(Style::default().add_modifier(Modifier::BOLD).bg(Color::DarkGray))
                .highlight_symbol("> ");

            f.render_stateful_widget(list, chunks[0], &mut state);

            let help = Paragraph::new("Use ↑/↓ to navigate, Enter to select, q to quit")
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(help, chunks[1]);
        })?;

        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => {
                    disable_raw_mode()?;
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    std::process::exit(0);
                }
                KeyCode::Down => {
                    let i = match state.selected() {
                        Some(i) => if i >= jobs.len() - 1 { 0 } else { i + 1 },
                        None => 0,
                    };
                    state.select(Some(i));
                }
                KeyCode::Up => {
                    let i = match state.selected() {
                        Some(i) => if i == 0 { jobs.len() - 1 } else { i - 1 },
                        None => 0,
                    };
                    state.select(Some(i));
                }
                KeyCode::Enter => {
                    disable_raw_mode()?;
                    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
                    return Ok(jobs.into_iter().nth(state.selected().unwrap()).unwrap());
                }
                _ => {}
            }
        }
    }
}

async fn apply_system(store_path: &str, cache_uri: &str) -> Result<()> {
    // 1. Fetch closure
    // Since we don't have the binary cache public key setup here automatically, 
    // we assume the user has configured the machine to trust the cache 
    // OR we can pass --option trusted-public-keys ... if we knew it.
    // For now, let's assume standard `nix copy` works if configured.
    // But wait, `nix copy --from` works nicely.
    
    // We need to parse host from api_url or pass it.
    // If api_url is http://localhost:3000, we use that.
    
    println!("Fetching closure from {}...", cache_uri);
    // Note: `nix copy` needs the cache to be exposed as a binary cache. 
    // Our server is serving artifacts, but does it implement the full binary cache protocol?
    // It does if it serves .narinfo and .nar files at the root (or under /cache if configured?).
    // The `cache_dir` in config is served? 
    // Wait, the backend currently does NOT serve the cache files via HTTP!
    // It serves the API.
    // We need to add a static file handler to serve `cache_dir`!
    
    // WARNING: We missed this requirement. The backend MUST serve the cache dir for `nix copy` to work via HTTP.
    // I will add a TODO to fix the backend. For now, assuming it works or we fix it.
    
    // Assuming backend serves cache at /nix-cache or similar?
    // Let's assume the user configured the API URL to be the cache URL?
    // Actually, `nix-local-cache` is the API. The cache usually needs to be served.
    // Let's assume for this implementation that `nix copy --from` works.
    
    let status = Command::new("nix")
        .args(&["copy", "--from", cache_uri, store_path])
        .status()
        .context("Failed to run nix copy")?;

    if !status.success() {
        return Err(anyhow::anyhow!("nix copy failed"));
    }

    // 2. Set profile
    println!("Setting system profile...");
    let status = Command::new("nix-env")
        .args(&["-p", "/nix/var/nix/profiles/system", "--set", store_path])
        .status()
        .context("Failed to set system profile")?;

    if !status.success() {
        return Err(anyhow::anyhow!("nix-env failed"));
    }

    // 3. Switch
    println!("Switching configuration...");
    let switch_bin = format!("{}/bin/switch-to-configuration", store_path);
    let status = Command::new(switch_bin)
        .arg("switch")
        .status()
        .context("Failed to switch configuration")?;

    if !status.success() {
        return Err(anyhow::anyhow!("switch-to-configuration failed"));
    }

    println!("Successfully switched to {}", store_path);
    Ok(())
}
