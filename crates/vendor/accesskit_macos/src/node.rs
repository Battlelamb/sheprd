// Copyright 2022 The AccessKit Authors. All rights reserved.
// Licensed under the Apache License, Version 2.0 (found in
// the LICENSE-APACHE file) or the MIT license (found in
// the LICENSE-MIT file), at your option.

// Derived from Chromium's accessibility abstraction.
// Copyright 2018 The Chromium Authors. All rights reserved.
// Use of this source code is governed by a BSD-style license that can be
// found in the LICENSE.chromium file.

#![allow(non_upper_case_globals)]

use accesskit::{Role, TextSelection, Toggled};
use accesskit_consumer::{FilterResult, Node};
use objc2_app_kit::*;
use objc2_foundation::ns_string;

use crate::filters::filter;

mod platform;
pub(crate) use platform::PlatformNode;

fn ns_role(node: &Node) -> &'static NSAccessibilityRole {
    let role = node.role();
    // TODO: Handle special cases.
    unsafe {
        match role {
            Role::Unknown => NSAccessibilityUnknownRole,
            Role::TextRun => NSAccessibilityUnknownRole,
            Role::Cell | Role::GridCell => NSAccessibilityCellRole,
            Role::Label => NSAccessibilityStaticTextRole,
            Role::Image => NSAccessibilityImageRole,
            Role::Link => NSAccessibilityLinkRole,
            Role::Row => NSAccessibilityRowRole,
            Role::ListItem => NSAccessibilityGroupRole,
            Role::ListMarker => ns_string!("AXListMarker"),
            Role::TreeItem => NSAccessibilityRowRole,
            Role::ListBoxOption => NSAccessibilityStaticTextRole,
            Role::MenuItem => NSAccessibilityMenuItemRole,
            Role::MenuListOption => NSAccessibilityMenuItemRole,
            Role::Paragraph => NSAccessibilityGroupRole,
            Role::GenericContainer => NSAccessibilityUnknownRole,
            Role::CheckBox => NSAccessibilityCheckBoxRole,
            Role::RadioButton => NSAccessibilityRadioButtonRole,
            Role::TextInput
            | Role::SearchInput
            | Role::EmailInput
            | Role::NumberInput
            | Role::PasswordInput
            | Role::PhoneNumberInput
            | Role::UrlInput => NSAccessibilityTextFieldRole,
            Role::Button => {
                if node.toggled().is_some() {
                    NSAccessibilityCheckBoxRole
                } else {
                    NSAccessibilityButtonRole
                }
            }
            Role::DefaultButton => NSAccessibilityButtonRole,
            Role::Pane => NSAccessibilityUnknownRole,
            Role::RowHeader => NSAccessibilityCellRole,
            Role::ColumnHeader => NSAccessibilityCellRole,
            Role::RowGroup => NSAccessibilityGroupRole,
            Role::List => NSAccessibilityListRole,
            Role::Table => NSAccessibilityTableRole,
            Role::LayoutTableCell => NSAccessibilityGroupRole,
            Role::LayoutTableRow => NSAccessibilityGroupRole,
            Role::LayoutTable => NSAccessibilityGroupRole,
            Role::Switch => NSAccessibilityCheckBoxRole,
            Role::Menu => NSAccessibilityMenuRole,
            Role::MultilineTextInput => NSAccessibilityTextAreaRole,
            Role::DateInput | Role::DateTimeInput | Role::WeekInput | Role::MonthInput => {
                ns_string!("AXDateField")
            }
            Role::TimeInput => ns_string!("AXTimeField"),
            Role::Abbr => NSAccessibilityGroupRole,
            Role::Alert => NSAccessibilityGroupRole,
            Role::AlertDialog => NSAccessibilityWindowRole,
            Role::Application => NSAccessibilityGroupRole,
            Role::Article => NSAccessibilityGroupRole,
            Role::Audio => NSAccessibilityGroupRole,
            Role::Banner => NSAccessibilityGroupRole,
            Role::Blockquote => NSAccessibilityGroupRole,
            Role::Canvas => NSAccessibilityImageRole,
            Role::Caption => NSAccessibilityGroupRole,
            Role::Caret => NSAccessibilityUnknownRole,
            Role::Code => NSAccessibilityGroupRole,
            Role::ColorWell => NSAccessibilityColorWellRole,
            Role::ComboBox => NSAccessibilityPopUpButtonRole,
            Role::EditableComboBox => NSAccessibilityComboBoxRole,
            Role::Complementary => NSAccessibilityGroupRole,
            Role::Comment => NSAccessibilityGroupRole,
            Role::ContentDeletion => NSAccessibilityGroupRole,
            Role::ContentInsertion => NSAccessibilityGroupRole,
            Role::ContentInfo => NSAccessibilityGroupRole,
            Role::Definition => NSAccessibilityGroupRole,
            Role::DescriptionList => NSAccessibilityListRole,
            Role::Details => NSAccessibilityGroupRole,
            Role::Dialog => NSAccessibilityWindowRole,
            Role::DisclosureTriangle => NSAccessibilityButtonRole,
            Role::Document => NSAccessibilityGroupRole,
            Role::EmbeddedObject => NSAccessibilityGroupRole,
            Role::Emphasis => NSAccessibilityGroupRole,
            Role::Feed => NSAccessibilityUnknownRole,
            Role::FigureCaption => NSAccessibilityGroupRole,
            Role::Figure => NSAccessibilityGroupRole,
            Role::Footer => NSAccessibilityGroupRole,
            Role::Form => NSAccessibilityGroupRole,
            Role::Grid => NSAccessibilityTableRole,
            Role::Group => NSAccessibilityGroupRole,
            Role::Header => NSAccessibilityGroupRole,
            Role::Heading => ns_string!("Heading"),
            Role::Iframe => NSAccessibilityGroupRole,
            Role::IframePresentational => NSAccessibilityGroupRole,
            Role::ImeCandidate => NSAccessibilityUnknownRole,
            Role::Keyboard => NSAccessibilityUnknownRole,
            Role::Legend => NSAccessibilityGroupRole,
            Role::LineBreak => NSAccessibilityGroupRole,
            Role::ListBox => NSAccessibilityListRole,
            Role::Log => NSAccessibilityGroupRole,
            Role::Main => NSAccessibilityGroupRole,
            Role::Mark => NSAccessibilityGroupRole,
            Role::Marquee => NSAccessibilityGroupRole,
            Role::Math => NSAccessibilityGroupRole,
            Role::MenuBar => NSAccessibilityMenuBarRole,
            Role::MenuItemCheckBox => NSAccessibilityMenuItemRole,
            Role::MenuItemRadio => NSAccessibilityMenuItemRole,
            Role::MenuListPopup => NSAccessibilityMenuRole,
            Role::Meter => NSAccessibilityLevelIndicatorRole,
            Role::Navigation => NSAccessibilityGroupRole,
            Role::Note => NSAccessibilityGroupRole,
            Role::PluginObject => NSAccessibilityGroupRole,
            Role::ProgressIndicator => NSAccessibilityProgressIndicatorRole,
            Role::RadioGroup => NSAccessibilityRadioGroupRole,
            Role::Region => NSAccessibilityGroupRole,
            Role::RootWebArea => ns_string!("AXWebArea"),
            Role::Ruby => NSAccessibilityGroupRole,
            Role::RubyAnnotation => NSAccessibilityUnknownRole,
            Role::ScrollBar => NSAccessibilityScrollBarRole,
            Role::ScrollView => NSAccessibilityUnknownRole,
            Role::Search => NSAccessibilityGroupRole,
            Role::Section => NSAccessibilityGroupRole,
            Role::SectionFooter => NSAccessibilityGroupRole,
            Role::SectionHeader => NSAccessibilityGroupRole,
            Role::Slider => NSAccessibilitySliderRole,
            Role::SpinButton => NSAccessibilityIncrementorRole,
            Role::Splitter => NSAccessibilitySplitterRole,
            Role::Status => NSAccessibilityGroupRole,
            Role::Strong => NSAccessibilityGroupRole,
            Role::Suggestion => NSAccessibilityGroupRole,
            Role::SvgRoot => NSAccessibilityGroupRole,
            Role::Tab => NSAccessibilityRadioButtonRole,
            Role::TabList => NSAccessibilityTabGroupRole,
            Role::TabPanel => NSAccessibilityGroupRole,
            Role::Term => NSAccessibilityGroupRole,
            Role::Time => NSAccessibilityGroupRole,
            Role::Timer => NSAccessibilityGroupRole,
            Role::TitleBar => NSAccessibilityStaticTextRole,
            Role::Toolbar => NSAccessibilityToolbarRole,
            Role::Tooltip => NSAccessibilityGroupRole,
            Role::Tree => NSAccessibilityOutlineRole,
            Role::TreeGrid => NSAccessibilityTableRole,
            Role::Video => NSAccessibilityGroupRole,
            Role::WebView => NSAccessibilityUnknownRole,
            // Use the group role for Role::Window, since the NSWindow
            // provides the top-level accessibility object for the window.
            Role::Window => NSAccessibilityGroupRole,
            Role::PdfActionableHighlight => NSAccessibilityButtonRole,
            Role::PdfRoot => NSAccessibilityGroupRole,
            Role::GraphicsDocument => NSAccessibilityGroupRole,
            Role::GraphicsObject => NSAccessibilityGroupRole,
            Role::GraphicsSymbol => NSAccessibilityImageRole,
            Role::DocAbstract => NSAccessibilityGroupRole,
            Role::DocAcknowledgements => NSAccessibilityGroupRole,
            Role::DocAfterword => NSAccessibilityGroupRole,
            Role::DocAppendix => NSAccessibilityGroupRole,
            Role::DocBackLink => NSAccessibilityLinkRole,
            Role::DocBiblioEntry => NSAccessibilityGroupRole,
            Role::DocBibliography => NSAccessibilityGroupRole,
            Role::DocBiblioRef => NSAccessibilityGroupRole,
            Role::DocChapter => NSAccessibilityGroupRole,
            Role::DocColophon => NSAccessibilityGroupRole,
            Role::DocConclusion => NSAccessibilityGroupRole,
            Role::DocCover => NSAccessibilityImageRole,
            Role::DocCredit => NSAccessibilityGroupRole,
            Role::DocCredits => NSAccessibilityGroupRole,
            Role::DocDedication => NSAccessibilityGroupRole,
            Role::DocEndnote => NSAccessibilityGroupRole,
            Role::DocEndnotes => NSAccessibilityGroupRole,
            Role::DocEpigraph => NSAccessibilityGroupRole,
            Role::DocEpilogue => NSAccessibilityGroupRole,
            Role::DocErrata => NSAccessibilityGroupRole,
            Role::DocExample => NSAccessibilityGroupRole,
            Role::DocFootnote => NSAccessibilityGroupRole,
            Role::DocForeword => NSAccessibilityGroupRole,
            Role::DocGlossary => NSAccessibilityGroupRole,
            Role::DocGlossRef => NSAccessibilityLinkRole,
            Role::DocIndex => NSAccessibilityGroupRole,
            Role::DocIntroduction => NSAccessibilityGroupRole,
            Role::DocNoteRef => NSAccessibilityLinkRole,
            Role::DocNotice => NSAccessibilityGroupRole,
            Role::DocPageBreak => NSAccessibilitySplitterRole,
            Role::DocPageFooter => NSAccessibilityGroupRole,
            Role::DocPageHeader => NSAccessibilityGroupRole,
            Role::DocPageList => NSAccessibilityGroupRole,
            Role::DocPart => NSAccessibilityGroupRole,
            Role::DocPreface => NSAccessibilityGroupRole,
            Role::DocPrologue => NSAccessibilityGroupRole,
            Role::DocPullquote => NSAccessibilityGroupRole,
            Role::DocQna => NSAccessibilityGroupRole,
            Role::DocSubtitle => ns_string!("AXHeading"),
            Role::DocTip => NSAccessibilityGroupRole,
            Role::DocToc => NSAccessibilityGroupRole,
            Role::ListGrid => NSAccessibilityUnknownRole,
            Role::Terminal => NSAccessibilityTextAreaRole,
        }
    }
}

