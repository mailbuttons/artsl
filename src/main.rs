//! artsl: console art that rolls across your terminal, drawn from the news.
//!
//! Three roles in one binary: the scroller you run for fun, the `generate`
//! command that turns headlines into a picture with Claude, and the `serve`
//! command mailbuttons.com runs so everyone gets a picture of the day and
//! subscribers get one drawn on demand.

mod art;
mod builtin;
mod config;
mod generate;
mod mailbuttons;
mod news;
mod scroll;
mod serve;
mod source;
mod store;

use anyhow::{bail, Context, Result};
use clap::{Args, CommandFactory, Parser, Subcommand};
use config::Config;
use source::{Origin, PickOpts};
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "artsl",
    version,
    about = "Console art that rolls across your terminal, drawn by an LLM from today's news.",
    long_about = "Like `sl`, but the picture changes with the news.\n\n\
With no options artsl shows the picture of the day from mailbuttons.com (or the built-in\n\
picture when offline). `artsl subscribe` signs you in with an emailed code; after that\n\
`artsl --fresh` has a new picture drawn for you on the spot."
)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    #[command(flatten)]
    show: ShowArgs,
}

#[derive(Args, Debug, Clone, Default)]
struct ShowArgs {
    /// Draw a new picture right now from the latest news (needs `artsl subscribe`)
    #[arg(short, long)]
    fresh: bool,
    /// Show the built-in picture
    #[arg(short, long)]
    builtin: bool,
    /// Never touch the network
    #[arg(short, long)]
    offline: bool,
    /// Scroll a picture from a JSON file
    #[arg(long, value_name = "PATH")]
    file: Option<PathBuf>,
    /// Milliseconds per column of travel (lower is faster)
    #[arg(short, long, value_name = "MS")]
    speed: Option<u64>,
    /// Ignore q and Ctrl-C until the picture has gone by, as sl does
    #[arg(long)]
    stubborn: bool,
    /// A slant for the on-demand picture, e.g. "something hopeful"
    #[arg(short, long, value_name = "TEXT")]
    theme: Option<String>,
    /// No credit line after the picture
    #[arg(short, long)]
    quiet: bool,
    /// mailbuttons base URL (for development)
    #[arg(long, env = "ARTSL_MAILBUTTONS_URL", global = true, value_name = "URL")]
    mailbuttons_url: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Cmd {
    /// Sign in to mailbuttons with an emailed code; enables `--fresh`
    Subscribe {
        /// Your email address (prompted if omitted)
        email: Option<String>,
    },
    /// Forget the mailbuttons key stored by `subscribe`
    Unsubscribe,
    /// Where things stand: config, cache, subscription
    Status,
    /// Print the current picture without animating it
    Print {
        #[command(flatten)]
        show: ShowArgs,
    },
    /// List the headlines a new picture would be drawn from
    News {
        /// Feed URL(s) to use instead of the configured ones
        #[arg(long = "feed", value_name = "URL")]
        feeds: Vec<String>,
    },
    /// Draw a new picture with your own Claude API key (ANTHROPIC_API_KEY)
    Generate {
        /// Write the picture here instead of stdout
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,
        /// Feed URL(s) to use instead of the configured ones
        #[arg(long = "feed", value_name = "URL")]
        feeds: Vec<String>,
        /// A slant for the picture
        #[arg(short, long, value_name = "TEXT")]
        theme: Option<String>,
        /// Model to draw with (default from config)
        #[arg(long)]
        model: Option<String>,
        /// Scroll the result as well
        #[arg(long)]
        show: bool,
    },
    /// Server: draw the picture of the day into a directory (for a timer)
    Publish {
        /// Directory that holds latest.json and archive/
        #[arg(long, env = "ARTSL_DATA_DIR", value_name = "DIR")]
        out: PathBuf,
        #[arg(long = "feed", value_name = "URL")]
        feeds: Vec<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Server: serve the picture of the day and draw on demand for subscribers
    Serve {
        #[arg(long, env = "ARTSL_LISTEN", default_value = "127.0.0.1:8090")]
        listen: String,
        #[arg(long, env = "ARTSL_DATA_DIR", value_name = "DIR")]
        data_dir: PathBuf,
        /// Hours between redraws of the picture of the day
        #[arg(long, env = "ARTSL_REFRESH_HOURS", default_value_t = 6)]
        refresh_hours: u64,
        /// Fresh pictures a subscriber may ask for per day
        #[arg(long, env = "ARTSL_DAILY_QUOTA", default_value_t = 24)]
        daily_quota: u32,
        /// Pictures drawn at the same time
        #[arg(long, env = "ARTSL_CONCURRENCY", default_value_t = 2)]
        concurrency: usize,
        /// Days to keep archived pictures (0 keeps everything)
        #[arg(long, env = "ARTSL_ARCHIVE_DAYS", default_value_t = 90)]
        archive_days: u64,
        /// Enables /art/v1/admin/* when set
        #[arg(long, env = "ARTSL_ADMIN_TOKEN", hide_env_values = true)]
        admin_token: Option<String>,
        #[arg(long = "feed", value_name = "URL")]
        feeds: Vec<String>,
        #[arg(long)]
        model: Option<String>,
    },
    /// Print the config file path
    ConfigPath,
    /// Print shell completions (bash, zsh, fish, elvish, powershell)
    Completions { shell: clap_complete::Shell },
    /// Print the manual page (roff)
    Man,
}

#[tokio::main]
async fn main() {
    if let Err(e) = real_main().await {
        eprintln!("artsl: {e:#}");
        std::process::exit(1);
    }
}

async fn real_main() -> Result<()> {
    let cli = Cli::parse();
    let mut cfg = Config::load()?;
    if let Some(url) = &cli.show.mailbuttons_url {
        cfg.mailbuttons.base_url = url.clone();
    }

    match cli.cmd {
        None => show(&cfg, &cli.show, true).await,
        Some(Cmd::Print { show: args }) => show(&cfg, &args, false).await,
        Some(Cmd::Subscribe { email }) => subscribe(&mut cfg, email).await,
        Some(Cmd::Unsubscribe) => {
            let Some(token) = cfg.token().map(str::to_owned) else {
                println!("Nothing stored; you weren't subscribed on this machine.");
                return Ok(());
            };
            let mb = mailbuttons::Mailbuttons::new(&cfg.mailbuttons.base_url)?;
            match mb.unsubscribe(&token).await {
                Ok(()) => println!("Token revoked at {}.", mb.base()),
                Err(e) => eprintln!("artsl: could not revoke the token remotely ({e:#}); forgetting it locally anyway."),
            }
            cfg.mailbuttons.token = None;
            cfg.save()?;
            println!("Forgotten. `artsl` still shows the picture of the day; `--fresh` needs `artsl subscribe` again.");
            Ok(())
        }
        Some(Cmd::Status) => status(&cfg),
        Some(Cmd::News { feeds }) => {
            let feeds = if feeds.is_empty() {
                cfg.generate.feeds.clone()
            } else {
                feeds
            };
            let http = reqwest::Client::new();
            let items = news::fetch_headlines(&http, &feeds, cfg.generate.headlines).await?;
            if items.is_empty() {
                bail!("no headlines came back from {} feed(s)", feeds.len());
            }
            print!("{}", news::format_for_prompt(&items));
            Ok(())
        }
        Some(Cmd::Generate {
            out,
            feeds,
            theme,
            model,
            show: also_show,
        }) => {
            let gen = generator(&cfg, model)?;
            let feeds = if feeds.is_empty() {
                cfg.generate.feeds.clone()
            } else {
                feeds
            };
            let http = reqwest::Client::new();
            let spinner = source::Spinner::start("Reading the news and drawing");
            let items = news::fetch_headlines(&http, &feeds, cfg.generate.headlines).await?;
            let art = gen.draw(&items, theme.as_deref()).await?;
            drop(spinner);
            let json = serde_json::to_string_pretty(&art)?;
            match &out {
                Some(path) => {
                    std::fs::write(path, &json)
                        .with_context(|| format!("writing {}", path.display()))?;
                    eprintln!("artsl: wrote {} ({})", path.display(), art.credit());
                }
                None if also_show => {}
                None => println!("{json}"),
            }
            let _ = source::save_cache(&art);
            if also_show {
                scroll_or_print(&cfg, &art, Origin::Fresh, &ShowArgs::default(), true)?;
            }
            Ok(())
        }
        Some(Cmd::Publish { out, feeds, model }) => {
            let gen = generator(&cfg, model)?;
            let feeds = if feeds.is_empty() {
                cfg.generate.feeds.clone()
            } else {
                feeds
            };
            let http = reqwest::Client::builder()
                .user_agent(concat!("artsl-server/", env!("CARGO_PKG_VERSION")))
                .build()?;
            tokio::fs::create_dir_all(&out).await?;
            let art = serve::publish_once(
                &gen,
                &http,
                &feeds,
                cfg.generate.headlines,
                &out,
                None,
                true,
            )
            .await?;
            println!("{} {}", art.id, art.credit());
            Ok(())
        }
        Some(Cmd::Serve {
            listen,
            data_dir,
            refresh_hours,
            daily_quota,
            concurrency,
            archive_days,
            admin_token,
            feeds,
            model,
        }) => {
            tracing_subscriber::fmt()
                .with_env_filter(
                    tracing_subscriber::EnvFilter::try_from_default_env()
                        .unwrap_or_else(|_| "info,tower_http=info".into()),
                )
                .with_target(false)
                .init();
            let gen = generator(&cfg, model)?;
            let feeds = if feeds.is_empty() {
                cfg.generate.feeds.clone()
            } else {
                feeds
            };
            serve::run(
                serve::ServeConfig {
                    listen,
                    data_dir,
                    mailbuttons_url: cfg.mailbuttons.base_url.clone(),
                    feeds,
                    headlines: cfg.generate.headlines,
                    refresh: Duration::from_secs(refresh_hours.max(1) * 3600),
                    daily_quota,
                    concurrency,
                    archive_days,
                    admin_token: admin_token.filter(|t| !t.trim().is_empty()),
                },
                gen,
            )
            .await
        }
        Some(Cmd::ConfigPath) => {
            println!("{}", config::config_path().display());
            Ok(())
        }
        Some(Cmd::Completions { shell }) => {
            let mut cmd = Cli::command();
            clap_complete::generate(shell, &mut cmd, "artsl", &mut std::io::stdout());
            Ok(())
        }
        Some(Cmd::Man) => {
            let cmd = Cli::command();
            let man = clap_mangen::Man::new(cmd);
            let mut out = Vec::new();
            man.render(&mut out)?;
            std::io::stdout().write_all(&out)?;
            Ok(())
        }
    }
}

fn generator(cfg: &Config, model: Option<String>) -> Result<generate::Generator> {
    let key = cfg.anthropic_api_key().context(
        "no Claude API key. Set ANTHROPIC_API_KEY, or put anthropic_api_key under [generate] in the config file.\n\
         (No key? `artsl --fresh` draws through your mailbuttons subscription instead.)",
    )?;
    generate::Generator::new(
        key,
        model.unwrap_or_else(|| cfg.generate.model.clone()),
        cfg.generate.effort.clone(),
    )
}

async fn show(cfg: &Config, args: &ShowArgs, animate: bool) -> Result<()> {
    let opts = PickOpts {
        builtin: args.builtin,
        fresh: args.fresh,
        offline: args.offline,
        theme: args.theme.clone(),
        file: args.file.clone(),
        mailbuttons_url: args.mailbuttons_url.clone(),
    };
    let (art, origin) = source::pick(cfg, &opts).await?;
    scroll_or_print(cfg, &art, origin, args, animate)
}

fn scroll_or_print(
    cfg: &Config,
    art: &art::Art,
    origin: Origin,
    args: &ShowArgs,
    animate: bool,
) -> Result<()> {
    let mut stdout = std::io::stdout();
    if !animate || !stdout.is_terminal() {
        scroll::print_plain(art, &mut stdout)?;
        if !args.quiet {
            writeln!(stdout, "  {} · {}", art.credit(), origin.label())?;
        }
        return Ok(());
    }
    let stubborn = args.stubborn || cfg.display.stubborn;
    let footer = if stubborn {
        format!(" artsl · {} · Ctrl-C will not help you", origin.label())
    } else {
        format!(" artsl · {} · q to stop", origin.label())
    };
    scroll::run(
        art,
        &scroll::ScrollOpts {
            speed_ms: args.speed.unwrap_or(cfg.display.speed_ms),
            frame_ms: cfg.display.frame_ms,
            stubborn,
            footer,
        },
    )?;
    if !args.quiet {
        println!("\x1b[2m{} · {}\x1b[0m", art.credit(), origin.label());
    }
    Ok(())
}

fn prompt(label: &str) -> Result<String> {
    print!("{label}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

async fn subscribe(cfg: &mut Config, email: Option<String>) -> Result<()> {
    let mb = mailbuttons::Mailbuttons::new(&cfg.mailbuttons.base_url)?;
    let email = match email {
        Some(e) => e,
        None => prompt("Email address: ")?,
    };
    if !email.contains('@') {
        bail!("that doesn't look like an email address");
    }
    mb.request_otp(&email).await?;
    println!("A six-digit code is on its way to {email}.");
    let mut key = None;
    for attempt in 0..3 {
        let code = prompt("Code: ")?;
        if code.is_empty() {
            continue;
        }
        match mb.cli_exchange(&email, &code).await {
            Ok(k) => {
                key = Some(k);
                break;
            }
            Err(e) if attempt < 2 => eprintln!("{e:#}. Try again."),
            Err(e) => return Err(e),
        }
    }
    let account_key = key.context("no code entered")?;
    // The account key is used exactly once, here, to obtain an art-only token.
    let token = mb.subscribe(&account_key, &machine_label()).await?;
    drop(account_key);
    cfg.mailbuttons.token = Some(token);
    cfg.mailbuttons.email = Some(email);
    cfg.save()?;
    println!(
        "Subscribed. `artsl --fresh` now draws a new picture from the latest news on demand.\n\
         An art-only token is stored in {} (it can draw pictures and nothing else).",
        config::config_path().display()
    );
    println!(
        "Note: signing in created an API key named \"CLI\" on your mailbuttons account. artsl does not keep it;\n\
         you can revoke it any time from the dashboard under Developers > API keys."
    );
    Ok(())
}

fn machine_label() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .unwrap_or_else(|| "artsl".to_string())
}

fn status(cfg: &Config) -> Result<()> {
    println!("config      {}", config::config_path().display());
    println!("cache       {}", source::cache_path().display());
    match source::load_cache() {
        Some((art, age)) => {
            let hours = age.as_secs() / 3600;
            let mins = (age.as_secs() % 3600) / 60;
            println!("cached      {} ({}h{:02}m old)", art.credit(), hours, mins);
        }
        None => println!("cached      nothing yet"),
    }
    println!(
        "feed        {} (refresh every {}h)",
        cfg.feed.url, cfg.feed.refresh_hours
    );
    println!("mailbuttons {}", cfg.mailbuttons.base_url);
    match cfg.token() {
        Some(t) => println!(
            "subscribed  yes ({}, token {}…) — `artsl --fresh` draws on demand",
            cfg.mailbuttons.email.as_deref().unwrap_or("email unknown"),
            t.chars().take(8).collect::<String>()
        ),
        None => println!("subscribed  no — run `artsl subscribe`"),
    }
    println!(
        "own key     {} — `artsl generate` draws with your own Claude key",
        if cfg.anthropic_api_key().is_some() {
            "ANTHROPIC_API_KEY set"
        } else {
            "not set"
        }
    );
    println!(
        "model       {} (effort {})",
        cfg.generate.model, cfg.generate.effort
    );
    println!("news feeds  {}", cfg.generate.feeds.len());
    Ok(())
}
