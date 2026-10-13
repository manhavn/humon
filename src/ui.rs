//! Shared measures and building blocks, so every bar, panel, row and button
//! is spaced and sized the same everywhere.
//!
//! Spacing comes in steps of 4: `SPACE` between controls, `INSET` from an
//! edge. Controls and list rows are `ROW` tall, bars `BAR`. Interface text is
//! `TEXT`, secondary text `SMALL`; data and code use the mono font at `SMALL`.

use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
use gpui_kit::component::{IconName, Selectable, Sizable};
use gpui_kit::*;

/// Between controls in a row, and between stacked blocks.
pub const SPACE: f32 = 8.;
/// From the edge of a bar, panel or pane to its content.
pub const INSET: f32 = 12.;
/// A list row or a control.
pub const ROW: f32 = 28.;
/// A bar holding a row of controls.
pub const BAR: f32 = 44.;
pub const TEXT: f32 = 13.;
pub const SMALL: f32 = 12.;

/// A horizontal bar of controls, divided from what follows.
pub fn bar(border: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .items_center()
        .gap(px(SPACE))
        .px(px(INSET))
        .min_h(px(BAR))
        .border_b_1()
        .border_color(border)
        .text_size(px(TEXT))
}

/// A panel that opens under a bar: export, code, indexes, explain.
pub fn panel(border: Hsla, background: Hsla) -> Div {
    div()
        .flex()
        .flex_none()
        .flex_col()
        .gap(px(SPACE))
        .px(px(INSET))
        .py(px(INSET))
        .border_b_1()
        .border_color(border)
        .bg(background)
        .text_size(px(TEXT))
}

/// The main action of a bar.
pub fn primary(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).small().primary().label(label)
}

/// A regular action.
pub fn action(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Button {
    Button::new(id).small().label(label)
}

/// A quiet action in a toolbar; `on` marks the panel it opens as open.
pub fn tool(id: impl Into<ElementId>, label: impl Into<SharedString>, on: bool) -> Button {
    Button::new(id).small().ghost().label(label).selected(on)
}

/// A quiet toolbar button showing only an icon, named in its tooltip.
pub fn icon_tool(id: impl Into<ElementId>, icon: IconName, tooltip: impl Into<SharedString>) -> Button {
    Button::new(id).small().ghost().icon(icon).tooltip(tooltip)
}

/// One-of-several choice, like a segmented control.
pub fn segmented(id: impl Into<ElementId>) -> ButtonGroup {
    ButtonGroup::new(id).small().outline()
}

/// A secondary line of text.
pub fn note(text: impl Into<SharedString>, muted: Hsla) -> Div {
    div().text_size(px(SMALL)).text_color(muted).child(text.into())
}
