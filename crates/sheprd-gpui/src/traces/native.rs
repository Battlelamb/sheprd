//! Native captures with synthetic sessions; never reads a person's transcripts.
use super::*;
use anyhow::{Context as _, Result, ensure};
use chrono::{Days, Utc};
use service::Snapshot;
use sheprd_client::herdr::AgentKind;
use sheprd_traces::{Activity, EventKind, Session, Visibility};
use std::{sync::Arc, time::Duration};

pub(crate) fn start(handle: WindowHandle<SheprdWindow>, cx: &mut App) {
    cx.spawn(async move |cx| match run(handle, cx).await {
        Ok(()) => {
            eprintln!("TRACES native PASS");
            std::process::exit(0);
        }
        Err(error) => {
            eprintln!("TRACES native FAIL: {error:#}");
            std::process::exit(1);
        }
    })
    .detach();
}

async fn run(handle: WindowHandle<SheprdWindow>, cx: &mut AsyncApp) -> Result<()> {
    let snapshot = Arc::new(snapshot());
    // i18n: data
    for (theme, label) in [("Catppuccin Mocha", "dark"), ("Catppuccin Latte", "light")] {
        for width in [1400., 640.] {
            handle.update(cx, |view, window, cx| -> Result<()> {
                view.config.theme = theme.into();
                view.theme = view.config.theme(label == "light")?;
                view.config.sidebar_style.activity_bar = true;
                view.activity.panel = ActivityPanel::Traces;
                view.traces.status = Status::Ready(snapshot.clone());
                view.ensure_traces_page(cx);
                window.resize(size(px(width), px(900.)));
                cx.notify();
                Ok(())
            })??;
            capture(handle, &format!("{label}-{width}"), cx).await?;
        }
    }
    handle.update(cx, |view, window, cx| -> Result<()> {
        view.config.theme = "Catppuccin Mocha".into(); // i18n: data
        view.theme = view.config.theme(false)?;
        view.ensure_traces_page(cx);
        let page = view.traces.page.as_mut().context("missing traces page")?;
        page.selected = Some(snapshot.sessions[0].id.clone());
        page.detail = Some(Detail::Loaded(events()));
        window.resize(size(px(1400.), px(1000.)));
        cx.notify();
        Ok(())
    })??;
    capture(handle, "detail", cx).await?;
    Ok(())
}

async fn capture(handle: WindowHandle<SheprdWindow>, label: &str, cx: &mut AsyncApp) -> Result<()> {
    cx.background_executor()
        .timer(Duration::from_millis(250))
        .await;
    AnyWindowHandle::from(handle).update(cx, |root, window, cx| -> Result<()> {
        let view = root
            .downcast::<SheprdWindow>()
            .map_err(|_| anyhow::anyhow!("unexpected root"))?;
        ensure!(view.read(cx).activity_section() == Some(ActivityPanel::Traces));
        window.draw(cx).clear(cx);
        Ok(())
    })??;
    #[cfg(any(feature = "mockup", target_os = "linux"))]
    if let Some(path) = std::env::var_os("HERDR_TEST_TRACES_CAPTURE") {
        let path = std::path::PathBuf::from(path).with_extension(format!("{label}.png"));
        ensure!(path.is_absolute(), "capture destination must be absolute");
        #[cfg(target_os = "linux")]
        cx.background_executor()
            .spawn(async move {
                let status = sheprd_client::new_command("python3")
                    .arg(concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../../scripts/capture-native-x11.py"
                    ))
                    .arg(std::process::id().to_string())
                    .arg(path)
                    .status()
                    .context("capture native X11 window")?;
                ensure!(status.success(), "native X11 capture failed");
                Ok::<_, anyhow::Error>(())
            })
            .await?;
        #[cfg(all(feature = "mockup", not(target_os = "linux")))]
        {
            let image = AnyWindowHandle::from(handle)
                .update(cx, |_, window, _| window.render_to_image())??;
            cx.background_executor()
                .spawn(async move { image.save(&path).context("save native Traces capture") })
                .await?;
        }
    }
    #[cfg(not(any(feature = "mockup", target_os = "linux")))]
    let _ = label;
    Ok(())
}

fn snapshot() -> Snapshot {
    let now = Utc::now();
    let agents = [
        AgentKind::Claude,
        AgentKind::Codex,
        AgentKind::Pi,
        AgentKind::Omp,
    ];
    let titles = [
        "Fix login redirects and cover expired sessions", // i18n: data
        "Add a health check to the API",                  // i18n: data
        "Keep the navigation header visible while scrolling", // i18n: data
        "Review database migrations before deployment",   // i18n: data
        "Improve keyboard navigation in the settings dialog", // i18n: data
        "Document the release checklist",                 // i18n: data
    ];
    let sessions = titles
        .iter()
        .enumerate()
        .map(|(i, title)| Session {
            id: format!("demo-{i}"),
            host_id: "demo-host".into(),
            host: "Workstation".into(), // i18n: data
            agent: agents[i % agents.len()],
            native_id: format!("demo-{i}"),
            user: "demo@workstation".into(),
            title: Some((*title).into()),
            model: Some(
                if i % 2 == 0 {
                    "claude-opus-4-6"
                } else {
                    "gpt-5.4"
                }
                .into(),
            ),
            cwd: Some("~/code/example-app".into()),
            branch: Some("fix/session-redirects".into()),
            agent_version: None,
            started_at: Some(now.timestamp_millis() - 3_600_000 * (i as i64 + 1)),
            updated_at: Some(now.timestamp_millis() - 60_000 * (i as i64 + 1)),
            messages: 24 + i as i64 * 13,
            user_messages: 5,
            tool_calls: 12 + i as i64 * 4,
            errors: i as i64 % 2,
            tokens_in: 12_400 + i as i64 * 8_700,
            tokens_out: 3_200 + i as i64 * 900,
            visibility: Visibility::Private,
        })
        .collect();
    let mut activity = Vec::new();
    for day in 0..30 {
        for (i, agent) in agents.iter().enumerate() {
            if let Some(date) = now.date_naive().checked_sub_days(Days::new(day)) {
                activity.push(Activity {
                    day: date.to_string(),
                    agent: *agent,
                    sessions: 1,
                    messages: ((day as i64 * 7 + i as i64 * 11) % 43) + 3,
                });
            }
        }
    }
    Snapshot { sessions, activity }
}

fn events() -> Vec<Event> {
    [
        (EventKind::User, None, "Fix login redirects when a session expires. Add a regression test."), // i18n: data
        (EventKind::Thinking, None, "Check the redirect target before refreshing the session."), // i18n: data
        (EventKind::ToolCall, Some("shell"), "cargo test session_redirect"), // i18n: data
        (EventKind::ToolResult, Some("shell"), "3 tests passed; 0 failed"), // i18n: data
        (EventKind::Agent, None, "The redirect now preserves the original route. Tests cover expired sessions and nested routes."), // i18n: data
    ].into_iter().enumerate().map(|(i, (kind, tool, text))| Event {
        seq: i as i64, at: None, kind, tool: tool.map(str::to_owned), text: text.into(),
    }).collect()
}