fn ns_sub_role(node: &Node) -> &'static NSAccessibilitySubrole {
    let role = node.role();

    unsafe {
        match role {
            Role::Alert => ns_string!("AXApplicationAlert"),
            Role::AlertDialog => NSAccessibilityDialogSubrole,
            Role::Article => ns_string!("AXDocumentArticle"),
            Role::Banner => ns_string!("AXLandmarkBanner"),
            Role::Button if node.toggled().is_some() => NSAccessibilityToggleSubrole,
            Role::Code => ns_string!("AXCodeStyleGroup"),
            Role::Complementary => ns_string!("AXLandmarkComplementary"),
            Role::ContentDeletion => ns_string!("AXDeleteStyleGroup"),
            Role::ContentInsertion => ns_string!("AXInsertStyleGroup"),
            Role::ContentInfo => ns_string!("AXLandmarkContentInfo"),
            Role::Definition => ns_string!("AXDefinition"),
            Role::Dialog => NSAccessibilityDialogSubrole,
            Role::Document => ns_string!("AXDocument"),
            Role::Emphasis => ns_string!("AXEmphasisStyleGroup"),
            Role::Feed => ns_string!("AXApplicationGroup"),
            Role::Footer => ns_string!("AXLandmarkContentInfo"),
            Role::Form => ns_string!("AXLandmarkForm"),
            Role::GraphicsDocument => ns_string!("AXDocument"),
            Role::Group => ns_string!("AXApplicationGroup"),
            Role::Header => ns_string!("AXLandmarkBanner"),
            Role::LayoutTableCell => NSAccessibilityGroupRole,
            Role::LayoutTableRow => NSAccessibilityTableRowSubrole,
            Role::Log => ns_string!("AXApplicationLog"),
            Role::Main => ns_string!("AXLandmarkMain"),
            Role::Marquee => ns_string!("AXApplicationMarquee"),
            Role::Math => ns_string!("AXDocumentMath"),
            Role::Meter => ns_string!("AXMeter"),
            Role::Navigation => ns_string!("AXLandmarkNavigation"),
            Role::Note => ns_string!("AXDocumentNote"),
            Role::PasswordInput => NSAccessibilitySecureTextFieldSubrole,
            Role::Region => ns_string!("AXLandmarkRegion"),
            Role::Search => ns_string!("AXLandmarkSearch"),
            Role::SearchInput => NSAccessibilitySearchFieldSubrole,
            Role::SectionFooter => ns_string!("AXSectionFooter"),
            Role::SectionHeader => ns_string!("AXSectionHeader"),
            Role::Status => ns_string!("AXApplicationStatus"),
            Role::Strong => ns_string!("AXStrongStyleGroup"),
            Role::Switch => NSAccessibilitySwitchSubrole,
            Role::Tab => NSAccessibilityTabButtonSubrole,
            Role::TabPanel => ns_string!("AXTabPanel"),
            Role::Term => ns_string!("AXTerm"),
            Role::Time => ns_string!("AXTimeGroup"),
            Role::Timer => ns_string!("AXApplicationTimer"),
            Role::TreeItem => NSAccessibilityOutlineRowSubrole,
            Role::Tooltip => ns_string!("AXUserInterfaceTooltip"),
            _ => NSAccessibilityUnknownSubrole,
        }
    }
}

