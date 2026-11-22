use super::{line_height, scaled};
use crate::{
    config::{Config, LayoutMode, SpaceLayout, Theme, mix},
    contrast,
    direction::Logical,
};
use gpui::{prelude::*, *};
use unicode_width::UnicodeWidthChar;

#[derive(Clone)]
pub(super) struct Tile {
    letters: SharedString,
    background: u32,
    foreground: u32,
}

impl Tile {
    pub(super) fn new(name: &str, theme: &Theme) -> Self {
        let hues = [0x687fac, 0x8870a5, 0x647f98, 0x94759c];
        let hash = name.bytes().fold(0u32, |hash, byte| {
            hash.wrapping_mul(31)
                .wrapping_add(u32::from(byte.to_ascii_lowercase()))
        });
        let hue = hues[hash as usize % hues.len()];
        let background = mix(theme.surface, hue, 24);
        let foreground = contrast::ink_on_chrome(hue, [background; 3], 4.5);
        Self {
            letters: initials(name).into(),
            background,
            foreground,
        }
    }

    pub(super) fn element(&self, size: f32) -> AnyElement {
        div()
            .debug_selector(|| format!("repository-tile-{}", self.letters))
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(size / 4.))
            .bg(rgb(self.background))
            .text_color(rgb(self.foreground))
            .text_size(px(size / 2.))
            .line_height(px(size))
            .font_weight(FontWeight::NORMAL)
            .overflow_hidden()
            .child(self.letters.clone())
            .into_any_element()
    }

    pub(super) fn chip(&self, key: &str, height: f32) -> Div {
        div()
            .debug_selector(|| format!("repository-chip-{key}"))
            .w(px(height * 1.4))
            .h(px(height))
            .child(self.element(height))
    }
}

pub(super) fn name(workspace: &sheprd_session::Workspace) -> Option<&str> {
    workspace.branch.as_ref()?;
    Some(
        workspace
            .worktree
            .as_ref()
            .filter(|tree| !tree.label.is_empty())
            .map_or(workspace.label.as_str(), |tree| tree.label.as_str()),
    )
}

pub(super) fn gap(config: &Config) -> f32 {
    if config.usage.inline && config.sidebar_layout.spaces != SpaceLayout::default() {
        return f32::from(config.sidebar_layout.spaces.row_gap) * line_height(&config.sidebar);
    }
    let gap = match config.layout.mode {
        LayoutMode::Classic { .. } => 7.,
        LayoutMode::Minimal => 16.,
        _ => 0.,
    };
    scaled(&config.sidebar, gap)
}

pub(super) fn separator(key: &str, gap: f32, inset: f32, theme: &Theme) -> Div {
    div()
        .debug_selector(|| format!("repository-separator-{key}"))
        .absolute()
        .inset_s(px(inset))
        .inset_e(px(inset))
        .top(px(-gap / 2.))
        .h(px(1.))
        .bg(rgb(mix(theme.surface, theme.muted, 30)))
}

fn initials(name: &str) -> String {
    let mut words = name
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty());
    let Some(first) = words.next() else {
        return "?".into();
    };
    let mut characters = first.chars();
    let leading = characters.next();
    let following = words
        .next()
        .and_then(|word| word.chars().next())
        .or_else(|| characters.next());
    let mut width = 0;
    leading
        .into_iter()
        .chain(following)
        .flat_map(char::to_uppercase)
        .take_while(|character| {
            width += character.width().unwrap_or(0);
            width <= 2
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_identity_survives_worktree_renames() {
        let mut session =
            crate::session_model::of_snapshot(&crate::sidebar::layout_tests::snapshot(6));
        let parent = &mut session.workspaces[3];
        assert_eq!(name(parent), Some("agent-launcher"));
        parent.label = "renamed main checkout".into();
        assert_eq!(name(parent), Some("agent-launcher"));
        let child = &mut session.workspaces[4];
        child.label = "renamed worktree".into();
        assert_eq!(name(child), Some("agent-launcher"));
        child.branch = None;
        assert_eq!(name(child), None);
    }

    #[test]
    fn custom_spacing_takes_precedence_over_group_defaults() -> anyhow::Result<()> {
        let mut config = Config::default();
        for mode in [LayoutMode::default(), LayoutMode::Minimal] {
            config.layout.mode = mode;
            config.sidebar_layout.spaces = toml::from_str(
                r#"rows = [["workspace"]]
row_gap = 0"#,
            )?;
            assert_eq!(gap(&config), 0.);
            config.sidebar_layout.spaces.row_gap = 2;
            assert_eq!(gap(&config), 2. * line_height(&config.sidebar));
        }
        Ok(())
    }

    #[test]
    fn initials_are_bounded_and_follow_repository_words() {
        for (name, expected) in [
            ("herdr-gpui", "HG"),
            ("moonmap", "MO"),
            ("flexcolor-rust", "FR"),
            ("  msgvault__moonframe", "MM"),
            ("a", "A"),
            ("---", "?"),
            ("東京-repo", "東"),
            ("école-outils", "ÉO"),
            ("ßeta", "SS"),
        ] {
            assert_eq!(initials(name), expected, "{name}");
        }
    }

    #[test]
    fn letters_contrast_on_every_builtin_theme() {
        for name in Theme::BUILTIN_NAMES {
            let Some(theme) = Theme::builtin(name) else {
                panic!("missing theme {name}");
            };
            for repository in ["herdr-gpui", "moonmap", "moltis", "moonswift"] {
                let tile = Tile::new(repository, &theme);
                assert!(
                    contrast::ratio(tile.foreground, tile.background) >= 4.5,
                    "{name}: {repository}"
                );
                let same = Tile::new(&repository.to_uppercase(), &theme);
                assert_eq!(tile.background, same.background);
                assert_eq!(tile.letters, same.letters);
            }
        }
    }
}
