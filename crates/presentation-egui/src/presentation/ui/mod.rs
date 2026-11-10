//! Redesigned UI component library: design tokens, vector icons, widgets, the app shell
//! (sidebar + transport bar) and page views.
//!
//! Everything here is "data in, actions out": components take borrowed props plus an
//! optional mutable UI-state struct and return [`UiAction`]s. Nothing depends on `GuiApp`.

pub mod actions;
pub mod collate;
pub mod fonts;
pub mod format;
pub mod i18n;
pub mod icons;
pub mod shell;
pub mod shortcuts;
pub mod text;
pub mod theme;
pub mod views;
pub mod widgets;

pub use actions::{Page, RepeatMode, RightPanel, SortColumn, SortSpec, UiAction};
pub use i18n::{Lang, Tr};
pub use icons::Icon;
pub use theme::{Accent, Palette, ThemeMode};

#[cfg(test)]
mod gallery_tests;