pub(crate) fn can_be_focused(node: &Node) -> bool {
    filter(node) == FilterResult::Include && node.role() != Role::Window
}

#[derive(PartialEq)]
pub(crate) enum Value {
    Bool(bool),
    Number(f64),
    String(String),
}

pub(crate) struct NodeWrapper<'a>(pub(crate) &'a Node<'a>);

impl NodeWrapper<'_> {
    fn is_root(&self) -> bool {
        self.0.is_root()
    }

    pub(crate) fn title(&self) -> Option<String> {
        if self.is_root() && self.0.role() == Role::Window {
            // If the group element that we expose for the top-level window
            // includes a title, VoiceOver behavior is broken.
            return None;
        }
        self.0.label()
    }

    pub(crate) fn description(&self) -> Option<String> {
        self.0.description()
    }

    pub(crate) fn placeholder(&self) -> Option<&str> {
        self.0.placeholder()
    }

    pub(crate) fn value(&self) -> Option<Value> {
        if let Some(toggled) = self.0.toggled() {
            return Some(Value::Bool(toggled != Toggled::False));
        }
        if self.0.role() == Role::Tab {
            // On Mac, tabs are exposed as radio buttons, and are treated as checkable.
            // Also, `Node::is_selected` is mapped to checked via `accessibilityValue`.
            return Some(Value::Bool(self.0.is_selected().unwrap_or(false)));
        }
        if let Some(value) = self.0.value() {
            return Some(Value::String(value));
        }
        if let Some(value) = self.0.numeric_value() {
            return Some(Value::Number(value));
        }
        None
    }

    pub(crate) fn supports_text_ranges(&self) -> bool {
        self.0.supports_text_ranges()
    }

    pub(crate) fn raw_text_selection(&self) -> Option<&TextSelection> {
        self.0.raw_text_selection()
    }

    fn is_container_with_selectable_children(&self) -> bool {
        self.0.is_container_with_selectable_children() && self.0.role() != Role::TabList
    }

    pub(crate) fn is_item_like(&self) -> bool {
        self.0.is_item_like() && self.0.role() != Role::Tab
    }
}
