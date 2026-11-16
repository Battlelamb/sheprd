//! `sheprd-t3`: T3 Code threads from the command line, through the same
//! client the app will use. See `--help`.

mod cli;

use std::{
    fs::{File, OpenOptions},
    io::{Read as _, Write as _},
    path::Path,
    process::ExitCode,
};

use anyhow::{Context as _, bail};
use secrecy::{ExposeSecret as _, SecretString};
use serde_json::json;
use sheprd_t3::{
    connection::Connection, discover, frame::ServerFrame, http::Server, shell::Shell,
    thread::Thread,
};
use url::Url;

use cli::{Command, Options};

/// The most a token file may weigh.
const TOKEN_LIMIT: u64 = 16 * 1024;

fn main() -> ExitCode {
    let options = match cli::parse(std::env::args_os().skip(1)) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("sheprd-t3: {error}");
            return ExitCode::from(2);
        }
    };
    match run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("sheprd-t3: {error:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(options: Options) -> anyhow::Result<()> {
    if options.command == Command::Help {
        print!("{}", cli::HELP);
        return Ok(());
    }
    let server = Server::new(origin(options.server.as_deref())?);
    let descriptor = server
        .describe()
        .with_context(|| format!("no T3 Code server answers at {}", server.origin()))?;
    let token_file = options.token_file.context("--token-file is required")?;
    match options.command {
        Command::Help => Ok(()),
        Command::Pair { code } => {
            let access = server.sign_in(&SecretString::from(code), "Sheprd")?;
            save_token(&token_file, &access.token)?;
            if options.json {
                println!(
                    "{}",
                    json!({
                        "environment": descriptor.environment_id,
                        "label": descriptor.label,
                        "scope": access.scope,
                        "version": descriptor.server_version,
                    })
                );
            } else {
                println!(
                    "Signed in to {} (T3 Code {}) with {}; the token is in {}.",
                    descriptor.label,
                    descriptor.server_version,
                    access.scope,
                    token_file.display()
                );
            }
            Ok(())
        }
        Command::Threads => {
            let mut connection = connect(&server, &token_file)?;
            let id = connection.request("orchestration.subscribeShell", json!({}))?;
            let mut shell = Shell::default();
            first_chunk(&mut connection, &id, |value| shell.apply(value))?;
            connection.interrupt(&id)?;
            connection.close();
            print_threads(&shell, options.json)
        }
        Command::Show { thread } => {
            let mut connection = connect(&server, &token_file)?;
            let id = connection.request(
                "orchestration.subscribeThread",
                json!({ "threadId": thread }),
            )?;
            let mut state = Thread::default();
            first_chunk(&mut connection, &id, |value| {
                state.apply(value);
                Ok(())
            })?;
            connection.interrupt(&id)?;
            connection.close();
            let agent = state
                .header
                .provider_instance_id
                .clone()
                .unwrap_or_else(|| "T3 Code".to_owned());
            let transcript = sheprd_t3::transcript::transcript(&state, &agent);
            if options.json {
                println!("{}", serde_json::to_string_pretty(&transcript)?);
            } else {
                println!("{} ({:?})", state.header.title, transcript.status);
                for entry in &transcript.entries {
                    println!("{}", line(&entry.kind));
                }
            }
            Ok(())
        }
    }
}

/// `--server`, or the server running on this computer.
fn origin(server: Option<&str>) -> anyhow::Result<Url> {
    if let Some(server) = server {
        return Ok(Url::parse(server)?);
    }
    let home = discover::t3_home(
        std::env::var_os("T3CODE_HOME").as_deref(),
        std::env::home_dir().as_deref(),
    )
    .context("no home directory to find T3 Code in")?;
    Ok(discover::local_origin(&home)?)
}

fn connect(server: &Server, token_file: &Path) -> anyhow::Result<Connection<std::net::TcpStream>> {
    let token = read_token(token_file)?;
    let ticket = server.ticket(&token)?;
    Ok(Connection::open(&server.socket(&ticket)?)?)
}

/// Reads frames until the stream `id` sends its first chunk, applying each
/// of its values.
fn first_chunk(
    connection: &mut Connection<std::net::TcpStream>,
    id: &str,
    mut apply: impl FnMut(serde_json::Value) -> sheprd_t3::Result<()>,
) -> anyhow::Result<()> {
    loop {
        match connection.next_frame()? {
            ServerFrame::Chunk { request_id, values } if request_id == id => {
                for value in values {
                    apply(value)?;
                }
                return Ok(());
            }
            ServerFrame::Exit { request_id, exit } if request_id == id => {
                bail!(
                    "T3 Code ended the stream: {}",
                    exit.cause().unwrap_or_default()
                );
            }
            _ => {}
        }
    }
}

fn print_threads(shell: &Shell, json: bool) -> anyhow::Result<()> {
    let mut threads: Vec<_> = shell.threads.values().collect();
    threads.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    if json {
        let threads: Vec<_> = threads
            .iter()
            .map(|thread| {
                json!({
                    "id": thread.id,
                    "project": shell.projects.get(&thread.project_id).map(|project| &project.title),
                    "provider": thread.provider_instance_id,
                    "status": thread.status,
                    "title": thread.title,
                    "updatedAt": thread.updated_at,
                    "waiting": thread.waiting(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&threads)?);
        return Ok(());
    }
    for thread in threads {
        let project = shell
            .projects
            .get(&thread.project_id)
            .map_or("?", |project| project.title.as_str());
        let status = if thread.waiting() {
            "waiting"
        } else {
            thread.status.as_deref().unwrap_or("?")
        };
        println!("{}\t{project}\t{status}\t{}", thread.id, thread.title);
    }
    Ok(())
}

/// An entry on one line, for the terminal.
fn line(kind: &sheprd_chat_protocol::EntryKind) -> String {
    use sheprd_chat_protocol::EntryKind;
    let first = |text: &str| text.lines().next().unwrap_or_default().to_owned();
    match kind {
        EntryKind::User { text, .. } => format!("> {}", first(text)),
        EntryKind::Agent { text } => first(text),
        EntryKind::Thought { text } => format!("(thinking) {}", first(text)),
        EntryKind::Tool(tool) => format!("[{:?}] {} ({:?})", tool.kind, tool.title, tool.status),
        EntryKind::Notice { text, level, .. } => format!("{level:?}: {}", first(text)),
        EntryKind::Image { .. } => "[picture]".to_owned(),
        EntryKind::Turn(_) => String::new(),
    }
}

fn save_token(path: &Path, token: &SecretString) -> anyhow::Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let mut file = options
        .open(path)
        .with_context(|| format!("could not write {}", path.display()))?;
    file.write_all(token.expose_secret().as_bytes())?;
    Ok(())
}

fn read_token(path: &Path) -> anyhow::Result<SecretString> {
    let mut text = String::new();
    File::open(path)
        .with_context(|| {
            format!(
                "could not read {}; run `sheprd-t3 pair` first",
                path.display()
            )
        })?
        .take(TOKEN_LIMIT)
        .read_to_string(&mut text)?;
    let token = text.trim();
    if token.is_empty() {
        bail!("{} holds no token", path.display());
    }
    Ok(SecretString::from(token.to_owned()))
}
