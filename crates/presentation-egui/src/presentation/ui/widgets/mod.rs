//! Reusable widgets.

pub mod artwork;
pub mod card;
pub mod dialog;
pub mod empty_state;
pub mod search_box;
pub mod seek_bar;
pub mod slider;
pub mod toast;
pub mod toggle;
pub mod volume;

pub use artwork::{artwork, cover_uv, paint_artwork, paint_placeholder, Tex};
pub use card::{badge, button, button_sized, card, card_frame, primary_button, secondary_button, section_header, ButtonKind};
pub use dialog::{dialog, dialog_error, dialog_text, dialog_text_field, DialogButton, DialogResult, DialogSpec};
pub use empty_state::empty_state;
pub use search_box::{search_box, search_box_with_hint, SearchBoxOutput};
pub use seek_bar::{seek_bar, SeekBarOutput};
pub use slider::{slim_slider, SlimSliderOutput, SlimSliderStyle};
pub use toast::{Toast, ToastKind, Toasts};
pub use toggle::{segmented, toggle_switch};
pub use volume::{gain_to_ui, ui_to_gain, volume_control, VolumeOutput, VOLUME_STEP};
