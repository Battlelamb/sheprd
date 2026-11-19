use super::*;
use crate::cli::control::McpClient;
use std::path::Path;

#[test]
fn each_client_gets_the_executable_in_its_own_format() {
    let path = Path::new("/Applications/Sheprd.app/Contents/MacOS/sheprd");
    assert_eq!(
        setup::snippet(McpClient::ClaudeCode, path),
        "claude mcp add sheprd -- /Applications/Sheprd.app/Contents/MacOS/sheprd mcp"
    );
    let desktop: Value =
        serde_json::from_str(&setup::snippet(McpClient::ClaudeDesktop, path)).unwrap();
    assert_eq!(
        desktop,
        json!({ "mcpServers": { "sheprd": { "command": path, "args": ["mcp"] } } })
    );
    assert_eq!(
        setup::snippet(McpClient::Codex, path),
        "[mcp_servers.sheprd]\ncommand = \"/Applications/Sheprd.app/Contents/MacOS/sheprd\"\nargs = [\"mcp\"]"
    );
}

#[test]
fn paths_with_spaces_or_quotes_stay_one_word() {
    let path = Path::new("/Users/me/My Apps/it's/sheprd");
    assert_eq!(
        setup::snippet(McpClient::ClaudeCode, path),
        r"claude mcp add sheprd -- '/Users/me/My Apps/it'\''s/sheprd' mcp"
    );
    let codex = setup::snippet(
        McpClient::Codex,
        Path::new("C:\\Program Files\\Sheprd\\sheprd.exe"),
    );
    assert!(
        codex.contains(r#"command = "C:\\Program Files\\Sheprd\\sheprd.exe""#),
        "{codex}"
    );
}
