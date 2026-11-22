//! `phone`: agents driving a paired phone's app.
use super::{Args, CliError, unknown};
use sheprd_remote_control::control::phone::Screen;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneCommand {
    /// The phones connected that let agents drive them.
    Phones,
    /// Which screen the phone shows.
    State { device: Option<String> },
    Open {
        device: Option<String>,
        target: PhoneTarget,
    },
    /// A note on the phone, from this computer's agent.
    Show {
        device: Option<String>,
        text: String,
    },
}

/// What `phone open` shows. A pane or workspace left out is the caller's,
/// filled in from its environment when the command runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PhoneTarget {
    Screen(Screen),
    /// A pane as the phone shows it: its chat, else its terminal.
    Pane(Option<String>),
    Workspace(Option<String>),
    Changes(Option<String>),
}

pub(super) fn phone(args: &mut Args) -> Result<PhoneCommand, CliError> {
    let device = args.text_option("--device")?;
    let Some(verb) = args.verb() else {
        return Ok(PhoneCommand::Phones);
    };
    Ok(match verb.to_str() {
        Some("list" | "phones") => PhoneCommand::Phones,
        Some("state") => PhoneCommand::State { device },
        Some("open") => PhoneCommand::Open {
            device,
            target: target(args)?,
        },
        // A note may be typed unquoted, as `phone show tests pass`.
        Some("show") => PhoneCommand::Show {
            device,
            text: args.words()?.ok_or(CliError::MissingArgument {
                command: "phone show",
                what: "TEXT",
            })?,
        },
        _ => return Err(unknown("phone", verb)),
    })
}

fn target(args: &mut Args) -> Result<PhoneTarget, CliError> {
    let Some(word) = args.optional_text()? else {
        return Ok(PhoneTarget::Pane(None));
    };
    let pane = |args: &mut Args, command| {
        args.optional_text()?.ok_or(CliError::MissingArgument {
            command,
            what: "PANE",
        })
    };
    Ok(match word.as_str() {
        "home" => PhoneTarget::Screen(Screen::Home),
        "workspaces" => PhoneTarget::Screen(Screen::Workspaces),
        "pull-requests" | "prs" => PhoneTarget::Screen(Screen::PullRequests),
        "settings" => PhoneTarget::Screen(Screen::Settings),
        "new-workspace" => PhoneTarget::Screen(Screen::NewWorkspace),
        "chat" => PhoneTarget::Screen(Screen::Chat {
            pane: pane(args, "phone open chat")?,
        }),
        "term" | "terminal" => PhoneTarget::Screen(Screen::Term {
            pane: pane(args, "phone open term")?,
        }),
        "pane" => PhoneTarget::Pane(args.optional_text()?),
        "workspace" => PhoneTarget::Workspace(args.optional_text()?),
        "changes" => PhoneTarget::Changes(args.optional_text()?),
        // Anything else is a pane's id, as `phone open w1-p2`.
        _ => PhoneTarget::Pane(Some(word)),
    })
}
